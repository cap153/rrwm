use std::io::{Read, Write};
use tracing::{error, info};
pub mod config;
pub mod protocol;
pub mod wm;
use crate::config::Config;
use crate::wm::AppState;
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::os::unix::io::{AsFd, AsRawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use wayland_client::Connection;

fn main() {
    tracing_subscriber::fmt::init();
    let args: Vec<String> = std::env::args().collect();

    // --- 使用 match 处理参数 ---
    if args.len() > 1 {
        match args[1].as_str() {
            "--waybar" => {
                run_waybar_client();
                return;
            }
            "--appid" => {
                run_appid_client();
                return;
            }
            "--help" | "-h" => {
                print_help();
                return;
            }
            _ => {
                eprintln!("Error: Unknown argument '{}'", args[1]);
                eprintln!("Try 'rrwm --help' for usage information.");
                print_help();
                std::process::exit(1);
            }
        }
    }

    let config = Config::load();
    let conn = Connection::connect_to_env().expect("Please run in River environment");
    let display = conn.display();
    let mut event_queue = conn.new_event_queue();
    let qh = event_queue.handle();

    let display_name = std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-0".to_string());

    // Socket 1: Waybar 广播
    let socket_path = format!("/tmp/rrwm-{}.sock", display_name);
    let _ = fs::remove_file(&socket_path);
    let listener = UnixListener::bind(&socket_path).expect("Unable to create IPC Socket");
    listener
        .set_nonblocking(true)
        .expect("Unable to set Socket non-blocking");

    // Socket 2: 指令查询
    let cmd_socket_path = format!("/tmp/rrwm-{}-cmd.sock", display_name);
    let _ = fs::remove_file(&cmd_socket_path);
    let cmd_listener =
        UnixListener::bind(&cmd_socket_path).expect("Unable to create Command Socket");
    cmd_listener
        .set_nonblocking(true)
        .expect("Unable to set Command Socket non-blocking");

    info!("-> IPC radio started at {:?}", socket_path);
    info!("-> Command listener started at {:?}", cmd_socket_path);

    // 必须在进入 loop 前提取，因为 loop 里 state 会被借用
    let wayland_fd = conn.as_fd().as_raw_fd();
    let ipc_fd = listener.as_raw_fd();
    let cmd_fd = cmd_listener.as_raw_fd();

    let mut state = AppState {
        config: config,
        needs_reload: false,
        river_wm: None,
        windows: Vec::new(),
        outputs: HashMap::new(),
        main_seat: None,
        current_width: 0,
        current_height: 0,
        tag_focus_history: HashMap::new(),
        last_geometry: HashMap::new(),
        focused_window: None,
        focused_tags: 1,
        xkb_manager: None,
        key_bindings: Vec::new(),
        input_manager: None,
        xkb_config: None,
        keyboards: Vec::new(),
        current_keymap: None,
        layer_shell_manager: None,
        device_names: HashMap::new(),
        ipc_listener: Some(listener),
        cmd_listener: Some(cmd_listener),
        ipc_clients: Vec::new(),
        output_manager: None,
        heads: Vec::new(),
        last_output_serial: 0,
        layout_roots: HashMap::new(),
        focused_output: None,
        pending_pointer_warp: None,
        last_sent_json: String::new(),
        anonymous_ls_outputs: Vec::new(),
        wl_name_to_monitor_name: HashMap::new(),
        active_river_outputs: Vec::new(),
        floating_cascade_index: 0,
    };

    let _registry = display.get_registry(&qh, ());
    info!("rrwm has started and is listening for events...");

    // --- 核心：多路复用主循环 (Solid Event Loop) ---
    loop {
        // 1. 先处理队列中剩余的事件 (非阻塞)
        // 这一步很重要，防止有事件卡在 buffer 里没被处理
        if let Err(e) = event_queue.dispatch_pending(&mut state) {
            error!("Dispatch error: {:?}", e);
            break;
        }

        // 2. 将缓冲区里的请求发出去
        let _ = event_queue.flush();

        // 3. 准备读取新的 Wayland 事件
        // prepare_read() 会返回一个 Guard，如果返回 None 说明队列里还有事件没处理完，应该 continue 回去处理
        let guard = if let Some(g) = event_queue.prepare_read() {
            g
        } else {
            continue;
        };

        // 4. 构造 poll 监听列表
        // 我们监听三个东西：Wayland连接、广播Socket、指令Socket
        let mut fds = vec![
            libc::pollfd {
                fd: wayland_fd,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: ipc_fd,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: cmd_fd,
                events: libc::POLLIN,
                revents: 0,
            },
        ];

        // 5. 阻塞等待 (挂起 CPU，直到有任意一个 FD 变为可读)
        // timeout = -1 表示无限等待
        let ret = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, -1) };

        if ret > 0 {
            // --- 情况 A: Wayland 有新数据 ---
            if fds[0].revents & libc::POLLIN != 0 {
                // 真正的读取网络数据到缓冲区
                if let Err(e) = guard.read() {
                    error!("Read events error: {:?}", e);
                    break;
                }
            } else {
                // 如果 Wayland 没数据，但 poll 返回了，说明是其他 Socket 醒了。
                // 我们需要销毁 Guard，取消这次 Wayland 读取，否则下次循环会死锁。
                drop(guard);
            }

            // --- 情况 B: Waybar 连接进来 ---
            if fds[1].revents & libc::POLLIN != 0 {
                state.handle_ipc_connections();
            }

            // --- 情况 C: 指令查询进来 (rrwm --appid) ---
            if fds[2].revents & libc::POLLIN != 0 {
                // 这里直接处理，不用等 ManageStart
                state.handle_command_connections();
            }
        } else {
            // poll 出错或意外唤醒
            drop(guard);
        }
    }
}

/// 客户端模式：连接 Socket 并把收到的东西直接打印出来
fn run_waybar_client() {
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
        error!("Unable to connect to rrwm Socket, please ensure rrwm is running.");
        std::process::exit(1);
    }
}

// --- 查询客户端实现 ---
fn run_appid_client() {
    let display_name = std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-0".to_string());
    let socket_path = format!("/tmp/rrwm-{}-cmd.sock", display_name); // 注意这里连的是 cmd socket

    if let Ok(mut stream) = UnixStream::connect(&socket_path) {
        // 1. 发送查询指令
        if let Err(e) = stream.write_all(b"ls_clients") {
            error!("Failed to send command: {}", e);
            return;
        }

        // 2. 读取并打印结果
        let mut response = String::new();
        if let Ok(_) = stream.read_to_string(&mut response) {
            print!("{}", response);
        }
    } else {
        error!("Unable to connect to rrwm Command Socket. Is rrwm running?");
        std::process::exit(1);
    }
}

// --- 帮助信息函数 ---
fn print_help() {
    println!("Usage: rrwm [OPTIONS]");
    println!("");
    println!("Options:");
    println!("  --waybar    Run in Waybar client mode (receive JSON status stream)");
    println!("  --appid     List all active windows and their AppIDs");
    println!("  --help      Print this help message");
}
