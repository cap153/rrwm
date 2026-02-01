// src/wm/actions.rs
use crate::wm::layout::{Direction, LayoutNode, SplitType}; // 修复点：引入布局相关的类型
use crate::wm::AppState;
use wayland_backend::client::ObjectId; // 修复点：引入 ObjectId 类型

#[derive(Debug, Clone)]
pub enum Action {
    CloseFocused,
    Focus(Direction),
    FocusTag(u32),       // 切换到某个标签掩码
    MoveToTag(u32),      // 将窗口移动到某个标签掩码
    Move(Direction),     // 统一处理方向性移动
    Spawn(Vec<String>),  // 纯净启动：[程序名, 参数1, 参数2]
    Shell(String),       // Shell 启动：一整串命令字符串
    ReloadConfiguration, // 重载配置
}

impl Action {
    /// 核心逻辑：把 TOML 里的字符串配置变成代码里的枚举
    pub fn from_config(name: &str, args: &Option<Vec<String>>, cmd: &Option<String>) -> Self {
        match name.to_lowercase().as_str() {
            // --- 内部指令：关闭窗口 ---
            "close_window" | "close_focused" => Action::CloseFocused,
            // --- 内部指令：重载配置 ---
            "reload_configuration" => Action::ReloadConfiguration,
            // --- 内部指令：焦点切换 ---
            "focus" => {
                let arg = args
                    .as_ref()
                    .and_then(|v| v.get(0))
                    .map(|s| s.as_str())
                    .unwrap_or("right");
                // 判断参数是“方向字符串”还是“数字字符串”
                match arg.parse::<u32>() {
                    Ok(tag_idx) => Action::FocusTag(1 << (tag_idx.saturating_sub(1))),
                    Err(_) => {
                        // 如果是方向
                        let dir = match arg.to_lowercase().as_str() {
                            "left" => Direction::Left,
                            "up" => Direction::Up,
                            "down" => Direction::Down,
                            _ => Direction::Right,
                        };
                        Action::Focus(dir)
                    }
                }
            }
            // 对应 Action::MoveToTag (Super+Shift+数字)
            "move" => {
                let arg = args
                    .as_ref()
                    .and_then(|v| v.get(0))
                    .map(|s| s.as_str())
                    .unwrap_or("1");
                if let Ok(tag_idx) = arg.parse::<u32>() {
                    Action::MoveToTag(1 << (tag_idx.saturating_sub(1)))
                } else {
                    let dir = match arg.to_lowercase().as_str() {
                        "left" => Direction::Left,
                        "up" => Direction::Up,
                        "down" => Direction::Down,
                        _ => Direction::Right,
                    };
                    Action::Move(dir)
                }
            }

            // "spawn" 模式：直接启动，不经过 sh
            "spawn" => Action::Spawn(args.clone().unwrap_or_default()),

            // "shell" 模式：交给 sh -c 处理复杂逻辑
            "shell" => Action::Shell(cmd.clone().unwrap_or_default()),

            _ => {
                println!("警告：未知的动作名称 {}", name);
                Action::Shell("true".to_string())
            }
        }
    }
}
impl AppState {
    pub fn perform_action(&mut self, action: Action) {
        match action {
            Action::ReloadConfiguration => {
                println!("-> 正在手动重载配置...");
                self.config = crate::config::Config::load();
                self.needs_reload = true;
                self.current_keymap = None;
                println!("-> 配置已重载，新的布局将在下次键盘接入或手动触发时生效");
            }
            // --- 标签切换逻辑 ---
            Action::FocusTag(mask) => {
                println!("-> 切换标签掩码为: {:b}", mask);
                self.focused_tags = mask;
                // 注意：修改 state 后，River 随后会自动触发 ManageStart 重新渲染
            }

            // --- 移动窗口到标签逻辑 ---
            Action::MoveToTag(target_mask) => {
                if let Some(f_id) = self.focused_window.clone() {
                    self.move_window_to_tag(&f_id, target_mask, true); // true 表示跟随
                }
            }

            // --- 方向性移动 (Super+Shift+n/i/u/e) ---
            Action::Move(dir) => {
                if let Some(f_id) = self.focused_window.clone() {
                    // 尝试在本地寻找邻居进行“换位”或“搬迁”
                    if let Some(_neighbor_id) = self.find_neighbor(&f_id, dir) {
                        // 本地移动逻辑：在 BSP 树中，这通常意味着移除并重新插入到邻居位置
                        // 为了保持 Cosmic 风格简洁，我们暂时采用：移除 -> 在邻居方向重新插入
                        self.move_window_locally(&f_id, dir);
                    } else {
                        // 边界情况：跨标签流转
                        match dir {
                            Direction::Right => self.move_window_relative(&f_id, 1),
                            Direction::Left => self.move_window_relative(&f_id, -1),
                            _ => {}
                        }
                    }
                }
            }
            // 直接启动逻辑：更轻量，无 Shell 开销
            Action::Spawn(cmd_list) => {
                if cmd_list.is_empty() {
                    return;
                }
                println!("-> [Spawn] 启动进程: {:?}", cmd_list);

                std::process::Command::new(&cmd_list[0])
                    .args(&cmd_list[1..])
                    .spawn()
                    .map_err(|e| eprintln!("-> Spawn 失败: {}", e))
                    .ok();
            }

            // Shell 启动逻辑：支持环境变量和管道
            Action::Shell(cmd_str) => {
                if cmd_str.is_empty() {
                    return;
                }
                println!("-> [Shell] 执行命令: {}", cmd_str);

                std::process::Command::new("sh")
                    .arg("-c")
                    .arg(cmd_str)
                    .spawn()
                    .map_err(|e| eprintln!("-> Shell 执行失败: {}", e))
                    .ok();
            }
            Action::Focus(dir) => {
                // 1. 定义一个变量记录是否在本页成功跳转了焦点
                let mut moved_locally = false;
                // 2. 只有在有焦点窗口时才尝试找邻居
                if let Some(f_id) = &self.focused_window {
                    if let Some(new_focus) = self.find_neighbor(f_id, dir) {
                        self.focused_window = Some(new_focus.clone());
                        self.tag_focus_history.insert(self.focused_tags, new_focus);
                        moved_locally = true;
                    }
                }
                // 3. 如果没有在本页跳转成功（可能是没邻居，也可能是根本没窗口）
                if !moved_locally {
                    match dir {
                        Direction::Right => self.cycle_tag(1), // 向右切到下一个标签
                        Direction::Left => self.cycle_tag(-1), // 向左切到上一个标签
                        _ => {}                                // 上下方向通常不跨标签
                    }
                }
            }
            Action::CloseFocused => {
                if let Some(f_id) = &self.focused_window {
                    if let Some(w_data) = self.windows.iter().find(|w| &w.id == f_id) {
                        w_data.window.close();
                    }
                }
            }
        }
    }

