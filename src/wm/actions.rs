use crate::wm::AppState;

pub enum Action {
    Spawn(String),
    CloseFocused,
    FocusNext,
    FocusPrev,
    // 未来可以扩展：ToggleTag(u32), SetTag(u32)
}

impl AppState {
    pub fn perform_action(&mut self, action: Action) {
        match action {
            Action::Spawn(cmd) => {
                std::process::Command::new("sh").arg("-c").arg(cmd).spawn().ok();
            }
            Action::CloseFocused => {
                if let Some(id) = &self.focused_window {
                    if let Some(w_data) = self.windows.iter().find(|w| &w.id == id) {
                        w_data.window.close();
                    }
                }
            }
            Action::FocusNext => {
                if let Some(current) = &self.focused_window {
                    if let Some(pos) = self.windows.iter().position(|w| &w.id == current) {
                        let next_pos = (pos + 1) % self.windows.len();
                        let next_id = self.windows[next_pos].id.clone();
                        self.focused_window = Some(next_id);
                    }
                }
            }
            _ => {}
        }
    }
}
