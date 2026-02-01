pub mod config;
pub mod protocol;
pub mod wm;

use crate::config::Config;
use crate::wm::AppState;
use std::collections::HashMap;
use wayland_client::Connection;

fn main() {
    let config = Config::load();
    let conn = Connection::connect_to_env().expect("请在 River 环境下运行");
    let display = conn.display();
    let mut event_queue = conn.new_event_queue();
    let qh = event_queue.handle();

    let mut state = AppState {
        config: config,
        river_wm: None,
        windows: Vec::new(),
        outputs: HashMap::new(),
        main_seat: None,
        current_width: 0,
        current_height: 0,
        layout_roots: HashMap::new(),
        tag_focus_history: HashMap::new(),
        last_geometry: HashMap::new(),
        focused_window: None,
        focused_tags: 1, // 默认查看第 1 个标签 (二进制 0001)
        xkb_manager: None,
        key_bindings: Vec::new(),
        input_manager: None,
        xkb_config: None,
        keyboards: Vec::new(),
        layer_shell_manager: None,
    };

    let _registry = display.get_registry(&qh, ());
    println!("rrwm 已启动，正在监听事件...");

    loop {
        if let Err(e) = event_queue.blocking_dispatch(&mut state) {
            eprintln!("Wayland 连接发生致命错误: {:?}", e);
            // 可以在这里打印更详细的 state 信息
            break;
        }
    }
}
