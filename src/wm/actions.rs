// src/wm/actions.rs
use crate::wm::AppState;
use crate::wm::layout::Direction;

#[derive(Clone)]
pub enum Action {
    CloseFocused,
    Focus(Direction),
    Spawn(String),
}

impl AppState {
    pub fn perform_action(&mut self, action: Action) {
        match action {
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
            Action::Spawn(cmd) => {
                std::process::Command::new("sh").arg("-c").arg(cmd).spawn().ok();
            }
        }
    }

    // 一个简单的几何邻居查找算法
    fn find_neighbor(&self, current_id: &wayland_backend::client::ObjectId, dir: Direction) -> Option<wayland_backend::client::ObjectId> {
        let cur_geo = self.last_geometry.get(current_id)?;
        let cur_center_x = cur_geo.x + cur_geo.w / 2;
        let cur_center_y = cur_geo.y + cur_geo.h / 2;

        self.windows.iter()
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
                    let dist = ((center_x - cur_center_x).pow(2) + (center_y - cur_center_y).pow(2)) as f32;
                    Some((w.id.clone(), dist))
                } else {
                    None
                }
            })
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap()) // 找最近的
            .map(|(id, _)| id)
    }
}
