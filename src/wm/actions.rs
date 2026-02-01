// src/wm/actions.rs
use crate::wm::layout::{Direction, LayoutNode, SplitType}; // 修复点：引入布局相关的类型
use crate::wm::AppState;
use serde::Serialize;
use std::io::Write;
use wayland_backend::client::ObjectId; // 修复点：引入 ObjectId 类型

// 定义发送给 Bar 的状态数据包
#[derive(Serialize, Clone)]
pub struct RrwmStatus {
    pub focused_tags: u32,     // 当前正在看哪个标签 (掩码)
    pub occupied_tags: u32,    // 哪些标签里有窗口 (掩码)
    pub active_window: String, // 当前聚焦的窗口标题 (比如 "Kitty")
}

#[derive(Serialize, Clone)]
pub struct WaybarResponse {
    pub text: String,
    pub tooltip: String,
    pub class: String,
}

#[derive(PartialEq)]
enum MoveHint {
    Leftmost,  // 强制出现在最左边
    Rightmost, // 强制出现在最右边
}

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

            // --- 编号移动 (Super+Shift+数字) ---
            Action::MoveToTag(target_mask) => {
                if let Some(f_id) = self.focused_window.clone() {
                    // 固定出现在左边
                    self.move_window_to_tag(&f_id, target_mask, true, MoveHint::Leftmost);
                }
            }
            // --- 方向性移动 (Super+Shift+n/i/u/e) ---
            Action::Move(dir) => {
                if let Some(f_id) = self.focused_window.clone() {
                    self.move_window_locally(&f_id, dir);
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

    /// 核心：处理 IPC 连接（迎接新听众）
    pub fn handle_ipc_connections(&mut self) {
        if let Some(ref listener) = self.ipc_listener {
            // 尝试接受新连接（因为是非阻塞的，所以没连接时会立刻报错并跳过）
            while let Ok((mut stream, _)) = listener.accept() {
                println!("-> IPC: 发现新听众 (Bar/Script)");
                // 刚连进来时，先给它发一个当前状态，别让人家等着
                let _ = self.send_status_to_stream(&mut stream);
                self.ipc_clients.push(stream);
            }
        }
    }

    /// 核心：向所有听众广播状态
    pub fn broadcast_status(&mut self) {
        if self.ipc_clients.is_empty() {
            return;
        }

        let occupied = self.get_occupied_tags();
        let user_icons = self
            .config
            .appearance
            .as_ref()
            .and_then(|a| a.tag_icons.as_ref());
        let mut tag_strings = Vec::new();

        // --- 核心算法：计算动态显示的截止点 ---
        // 1. 找到最大的“被占用”标签索引 (0-31)
        // 32 - leading_zeros 得到的是位数，减 1 才是索引
        let max_occupied_idx = if occupied == 0 {
            0
        } else {
            32 - occupied.leading_zeros() - 1
        };

        // 2. 找到当前“被聚焦”标签索引
        let focused_idx = if self.focused_tags == 0 {
            0
        } else {
            32 - self.focused_tags.leading_zeros() - 1
        };

        // 3. 决定显示范围：取两者的最大值，再 +1 (保留一个空位作为缓冲)
        let visual_bound = (max_occupied_idx.max(focused_idx) + 1).min(8);

        // --- 循环生成 ---
        for i in 0..=visual_bound {
            let mask = 1 << i;
            let icon = user_icons
                .and_then(|icons| icons.get(i as usize))
                .cloned()
                .unwrap_or_else(|| (i + 1).to_string());

            let styled_icon = if (self.focused_tags & mask) != 0 {
                format!("<span color='#bd93f9' underline='single'>{}</span>", icon)
            } else if (occupied & mask) != 0 {
                format!("<span color='#ffffff'>{}</span>", icon)
            } else {
                format!("<span color='#666666'>{}</span>", icon)
            };
            tag_strings.push(styled_icon);
        }

        let response = WaybarResponse {
            text: tag_strings.join("  "),
            tooltip: format!("Focus: {}", self.get_active_window_title()),
            class: "rrwm-status".to_string(),
        };

        if let Ok(mut json) = serde_json::to_string(&response) {
            json.push('\n');
            self.ipc_clients
                .retain_mut(|client| std::io::Write::write_all(client, json.as_bytes()).is_ok());
        }
    }

    // 内部辅助：向单个流发送状态
    fn send_status_to_stream(
        &self,
        stream: &mut std::os::unix::net::UnixStream,
    ) -> std::io::Result<()> {
        let status = RrwmStatus {
            focused_tags: self.focused_tags,
            occupied_tags: self.get_occupied_tags(),
            active_window: self.get_active_window_title(),
        };
        let mut json = serde_json::to_string(&status).unwrap();
        json.push('\n');
        stream.write_all(json.as_bytes())
    }

    /// 计算哪些标签有窗口
    pub fn get_occupied_tags(&self) -> u32 {
        let mut mask = 0u32;
        for w in &self.windows {
            if w.app_id.is_some() {
                mask |= w.tags;
            }
        }
        mask
    }

    /// 获取焦点窗口标题
    pub fn get_active_window_title(&self) -> String {
        if let Some(f_id) = &self.focused_window {
            if let Some(w) = self.windows.iter().find(|w| &w.id == f_id) {
                return w.app_id.clone().unwrap_or_else(|| "Unknown".to_string());
            }
        }
        "".to_string()
    }

    /// 搬迁窗口至新 Tag
    fn move_window_to_tag(
        &mut self,
        win_id: &ObjectId,
        target_mask: u32,
        follow: bool,
        hint: MoveHint,
    ) {
        let old_tag = match self.windows.iter().find(|w| &w.id == win_id) {
            Some(w) => w.tags,
            None => return,
        };
        if old_tag == target_mask {
            return;
        }

        // 1. 接班人逻辑：如果焦点走了，给旧标签指派新领袖
        if self.tag_focus_history.get(&old_tag) == Some(win_id) {
            let replacement = self
                .windows
                .iter()
                .find(|w| &w.id != win_id && (w.tags & old_tag) != 0)
                .map(|w| w.id.clone());
            if let Some(rid) = replacement {
                self.tag_focus_history.insert(old_tag, rid);
            } else {
                self.tag_focus_history.remove(&old_tag);
            }
        }

        // 2. 从旧树中移除
        if let Some(root) = self.layout_roots.remove(&old_tag) {
            if let Some(new_root) = LayoutNode::remove_at(root, win_id) {
                self.layout_roots.insert(old_tag, new_root);
            }
        }

        // 3. 更新窗口数据副本
        let mut win_data_opt = None;
        if let Some(w_info) = self.windows.iter_mut().find(|w| &w.id == win_id) {
            w_info.tags = target_mask;
            win_data_opt = Some(w_info.clone());
        }

        // 4. 插入新树：这种插入方式非常简单高效，直接包裹旧树
        if let Some(w_data) = win_data_opt {
            if let Some(old_root) = self.layout_roots.remove(&target_mask) {
                let new_root = match hint {
                    MoveHint::Leftmost => LayoutNode::Container {
                        split_type: SplitType::Vertical,
                        ratio: 0.5,
                        left_child: Box::new(LayoutNode::Window(w_data)),
                        right_child: Box::new(old_root),
                    },
                    MoveHint::Rightmost => LayoutNode::Container {
                        split_type: SplitType::Vertical,
                        ratio: 0.5,
                        left_child: Box::new(old_root),
                        right_child: Box::new(LayoutNode::Window(w_data)),
                    },
                };
                self.layout_roots.insert(target_mask, new_root);
            } else {
                // 目标 Tag 是空的，直接做根节点
                self.layout_roots
                    .insert(target_mask, LayoutNode::Window(w_data));
            }
        }

        // 5. 状态同步
        self.tag_focus_history.insert(target_mask, win_id.clone());
        if follow {
            self.focused_tags = target_mask;
            self.focused_window = Some(win_id.clone());
        }
        if let Some(wm) = &self.river_wm {
            wm.manage_dirty();
        }
    }

    /// 相对标签移动（向左/向右一个 Tag）
    fn move_window_relative(&mut self, win_id: &ObjectId, delta: i32, hint: MoveHint) {
        let mut current_idx = 0;
        for i in 0..9 {
            if (self.focused_tags & (1 << i)) != 0 {
                current_idx = i as i32;
                break;
            }
        }
        let next_idx = (current_idx + delta).rem_euclid(9) as u32;
        let next_mask = 1 << next_idx;
        self.move_window_to_tag(win_id, next_mask, true, hint);
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
                    self.move_window_relative(win_id, -1, MoveHint::Rightmost);
                }
                Direction::Right => {
                    println!("-> 右边界已达，跨标签移动至下一个 Tag");
                    self.move_window_relative(win_id, 1, MoveHint::Leftmost);
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
    /// 智能动态流转逻辑
    fn cycle_tag(&mut self, delta: i32) {
        // 1. 获取当前 Tag 的索引 (0-31)
        // trailing_zeros() 对于 0001 返回 0，对于 0010 返回 1，比循环更高效
        let current_idx = self.focused_tags.trailing_zeros();

        // 2. 计算动态边界 (Dynamic Boundary)
        let occupied = self.get_occupied_tags();
        
        // 找到最高位的 1 在哪里。如果没有窗口，max_occupied_idx 设为 0 (Tag 1)
        // 32 - leading_zeros - 1 得到最高位的索引
        let max_occupied_idx = if occupied == 0 {
            0
        } else {
            32 - occupied.leading_zeros() - 1
        };

        // 边界 = 最大占用索引 + 1 (保留一个备用空位)
        // 限制最大为 8 (即 Tag 9)，防止跑到 Tag 10+ 去
        let bound_idx = (max_occupied_idx + 1).min(8);

        // 3. 计算目标索引
        let next_idx = if delta > 0 {
            // --- 向右移动 ---
            if current_idx >= bound_idx {
                // 如果当前已经在边界（或者意外超出了边界），回到 Tag 1
                0 
            } else {
                // 否则正常向右
                current_idx + 1
            }
        } else {
            // --- 向左移动 ---
            if current_idx == 0 {
                // 如果在 Tag 1，跳到边界 (最右有窗口Tag + 1)
                bound_idx
            } else {
                // 否则正常向左
                current_idx - 1
            }
        };

        let next_mask = 1 << next_idx;

        // 仅当 Tag 真正改变时才执行操作，避免日志刷屏
        if next_mask != self.focused_tags {
            println!("-> 动态流转: Tag {} -> Tag {}", current_idx + 1, next_idx + 1);
            self.focused_tags = next_mask;
            // River 会在下一帧自动感知 focused_tags 变化并发起 ManageStart
        }
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
