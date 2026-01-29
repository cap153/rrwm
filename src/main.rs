pub mod river_protocol {
    pub extern crate bitflags;
    pub extern crate wayland_backend;
    pub extern crate wayland_client;
    pub use wayland_client::protocol::{wl_output, wl_seat, wl_surface};

    pub mod __interfaces {
        pub use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("./protocols/river-window-management-v1.xml");
    }
    use self::__interfaces::*;
    wayland_scanner::generate_client_code!("./protocols/river-window-management-v1.xml");
}

use river_protocol::river_node_v1::RiverNodeV1;
use river_protocol::river_output_v1::RiverOutputV1;
use river_protocol::river_seat_v1::RiverSeatV1;
use river_protocol::river_window_manager_v1::RiverWindowManagerV1;
use river_protocol::river_window_v1::RiverWindowV1;
use std::collections::HashMap;
use wayland_client::{protocol::wl_registry, Connection, Dispatch, Proxy, QueueHandle};

struct OutputData {
    width: i32,
    height: i32,
}

// 给 WindowData 增加一个 ID，方便查找
#[derive(Clone)] // 让 WindowData 也可以克隆
struct WindowData {
    id: wayland_backend::client::ObjectId,
    window: RiverWindowV1,
    node: Option<RiverNodeV1>,
}

// 增加一个简单的结构体来存放计算好的几何数据
#[derive(Debug, Clone, Copy)]
struct Geometry {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

struct AppState {
    river_wm: Option<RiverWindowManagerV1>,
    windows: Vec<WindowData>,
    outputs: HashMap<wayland_backend::client::ObjectId, OutputData>,
    main_seat: Option<RiverSeatV1>,
    current_width: i32,
    current_height: i32,
    layout_root: Option<LayoutNode>, // 这就是我们的“大脑”
    // 记录每个窗口 ID 对应的位置和大小
    last_geometry: HashMap<wayland_backend::client::ObjectId, Geometry>,
    // 当前聚焦的窗口 ID
    focused_window: Option<wayland_backend::client::ObjectId>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum SplitType {
    Horizontal, // 上下平分
    Vertical,   // 左右平分
}

enum LayoutNode {
    // 这是一个具体的窗口
    Window(WindowData),
    // 这是一个“容器”，它把空间一分为二
    Container {
        split_type: SplitType,
        ratio: f32, // 切分比例，通常是 0.5
        left_child: Box<LayoutNode>,
        right_child: Box<LayoutNode>,
    },
}

fn calculate_layout(
    node: &LayoutNode,
    area: Geometry,
    results: &mut Vec<(RiverWindowV1, Geometry)>,
) {
    match node {
        LayoutNode::Window(w_data) => {
            // 如果是叶子节点（窗口），直接记录它的坐标
            results.push((w_data.window.clone(), area));
        }
        LayoutNode::Container {
            split_type,
            ratio,
            left_child,
            right_child,
        } => {
            // 如果是容器，根据比例切分空间
            if *split_type == SplitType::Vertical {
                // 纵切（左右分）
                let left_w = (area.w as f32 * ratio) as i32;
                let right_w = area.w - left_w;

                // 递归左边
                calculate_layout(left_child, Geometry { w: left_w, ..area }, results);
                // 递归右边
                calculate_layout(
                    right_child,
                    Geometry {
                        x: area.x + left_w,
                        w: right_w,
                        ..area
                    },
                    results,
                );
            } else {
                // 横切（上下分）
                let top_h = (area.h as f32 * ratio) as i32;
                let bottom_h = area.h - top_h;

                calculate_layout(left_child, Geometry { h: top_h, ..area }, results);
                calculate_layout(
                    right_child,
                    Geometry {
                        y: area.y + top_h,
                        h: bottom_h,
                        ..area
                    },
                    results,
                );
            }
        }
    }
}
// 监听注册表（全局菜单）
impl Dispatch<wl_registry::WlRegistry, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        // 把 _version 改成 version: _
        if let wl_registry::Event::Global {
            name,
            interface,
            version: _,
        } = event
        {
            if interface == "river_window_manager_v1" {
                println!("[ID:{}] 发现 River 管理接口，开始绑定...", name);
                let wm = proxy.bind::<RiverWindowManagerV1, _, _>(name, 3, qh, ());
                state.river_wm = Some(wm);
            }
        }
    }
}

