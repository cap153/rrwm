pub mod layout;
pub mod actions;

use std::collections::HashMap;
use wayland_backend::client::ObjectId;
use wayland_client::protocol::wl_registry;
use wayland_client::{Connection, Dispatch, QueueHandle, Proxy};

// 导入生成的 River 协议
use crate::protocol::river::{
    river_node_v1::RiverNodeV1,
    river_output_v1::RiverOutputV1,
    river_seat_v1::RiverSeatV1,
    river_window_manager_v1::RiverWindowManagerV1,
    river_window_v1::{RiverWindowV1, Edges},
};

// 导入本地布局逻辑
use self::layout::{calculate_layout, Geometry, LayoutNode, SplitType};

#[derive(Debug, Clone)]
pub struct OutputData {
    pub width: i32,
    pub height: i32,
}

#[derive(Clone)]
pub struct WindowData {
    pub id: ObjectId,
    pub window: RiverWindowV1,
    pub node: Option<RiverNodeV1>,
}

pub struct AppState {
    pub river_wm: Option<RiverWindowManagerV1>,
    pub windows: Vec<WindowData>,
    pub outputs: HashMap<ObjectId, OutputData>,
    pub main_seat: Option<RiverSeatV1>,
    pub current_width: i32,
    pub current_height: i32,
    pub layout_root: Option<LayoutNode>,
    pub last_geometry: HashMap<ObjectId, Geometry>,
    pub focused_window: Option<ObjectId>,
}

// --- 1. 监听 WlRegistry (寻找全局接口) ---
impl Dispatch<wl_registry::WlRegistry, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
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

// --- 2. 核心：监听 RiverWindowManagerV1 (管理循环) ---
impl Dispatch<RiverWindowManagerV1, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &RiverWindowManagerV1,
        event: crate::protocol::river::river_window_manager_v1::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        use crate::protocol::river::river_window_manager_v1::Event as WmEvent;
        match event {
            WmEvent::Output { id: _ } => {
                // 由 Dispatch<RiverOutputV1> 处理详细分辨率
            }
            WmEvent::Seat { id } => {
                println!("-> 发现新输入设备 (Seat)");
                state.main_seat = Some(id);
            }
            WmEvent::Window { id } => {
                println!("-> 发现新窗口，执行 Cosmic 切割: {:?}", id.id());
                let new_data = WindowData {
                    id: id.id(),
                    window: id.clone(),
                    node: None,
                };
                state.windows.push(new_data.clone());

                // 执行自动切割逻辑
                match state.layout_root.take() {
                    None => {
                        state.layout_root = Some(LayoutNode::Window(new_data));
                    }
                    Some(mut root) => {
                        let (target_id, split) = if let Some(f_id) = &state.focused_window {
                            let split = if let Some(geo) = state.last_geometry.get(f_id) {
                                if geo.w > geo.h { SplitType::Vertical } else { SplitType::Horizontal }
                            } else {
                                SplitType::Vertical
                            };
                            (f_id.clone(), split)
                        } else {
                            // 无焦点时默认切分上一个窗口
                            let last_id = state.windows.get(state.windows.len().saturating_sub(2))
                                .map(|w| w.id.clone())
                                .unwrap();
                            (last_id, SplitType::Vertical)
                        };
                        root.insert_at(&target_id, new_data, split);
                        state.layout_root = Some(root);
                    }
                }

                // 自动聚焦
                state.focused_window = Some(id.id());
                if let Some(seat) = &state.main_seat {
                    seat.focus_window(&id);
                }
            }
            WmEvent::ManageStart => {
                // 同步焦点
                if let Some(f_id) = &state.focused_window {
                    if let Some(w_data) = state.windows.iter().find(|w| &w.id == f_id) {
                        if let Some(seat) = &state.main_seat {
                            seat.focus_window(&w_data.window);
                        }
                    }
                }
                // 计算并应用布局
                if let Some(root) = &state.layout_root {
                    let mut results = Vec::new();
                    let screen = Geometry { x: 0, y: 0, w: state.current_width, h: state.current_height };
                    calculate_layout(root, screen, &mut results);

                    state.last_geometry.clear();
                    for (window, geom) in results {
                        state.last_geometry.insert(window.id(), geom);
                        window.propose_dimensions(geom.w, geom.h);
                        window.set_tiled(Edges::all());
                    }
                }
                proxy.manage_finish();
            }
            WmEvent::RenderStart => {
                if let Some(root) = &state.layout_root {
                    let mut results = Vec::new();
                    let screen = Geometry { x: 0, y: 0, w: state.current_width, h: state.current_height };
                    calculate_layout(root, screen, &mut results);

                    for (window, geom) in results {
                        if let Some(w_data) = state.windows.iter_mut().find(|w| w.id == window.id()) {
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

    wayland_client::event_created_child!(AppState, RiverWindowManagerV1, [
        6 => (RiverWindowV1, ()),
        7 => (RiverOutputV1, ()),
        8 => (RiverSeatV1, ())
    ]);
}

// --- 3. 监听 RiverOutputV1 (获取屏幕分辨率) ---
impl Dispatch<RiverOutputV1, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &RiverOutputV1,
        event: crate::protocol::river::river_output_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use crate::protocol::river::river_output_v1::Event as OutEvent;
        match event {
            OutEvent::Dimensions { width, height } => {
                println!("-> 显示器分辨率更新: {}x{}", width, height);
                state.current_width = width;
                state.current_height = height;
                state.outputs.insert(proxy.id(), OutputData { width, height });
            }
            _ => {}
        }
    }
}

// --- 4. 监听 RiverSeatV1 (处理鼠标交互焦点) ---
impl Dispatch<RiverSeatV1, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &RiverSeatV1,
        event: crate::protocol::river::river_seat_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use crate::protocol::river::river_seat_v1::Event as SeatEvent;
        match event {
            SeatEvent::WindowInteraction { window } => {
                println!("-> 鼠标点击窗口: {:?}", window.id());
                state.focused_window = Some(window.id());
                proxy.focus_window(&window);
            }
            _ => {}
        }
    }
}

// --- 5. 监听 RiverWindowV1 (窗口关闭与智能填充) ---
impl Dispatch<RiverWindowV1, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &RiverWindowV1,
        event: crate::protocol::river::river_window_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use crate::protocol::river::river_window_v1::Event as WinEvent;
        match event {
            WinEvent::Closed => {
                let target_id = proxy.id();
                println!("-> 窗口已关闭: {:?}", target_id);

                if let Some(root) = state.layout_root.take() {
                    state.layout_root = LayoutNode::remove_at(root, &target_id);
                }

                state.windows.retain(|w| w.id != target_id);
                state.last_geometry.remove(&target_id);

                if state.focused_window.as_ref() == Some(&target_id) {
                    state.focused_window = state.windows.last().map(|w| w.id.clone());
                }
            }
            _ => {}
        }
    }
}

// --- 6. 监听 RiverNodeV1 (渲染节点，目前为空) ---
impl Dispatch<RiverNodeV1, ()> for AppState {
    fn event(_: &mut Self, _: &RiverNodeV1, _: crate::protocol::river::river_node_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