    fn move_window_to_tag(&mut self, win_id: &ObjectId, target_mask: u32, follow: bool) {
        let old_tag = match self.windows.iter().find(|w| &w.id == win_id) {
            Some(w) => w.tags,
            None => return,
        };

        if old_tag == target_mask {
            return;
        }

        // --- 在迁出前，更新旧标签的焦点记忆 ---
        if self.tag_focus_history.get(&old_tag) == Some(win_id) {
            // 在旧标签里找一个“留守”的窗口作为新领袖
            let replacement = self
                .windows
                .iter()
                .find(|w| &w.id != win_id && (w.tags & old_tag) != 0)
                .map(|w| w.id.clone());

            if let Some(new_id) = replacement {
                self.tag_focus_history.insert(old_tag, new_id);
            } else {
                // 如果旧标签搬空了，就删掉记录
                self.tag_focus_history.remove(&old_tag);
            }
        }

        // 1. 从旧树中彻底移除
        if let Some(root) = self.layout_roots.remove(&old_tag) {
            if let Some(new_root) = LayoutNode::remove_at(root, win_id) {
                self.layout_roots.insert(old_tag, new_root);
            }
        }
        // 2. 更新窗口数据的标签
        let mut win_data_opt = None;
        if let Some(w_info) = self.windows.iter_mut().find(|w| &w.id == win_id) {
            w_info.tags = target_mask;
            win_data_opt = Some(w_info.clone());
        }

        // 3. 插入到新树中
        if let Some(w_data) = win_data_opt {
            if let Some(mut root) = self.layout_roots.remove(&target_mask) {
                // 找到新标签页的焦点窗口进行切割插入
                let target_in_new = self
                    .tag_focus_history
                    .get(&target_mask)
                    .cloned()
                    .unwrap_or_else(|| w_data.id.clone());

                // 默认垂直切分（左右）
                root.insert_at(&target_in_new, w_data, SplitType::Vertical);
                self.layout_roots.insert(target_mask, root);
            } else {
                // 如果目标标签是空的，直接设为根
                self.layout_roots
                    .insert(target_mask, LayoutNode::Window(w_data));
            }
        }

        // 4. 更新焦点记忆
        self.tag_focus_history.insert(target_mask, win_id.clone());

        // 5. 如果需要跟随（bspwm 风格）
        if follow {
            self.focused_tags = target_mask;
            self.focused_window = Some(win_id.clone());
        }
    }

