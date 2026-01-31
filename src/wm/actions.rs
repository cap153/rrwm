// src/wm/actions.rs
use crate::wm::layout::Direction;
use crate::wm::AppState;

#[derive(Debug, Clone)]
pub enum Action {
    CloseFocused,
    Focus(Direction),
    FocusTag(u32),      // 切换到某个标签掩码
    MoveToTag(u32),     // 将窗口移动到某个标签掩码
    Spawn(Vec<String>), // 纯净启动：[程序名, 参数1, 参数2]
    Shell(String),      // Shell 启动：一整串命令字符串
}

impl Action {
    /// 核心逻辑：把 TOML 里的字符串配置变成代码里的枚举
    pub fn from_config(name: &str, args: &Option<Vec<String>>, cmd: &Option<String>) -> Self {
        match name.to_lowercase().as_str() {
            // --- 内部指令：关闭窗口 ---
            "close_window" | "close_focused" => Action::CloseFocused,

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
                match arg.parse::<u32>() {
                    Ok(tag_idx) => Action::MoveToTag(1 << (tag_idx.saturating_sub(1))),
                    Err(_) => {
                        // 这里可以以后扩展 move left/right 到跨标签
                        Action::Shell("true".to_string())
                    }
                }
            }

            // "spawn" 模式：直接启动，不经过 sh
            "spawn" => {
                let cmd_list = args.clone().unwrap_or_default();
                Action::Spawn(cmd_list)
            }

            // "shell" 模式：交给 sh -c 处理复杂逻辑
            "shell" => {
                let cmd_str = cmd.clone().unwrap_or_default();
                Action::Shell(cmd_str)
            }

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
            // --- 标签切换逻辑 ---
            Action::FocusTag(mask) => {
                println!("-> 切换标签掩码为: {:b}", mask);
                self.focused_tags = mask;
                // 注意：修改 state 后，River 随后会自动触发 ManageStart 重新渲染
            }

            // --- 移动窗口到标签逻辑 ---
            Action::MoveToTag(mask) => {
                if let Some(f_id) = &self.focused_window {
                    if let Some(w_data) = self.windows.iter_mut().find(|w| &w.id == f_id) {
                        println!("-> 将窗口 {:?} 移动到标签掩码: {:b}", f_id, mask);
                        w_data.tags = mask;
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
                        Direction::Right => self.cycle_tag(1),  // 向右切到下一个标签
                        Direction::Left  => self.cycle_tag(-1), // 向左切到上一个标签
                        _ => {} // 上下方向通常不跨标签
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
    fn find_neighbor(
        &self,
        current_id: &wayland_backend::client::ObjectId,
        dir: Direction,
    ) -> Option<wayland_backend::client::ObjectId> {
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