// 监听 River 管理器（核心业务）
// 修改这一块逻辑
// 找到这一块，进行替换
impl Dispatch<RiverWindowManagerV1, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &RiverWindowManagerV1,
        event: river_protocol::river_window_manager_v1::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        use river_protocol::river_window_manager_v1::Event as WmEvent;
        // 找到 match event 部分，修改如下：
        match event {
            WmEvent::Output { id: _ } => println!("-> 发现新显示器"),
            WmEvent::Seat { id } => {
                println!("-> 发现新输入设备 (Seat)");
                state.main_seat = Some(id);
            }
            WmEvent::Window { id } => {
                println!("-> 发现新窗口，执行 Cosmic 切割");
                let new_win_data = WindowData {
                    id: id.id(),
                    window: id.clone(),
                    node: None,
                };

                // 把新窗口存入 flat 列表（用于查找）
                state.windows.push(new_win_data.clone());

                // --- 核心：Cosmic 自动切割树 ---
                match state.layout_root.take() {
                    None => {
                        // 如果是第一个窗口，直接设为根
                        state.layout_root = Some(LayoutNode::Window(new_win_data));
                    }
                    Some(old_root) => {
                        // 根据当前屏幕比例决定切割方向
                        let split = if state.current_width > state.current_height {
                            SplitType::Vertical
                        } else {
                            SplitType::Horizontal
                        };

                        // 创建新容器，旧内容在左/上，新窗口在右/下
                        state.layout_root = Some(LayoutNode::Container {
                            split_type: split,
                            ratio: 0.5,
                            left_child: Box::new(old_root),
                            right_child: Box::new(LayoutNode::Window(new_win_data)),
                        });
                    }
                }

                // 自动聚焦
                if let Some(seat) = &state.main_seat {
                    seat.focus_window(&id);
                }
            }
            WmEvent::ManageStart => {
                if let Some(root) = &state.layout_root {
                    let mut results = Vec::new();
                    let screen = Geometry {
                        x: 0,
                        y: 0,
                        w: state.current_width,
                        h: state.current_height,
                    };

                    // 1. 计算布局
                    calculate_layout(root, screen, &mut results);

                    // 2. 告诉每个窗口它该有多大
                    for (window, geom) in results {
                        window.propose_dimensions(geom.w, geom.h);
                    }
                }
                proxy.manage_finish();
            }
            WmEvent::RenderStart => {
                if let Some(root) = &state.layout_root {
                    let mut results = Vec::new();
                    let screen = Geometry {
                        x: 0,
                        y: 0,
                        w: state.current_width,
                        h: state.current_height,
                    };
                    calculate_layout(root, screen, &mut results);

                    // 3. 设置位置
                    for (window, geom) in results {
                        // 在 flat 列表中找到对应的 WindowData 来获取/创建 Node
                        if let Some(w_data) = state.windows.iter_mut().find(|w| w.id == window.id())
                        {
                            if w_data.node.is_none() {
                                w_data.node = Some(window.get_node(qh, ()));
                            }
                            if let Some(node) = &w_data.node {
                                node.set_position(geom.x, geom.y);
                                node.place_top();
                            }
                        }
                    }
                }
                proxy.render_finish();
            }
            _ => {}
        }
    }

    // 使用文档推荐的“快捷方式”
    // 参数 1: 你的全局状态结构体名字
    // 参数 2: 当前这个 Dispatch 对应的接口
    // 参数 3: 一个映射表 [ 编号 => (子对象类型, 附加数据类型) ]
    wayland_client::event_created_child!(AppState, RiverWindowManagerV1, [
        6 => (RiverWindowV1, ()),   // window 事件创建窗口对象
        7 => (RiverOutputV1, ()),   // output 事件创建显示器对象
        8 => (RiverSeatV1, ())      // seat 事件创建输入设备对象
    ]);
}

// 注意：我们要为这些“新发现”的东西也写 Dispatch，即使现在不做什么
// 否则程序会因为不知道怎么处理这些对象而崩溃
impl Dispatch<RiverOutputV1, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &RiverOutputV1,
        event: river_protocol::river_output_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        use river_protocol::river_output_v1::Event as OutEvent;
        match event {
            OutEvent::Dimensions { width, height } => {
                println!("-> 显示器分辨率更新: {}x{}", width, height);
                state.current_width = width;
                state.current_height = height;
                // 同时更新 HashMap 里的数据
                state
                    .outputs
                    .insert(proxy.id(), OutputData { width, height });
            }
            _ => {}
        }
    }
}
impl Dispatch<RiverSeatV1, ()> for AppState {
    fn event(
        _: &mut Self,
        _: &river_protocol::river_seat_v1::RiverSeatV1,
        _: river_protocol::river_seat_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<RiverWindowV1, ()> for AppState {
    fn event(
        _: &mut Self,
        _: &river_protocol::river_window_v1::RiverWindowV1,
        _: river_protocol::river_window_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<RiverNodeV1, ()> for AppState {
    fn event(
        _: &mut Self,
        _: &RiverNodeV1,
        _: river_protocol::river_node_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

fn main() {
    let conn = Connection::connect_to_env().expect("请在 River 环境下运行");
    let display = conn.display();
    let mut event_queue = conn.new_event_queue();
    let qh = event_queue.handle();

    let mut state = AppState {
        river_wm: None,
        windows: Vec::new(),
        outputs: HashMap::new(), // 初始化哈希表
        main_seat: None,
        current_width: 0,
        current_height: 0,
        layout_root: None,
    };
    let _registry = display.get_registry(&qh, ());

    println!("rrwm 已启动，正在监听事件...");

    loop {
        // 就像 JS 的事件循环一样，永远跑下去
        event_queue.blocking_dispatch(&mut state).unwrap();
    }
}
