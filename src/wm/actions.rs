// src/wm/actions.rs
use crate::wm::layout::Direction;
use crate::wm::AppState;

#[derive(Debug, Clone)]
pub enum Action {
    CloseFocused,
    Focus(Direction),
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
                // 从 args 数组的第一个元素读取方向
                let dir_str = args
                    .as_ref()
                    .and_then(|v| v.get(0))
                    .map(|s| s.as_str())
                    .unwrap_or("right"); // 默认向右

                let dir = match dir_str.to_lowercase().as_str() {
                    "left" => Direction::Left,
                    "right" => Direction::Right,
                    "up" => Direction::Up,
                    "down" => Direction::Down,
                    _ => Direction::Right,
                };
                Action::Focus(dir)
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
                if let Some(f_id) = &self.focused_window {
                    if let Some(new_focus) = self.find_neighbor(f_id, dir) {
                        self.focused_window = Some(new_focus);
                        // 注意：这里只是改变了变量，真正的指令在 ManageStart 里发送给 River
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

    // 一个简单的几何邻居查找算法
    fn find_neighbor(
        &self,
        current_id: &wayland_backend::client::ObjectId,
        dir: Direction,
    ) -> Option<wayland_backend::client::ObjectId> {
        let cur_geo = self.last_geometry.get(current_id)?;
        let cur_center_x = cur_geo.x + cur_geo.w / 2;
        let cur_center_y = cur_geo.y + cur_geo.h / 2;

        self.windows
            .iter()
            .filter(|w| &w.id != current_id) // 排除自己
            .filter_map(|w| {
                let g = self.last_geometry.get(&w.id)?;
                let center_x = g.x + g.w / 2;
                let center_y = g.y + g.h / 2;

                // 根据方向过滤
                let is_in_dir = match dir {
                    Direction::Left => center_x < cur_center_x,
                    Direction::Right => center_x > cur_center_x,
                    Direction::Up => center_y < cur_center_y,
                    Direction::Down => center_y > cur_center_y,
                };

                if is_in_dir {
                    // 计算距离（勾股定理）
                    let dist = (((center_x - cur_center_x).pow(2)
                        + (center_y - cur_center_y).pow(2)) as f32)
                        .sqrt();
                    Some((w.id.clone(), dist))
                } else {
                    None
                }
            })
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(id, _)| id)
    }
}