    /// 相对标签移动（向左/向右一个 Tag）
    fn move_window_relative(&mut self, win_id: &ObjectId, delta: i32) {
        let mut current_idx = 0;
        for i in 0..9 {
            if (self.focused_tags & (1 << i)) != 0 {
                current_idx = i as i32;
                break;
            }
        }
        let next_idx = (current_idx + delta).rem_euclid(9) as u32;
        let next_mask = 1 << next_idx;
        self.move_window_to_tag(win_id, next_mask, true);
    }

    /// 本地移动：在同一 Tag 内重新排列
    fn move_window_locally(&mut self, win_id: &ObjectId, dir: Direction) {
        // 1. 尝试在当前方向寻找邻居
        if let Some(neighbor_id) = self.find_neighbor(win_id, dir) {
            println!("-> 发现邻居 {:?}，执行位置交换", neighbor_id);

            // 执行树内交换
            if let Some(root) = self.layout_roots.get_mut(&self.focused_tags) {
                LayoutNode::swap_windows(root, win_id, &neighbor_id);
            }
            // 交换后，焦点依然跟着原来的窗口
            self.focused_window = Some(win_id.clone());
            self.tag_focus_history
                .insert(self.focused_tags, win_id.clone());
        } else {
            // 2. 边界判定：如果水平方向没邻居了，执行跨标签流转（bspwm 风格）
            match dir {
                Direction::Left => {
                    println!("-> 左边界已达，跨标签移动至上一个 Tag");
                    self.move_window_relative(win_id, -1);
                }
                Direction::Right => {
                    println!("-> 右边界已达，跨标签移动至下一个 Tag");
                    self.move_window_relative(win_id, 1);
                }
                _ => {
                    println!("-> 上下边界已达，暂不处理跨标签");
                }
            }
        }
        // --- 手动触发重新排版 ---
        if let Some(wm) = &self.river_wm {
            wm.manage_dirty();
        }
    }
    /// 标签页流转逻辑：实现类似 bspwm 的 desktop -f next/prev
    fn cycle_tag(&mut self, delta: i32) {
        // 假设我们只用 1-9 号标签
        let mut current_idx = 0;
        for i in 0..9 {
            if (self.focused_tags & (1 << i)) != 0 {
                current_idx = i as i32;
                break;
            }
        }

        let next_idx = (current_idx + delta).rem_euclid(9) as u32;
        let next_mask = 1 << next_idx;

        println!("-> 触碰边界，流转至 Tag {}", next_idx + 1);
        self.focused_tags = next_mask;
    }

    /// 邻居查找，严格方向判定
    fn find_neighbor(&self, current_id: &ObjectId, dir: Direction) -> Option<ObjectId> {
        let cur_geo = self.last_geometry.get(current_id)?;

        self.windows
            .iter()
            .filter(|w| &w.id != current_id && (w.tags & self.focused_tags) != 0)
            .filter_map(|w| {
                let g = self.last_geometry.get(&w.id)?;

                // 判定是否在方向上
                let is_in_direction = match dir {
                    Direction::Left => g.x + g.w <= cur_geo.x,
                    Direction::Right => g.x >= cur_geo.x + cur_geo.w,
                    Direction::Up => g.y + g.h <= cur_geo.y,
                    Direction::Down => g.y >= cur_geo.y + cur_geo.h,
                };

                if !is_in_direction {
                    return None;
                }

                // 计算投影重叠度（判定它们是否“对得齐”）
                let overlap = match dir {
                    Direction::Left | Direction::Right => {
                        // 检查垂直方向是否有交叉
                        let over = cur_geo.y.max(g.y) < (cur_geo.y + cur_geo.h).min(g.y + g.h);
                        if over {
                            1000
                        } else {
                            0
                        } // 有重叠给予极高权重
                    }
                    Direction::Up | Direction::Down => {
                        // 检查水平方向是否有交叉
                        let over = cur_geo.x.max(g.x) < (cur_geo.x + cur_geo.w).min(g.x + g.w);
                        if over {
                            1000
                        } else {
                            0
                        }
                    }
                };

                // 计算边缘距离
                let dist = match dir {
                    Direction::Left => cur_geo.x - (g.x + g.w),
                    Direction::Right => g.x - (cur_geo.x + cur_geo.w),
                    Direction::Up => cur_geo.y - (g.y + g.h),
                    Direction::Down => g.y - (cur_geo.y + cur_geo.h),
                };

                // 分数：重叠度越高、距离越近，分数越低（越优）
                let score = dist - overlap;
                Some((w.id.clone(), score))
            })
            .min_by_key(|&(_, score)| score)
            .map(|(id, _)| id)
    }
}
