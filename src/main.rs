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
impl LayoutNode {
    // 这是一个递归函数，用来在树里插入新窗口
    fn insert_at(
        &mut self,
        target_id: &wayland_backend::client::ObjectId,
        new_win: WindowData,
        split: SplitType,
    ) -> bool {
        match self {
            LayoutNode::Window(w_data) => {
                if &w_data.id == target_id {
                    // 找到了！执行“细胞分裂”
                    // 1. 把当前的窗口数据取出来
                    let old_win = w_data.clone();
                    // 2. 把当前节点从 Window 变成 Container
                    *self = LayoutNode::Container {
                        split_type: split,
                        ratio: 0.5,
                        left_child: Box::new(LayoutNode::Window(old_win)),
                        right_child: Box::new(LayoutNode::Window(new_win)),
                    };
                    return true;
                }
                false
            }
            LayoutNode::Container {
                left_child,
                right_child,
                ..
            } => {
                // 如果不是，继续往子节点找
                if left_child.insert_at(target_id, new_win.clone(), split) {
                    return true;
                }
                right_child.insert_at(target_id, new_win, split)
            }
        }
    }
    // 递归删除函数
    // 返回值：Option<LayoutNode> 是处理后的新节点。如果返回 None，说明该分支被删空了。
    fn remove_at(
        node: LayoutNode,
        target_id: &wayland_backend::client::ObjectId,
    ) -> Option<LayoutNode> {
        match node {
            LayoutNode::Window(w_data) => {
                if &w_data.id == target_id {
                    // 如果自己就是要删除的窗口，返回 None（消失）
                    None
                } else {
                    // 否则保留自己
                    Some(LayoutNode::Window(w_data))
                }
            }
            LayoutNode::Container {
                split_type,
                ratio,
                left_child,
                right_child,
            } => {
                // 递归处理子节点
                let new_left = LayoutNode::remove_at(*left_child, target_id);
                let new_right = LayoutNode::remove_at(*right_child, target_id);

                match (new_left, new_right) {
                    // 1. 左右都还在：保持容器结构
                    (Some(l), Some(r)) => Some(LayoutNode::Container {
                        split_type,
                        ratio,
                        left_child: Box::new(l),
                        right_child: Box::new(r),
                    }),
                    // 2. 左边没了：右边“上位”替代父容器
                    (None, Some(r)) => Some(r),
                    // 3. 右边没了：左边“上位”替代父容器
                    (None, None) => None, // 逻辑上不应该出现，除非全是空的
                    (Some(l), None) => Some(l),
                }
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
                let new_data = WindowData {
                    id: id.id(),
                    window: id.clone(),
                    node: None,
                };
                state.windows.push(new_data.clone());

                match state.layout_root.take() {
                    None => {
                        state.layout_root = Some(LayoutNode::Window(new_data));
                    }
                    Some(mut root) => {
                        // 1. 找到当前聚焦窗口的坐标
                        let (target_id, split) = if let Some(f_id) = &state.focused_window {
                            // 查找这个窗口的宽高比
                            let split = if let Some(geo) = state.last_geometry.get(f_id) {
                                if geo.w > geo.h {
                                    SplitType::Vertical
                                } else {
                                    SplitType::Horizontal
                                }
                            } else {
                                SplitType::Vertical
                            };
                            (f_id.clone(), split)
                        } else {
                            // 如果没焦点，就找最后一个窗口
                            let last_id = state
                                .windows
                                .get(state.windows.len() - 2)
                                .map(|w| w.id.clone())
                                .unwrap();
                            (last_id, SplitType::Vertical)
                        };

                        // 2. 在树中执行切割
                        root.insert_at(&target_id, new_data, split);
                        state.layout_root = Some(root);
                    }
                }

                // 记得更新焦点
                state.focused_window = Some(id.id());
                if let Some(seat) = &state.main_seat {
                    seat.focus_window(&id);
                }
            }
            WmEvent::ManageStart => {
                // 在 ManageStart 里的焦点逻辑部分：
                if let Some(f_id) = &state.focused_window {
                    if let Some(w_data) = state.windows.iter().find(|w| &w.id == f_id) {
                        if let Some(seat) = &state.main_seat {
                            seat.focus_window(&w_data.window);
                        }
                    }
                }
                if let Some(root) = &state.layout_root {
                    let mut results = Vec::new();
                    let screen = Geometry {
                        x: 0,
                        y: 0,
                        w: state.current_width,
                        h: state.current_height,
                    };
                    calculate_layout(root, screen, &mut results);

                    // 关键：清空并记录当前所有窗口的坐标
                    state.last_geometry.clear();
                    // 修改 ManageStart 内部的循环
                    for (window, geom) in results {
                        state.last_geometry.insert(window.id(), geom);
                        window.propose_dimensions(geom.w, geom.h);
                        window.set_tiled(
                            river_protocol::river_window_v1::Edges::all(),
                        );
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
        state: &mut Self,
        proxy: &RiverSeatV1, // 这个 proxy 就是当前的 seat (鼠标/键盘组合)
        event: river_protocol::river_seat_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        use river_protocol::river_seat_v1::Event as SeatEvent;
        match event {
            // 当用户点击或触摸某个窗口时触发
            SeatEvent::WindowInteraction { window } => {
                println!("-> 鼠标点击窗口: {:?}", window.id());
                // 1. 更新我们的全局状态，记录现在谁是焦点
                state.focused_window = Some(window.id());
                // 2. 关键指令：告诉 River 正式把键盘焦点给这个窗口
                proxy.focus_window(&window);
            }

            // 可选：实现“焦点跟随鼠标”（鼠标移上去就聚焦，不需要点击）
            // 如果你喜欢这种风格，可以取消下面这一行的注释：
            // SeatEvent::PointerEnter { window } => { proxy.focus_window(&window); state.focused_window = Some(window.id()); }
            _ => {}
        }
    }
}
impl Dispatch<RiverWindowV1, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &RiverWindowV1,
        event: river_protocol::river_window_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        use river_protocol::river_window_v1::Event as WinEvent;
        match event {
            WinEvent::Closed => {
                let target_id = proxy.id();
                println!("-> 窗口已关闭: {:?}", target_id);

                // 1. 从树中移除（智能填充逻辑）
                if let Some(root) = state.layout_root.take() {
                    state.layout_root = LayoutNode::remove_at(root, &target_id);
                }

                // 2. 从 flat 列表中移除
                state.windows.retain(|w| w.id != target_id);

                // 3. 更新焦点：如果关掉的是焦点窗口，把焦点给剩下的最后一个窗口
                if state.focused_window.as_ref() == Some(&target_id) {
                    state.focused_window = state.windows.last().map(|w| w.id.clone());
                }

                // 注意：这里不需要手动重画，River 随后会自动发 ManageStart
            }
            _ => {}
        }
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
        last_geometry: HashMap::new(), // 初始时没有任何几何信息记录，用空的 HashMap
        focused_window: None,          // 初始时没有任何窗口获得焦点，用 None
    };
    let _registry = display.get_registry(&qh, ());

    println!("rrwm 已启动，正在监听事件...");

    loop {
        // 就像 JS 的事件循环一样，永远跑下去
        event_queue.blocking_dispatch(&mut state).unwrap();
    }
}
