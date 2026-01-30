pub mod protocol;
pub mod wm;

use std::collections::HashMap;
use wayland_client::{Connection, QueueHandle};
use crate::wm::AppState;

fn main() {
    let conn = Connection::connect_to_env().expect("请在 River 环境下运行");
    let display = conn.display();
    let mut event_queue = conn.new_event_queue();
    let qh = event_queue.handle();

    let mut state = AppState {
        river_wm: None,
        windows: Vec::new(),
        outputs: HashMap::new(),
        main_seat: None,
        current_width: 0,
        current_height: 0,
        layout_root: None,
        last_geometry: HashMap::new(),
        focused_window: None,
    };

    let _registry = display.get_registry(&qh, ());
    println!("rrwm 已启动，正在监听事件...");

    loop {
        event_queue.blocking_dispatch(&mut state).unwrap();
    }
}
