pub mod config;
pub mod protocol;
pub mod wm;
use crate::config::Config;
use crate::wm::AppState;
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::os::unix::net::{UnixListener, UnixStream}; 
use wayland_client::Connection;

fn main() {
    // 获取命令行参数
    let args: Vec<String> = std::env::args().collect();
    // 判定是否进入“客户端状态模式”
    if args.len() > 1 && args[1] == "--status" {
        run_status_client();
        return; // 运行完客户端就退出，不启动 WM
    }

    let config = Config::load();
    let conn = Connection::connect_to_env().expect("请在 River 环境下运行");
    let display = conn.display();
    let mut event_queue = conn.new_event_queue();
    let qh = event_queue.handle();

    // 我们用 WAYLAND_DISPLAY 来区分不同的会话，防止多个 river 实例冲突
    let display_name = std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-0".to_string());
    let socket_path = format!("/tmp/rrwm-{}.sock", display_name);
    // 如果之前程序崩溃留下了旧文件，先删掉它，否则会绑定失败
    let _ = fs::remove_file(&socket_path);
    let listener = UnixListener::bind(&socket_path).expect("无法创建 IPC Socket");
    // 设置为非阻塞模式，这样我们检查新连接时才不会卡住整个 WM
    listener
        .set_nonblocking(true)
        .expect("无法设置 Socket 非阻塞");

    println!("-> IPC 电台已在 {:?} 启动", socket_path);

    let mut state = AppState {
        config: config,
        needs_reload: false,
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
        current_keymap: None,
        layer_shell_manager: None,
        device_names: HashMap::new(), // 初始化哈希表
        ipc_listener: Some(listener),
        ipc_clients: Vec::new(),
        output_manager: None,
        heads: Vec::new(),
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

/// 客户端模式：连接 Socket 并把收到的东西直接打印出来
fn run_status_client() {
    let display_name = std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-0".to_string());
    let socket_path = format!("/tmp/rrwm-{}.sock", display_name);

    if let Ok(stream) = UnixStream::connect(&socket_path) {
        let mut reader = BufReader::new(stream);
        let mut line = String::new();

        // 持续读取 Socket 里的每一行并打印到 stdout
        // Waybar 的 custom/script 模块会自动捕获这个 stdout
        while reader.read_line(&mut line).unwrap_or(0) > 0 {
            print!("{}", line);
            line.clear();
        }
    } else {
        eprintln!("错误：无法连接到 rrwm Socket，请确保 rrwm 正在运行。");
        std::process::exit(1);
    }
}
