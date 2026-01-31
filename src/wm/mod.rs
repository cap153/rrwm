pub mod actions;
pub mod binds;
pub mod layout;
use self::actions::Action;
use self::layout::{calculate_layout, Geometry, LayoutNode, SplitType};
use crate::protocol::river_input::river_input_device_v1::RiverInputDeviceV1;
use crate::protocol::river_input::river_input_manager_v1::RiverInputManagerV1;
use crate::protocol::river_layer_shell::river_layer_shell_output_v1::{
    Event as LayerOutEvent, RiverLayerShellOutputV1,
};
use crate::protocol::river_layer_shell::river_layer_shell_seat_v1::{
    Event as LayerSeatEvent, RiverLayerShellSeatV1,
};
use crate::protocol::river_layer_shell::river_layer_shell_v1::RiverLayerShellV1;
use crate::protocol::river_wm::{
    river_node_v1::RiverNodeV1,
    river_output_v1::RiverOutputV1,
    river_seat_v1::RiverSeatV1,
    river_window_manager_v1::RiverWindowManagerV1,
    river_window_v1::{Edges, RiverWindowV1},
};
use crate::protocol::river_xkb::{
    river_xkb_binding_v1::{Event as BindingEvent, RiverXkbBindingV1},
    river_xkb_bindings_v1::RiverXkbBindingsV1,
};
use crate::protocol::river_xkb_config::river_xkb_config_v1::{
    Event as ConfigEvent, KeymapFormat, RiverXkbConfigV1,
};
use crate::protocol::river_xkb_config::river_xkb_keyboard_v1::{
    Event as KbEvent, RiverXkbKeyboardV1,
};
use crate::protocol::river_xkb_config::river_xkb_keymap_v1::{
    Event as KeymapEvent, RiverXkbKeymapV1,
};
use std::collections::HashMap;
use std::io::Write;
use std::os::unix::io::AsFd;
use wayland_backend::client::ObjectId;
use wayland_client::protocol::wl_registry;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use xkbcommon::xkb;

/// 快捷键状态结构：将 River 绑定对象与本地 Action 关联
pub struct KeyBinding {
    pub obj: RiverXkbBindingV1,
    pub action: Action,
}

#[derive(Debug, Clone)]
pub struct OutputData {
    pub width: i32,
    pub height: i32,
    pub usable_area: Geometry,
}

#[derive(Clone)]
pub struct WindowData {
    pub id: ObjectId,
    pub window: RiverWindowV1,
    pub node: Option<RiverNodeV1>,
}

pub struct AppState {
    pub config: crate::config::Config,
    pub river_wm: Option<RiverWindowManagerV1>,
    pub windows: Vec<WindowData>,
    pub outputs: HashMap<ObjectId, OutputData>,
    pub main_seat: Option<RiverSeatV1>,
    pub current_width: i32,
    pub current_height: i32,
    pub layout_root: Option<LayoutNode>,
    pub last_geometry: HashMap<ObjectId, Geometry>,
    pub focused_window: Option<ObjectId>,
    pub xkb_manager: Option<RiverXkbBindingsV1>,
    pub key_bindings: Vec<KeyBinding>,
    pub input_manager: Option<RiverInputManagerV1>,
    pub xkb_config: Option<RiverXkbConfigV1>,
    pub keyboards: Vec<RiverXkbKeyboardV1>,
    pub layer_shell_manager: Option<RiverLayerShellV1>,
}

// --- 1. 监听 WlRegistry (寻找全局接口) ---
impl Dispatch<wl_registry::WlRegistry, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name, interface, ..
        } = event
        {
            match interface.as_str() {
                "river_layer_shell_v1" => {
                    println!("[ID:{}] 绑定：层级表面管理器 (Waybar 权限已开启)", name);
                    let manager = proxy.bind::<RiverLayerShellV1, _, _>(name, 1, qh, ());
                    state.layer_shell_manager = Some(manager);
                }
                "river_window_manager_v1" => {
                    let wm = proxy.bind::<RiverWindowManagerV1, _, _>(name, 3, qh, ());
                    state.river_wm = Some(wm);
                }
                "river_xkb_bindings_v1" => {
                    let xkb = proxy.bind::<RiverXkbBindingsV1, _, _>(name, 2, qh, ());
                    state.xkb_manager = Some(xkb);
                }
                "river_input_manager_v1" => {
                    let manager = proxy.bind::<RiverInputManagerV1, _, _>(name, 1, qh, ());
                    state.input_manager = Some(manager);
                }
                "river_xkb_config_v1" => {
                    let config = proxy.bind::<RiverXkbConfigV1, _, _>(name, 1, qh, ());
                    state.xkb_config = Some(config);
                }
                _ => {}
            }
        }
    }
}

// --- 2. 核心：监听 RiverWindowManagerV1 (管理循环) ---
impl Dispatch<RiverWindowManagerV1, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &RiverWindowManagerV1,
        event: crate::protocol::river_wm::river_window_manager_v1::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        use crate::protocol::river_wm::river_window_manager_v1::Event as WmEvent;
        match event {
            WmEvent::Seat { id } => {
                println!("-> 发现新输入设备 (Seat)");
                state.main_seat = Some(id.clone());
                // 2. 清理点：不再手动注册默认键，而是统一调用 binds 模块
                // 它会自动处理 TOML 配置或使用保底默认值
                self::binds::setup_keybindings(state, qh);
            }
            WmEvent::Window { id } => {
                println!("-> 发现新窗口，执行 Cosmic 切割: {:?}", id.id());
                let new_data = WindowData {
                    id: id.id(),
                    window: id.clone(),
                    node: None,
                };
                state.windows.push(new_data.clone());
                match state.layout_root.take() {
                    None => state.layout_root = Some(LayoutNode::Window(new_data)),
                    Some(mut root) => {
                        let (target_id, split) = if let Some(f_id) = &state.focused_window {
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
                            let last_id = state
                                .windows
                                .get(state.windows.len().saturating_sub(2))
                                .map(|w| w.id.clone())
                                .unwrap();
                            (last_id, SplitType::Vertical)
                        };
                        root.insert_at(&target_id, new_data, split);
                        state.layout_root = Some(root);
                    }
                }
                state.focused_window = Some(id.id());
                if let Some(seat) = &state.main_seat {
                    seat.focus_window(&id);
                }
            }
            WmEvent::ManageStart => {
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
                    state.last_geometry.clear();
                    for (window, geom) in results {
                        state.last_geometry.insert(window.id(), geom);
                        window.propose_dimensions(geom.w, geom.h);
                        window.set_tiled(Edges::all());
                    }
                }
                // 重要：在 ManageStart 事务中启用所有快捷键
                for kb in &state.key_bindings {
                    kb.obj.enable();
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
                    for (window, geom) in results {
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
            WmEvent::Output { id } => {
                // 之前可能为空，现在要处理
                if let Some(ls_mgr) = &state.layer_shell_manager {
                    // 为这个输出创建一个层级表面监听器
                    ls_mgr.get_output(&id, qh, ());
                }
            }
            _ => {}
        }
    }
    wayland_client::event_created_child!(AppState, RiverWindowManagerV1, [
        6 => (RiverWindowV1, ()), 7 => (RiverOutputV1, ()), 8 => (RiverSeatV1, ())
    ]);
}

// --- 3. 监听 RiverOutputV1 (分辨率) ---
impl Dispatch<RiverOutputV1, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &RiverOutputV1,
        event: crate::protocol::river_wm::river_output_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use crate::protocol::river_wm::river_output_v1::Event as OutEvent;
        if let OutEvent::Dimensions { width, height } = event {
            println!("-> 分辨率更新: {}x{}", width, height);
            state.current_width = width;
            state.current_height = height;
            // 修复点：提供完整的 OutputData 结构
            state.outputs.insert(
                proxy.id(),
                OutputData {
                    width,
                    height,
                    usable_area: Geometry {
                        x: 0,
                        y: 0,
                        w: width,
                        h: height,
                    }, // 初始占满全屏
                },
            );
        }
    }
}

// --- 4. 监听 RiverSeatV1 (点击聚焦) ---
impl Dispatch<RiverSeatV1, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &RiverSeatV1,
        event: crate::protocol::river_wm::river_seat_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use crate::protocol::river_wm::river_seat_v1::Event as SeatEvent;
        if let SeatEvent::WindowInteraction { window } = event {
            state.focused_window = Some(window.id());
            proxy.focus_window(&window);
        }
    }
}

// --- 5. 监听 RiverWindowV1 (关闭) ---
impl Dispatch<RiverWindowV1, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &RiverWindowV1,
        event: crate::protocol::river_wm::river_window_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use crate::protocol::river_wm::river_window_v1::Event as WinEvent;
        if let WinEvent::Closed = event {
            let id = proxy.id();
            if let Some(root) = state.layout_root.take() {
                state.layout_root = LayoutNode::remove_at(root, &id);
            }
            state.windows.retain(|w| w.id != id);
            state.last_geometry.remove(&id);
            if state.focused_window.as_ref() == Some(&id) {
                state.focused_window = state.windows.last().map(|w| w.id.clone());
            }
        }
    }
}

// --- 6. 监听 XKB 管理与事件 ---
impl Dispatch<RiverXkbBindingsV1, ()> for AppState {
    fn event(
        _: &mut Self,
        _: &RiverXkbBindingsV1,
        _: crate::protocol::river_xkb::river_xkb_bindings_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<RiverXkbBindingV1, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &RiverXkbBindingV1,
        event: BindingEvent,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let BindingEvent::Pressed = event {
            if let Some(kb) = state.key_bindings.iter().find(|b| b.obj.id() == proxy.id()) {
                state.perform_action(kb.action.clone());
            }
        }
    }
}

// --- 7. 键盘布局自动加载逻辑 ---
impl Dispatch<RiverXkbConfigV1, ()> for AppState {
    fn event(
        state: &mut Self,
        _: &RiverXkbConfigV1,
        event: ConfigEvent,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let ConfigEvent::XkbKeyboard { id } = event {
            if let Some(kb_cfg) = state
                .config
                .input
                .as_ref()
                .and_then(|i| i.keyboard.as_ref())
            {
                println!("-> 加载布局: {} ({:?})", kb_cfg.layout, kb_cfg.variant);
                let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
                let rules = "evdev".to_string();
                let model = kb_cfg.model.clone().unwrap_or_else(|| "pc105".to_string());
                let layout = kb_cfg.layout.clone();
                let variant = kb_cfg.variant.clone().unwrap_or_default();
                let options = kb_cfg.options.clone();

                let keymap = xkb::Keymap::new_from_names(
                    &context,
                    &rules,
                    &model,
                    &layout,
                    &variant,
                    options,
                    xkb::KEYMAP_COMPILE_NO_FLAGS,
                );
                if let Some(map) = keymap {
                    let keymap_str = map.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1);
                    let mut temp_file = tempfile::tempfile().expect("临时文件失败");
                    temp_file.write_all(keymap_str.as_bytes()).ok();
                    if let Some(mgr) = &state.xkb_config {
                        mgr.create_keymap(temp_file.as_fd(), KeymapFormat::TextV1, qh, ());
                        state.keyboards.push(id);
                    }
                }
            }
        }
    }
    wayland_client::event_created_child!(AppState, RiverXkbConfigV1, [1 => (RiverXkbKeyboardV1, ())]);
}

impl Dispatch<RiverXkbKeymapV1, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &RiverXkbKeymapV1,
        event: KeymapEvent,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let KeymapEvent::Success = event {
            for kb in &state.keyboards {
                kb.set_keymap(proxy);
            }
        }
    }
}

// --- 8. 空实现 ---
impl Dispatch<RiverInputManagerV1, ()> for AppState {
    fn event(
        _: &mut Self,
        _: &RiverInputManagerV1,
        _: crate::protocol::river_input::river_input_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
    wayland_client::event_created_child!(AppState, RiverInputManagerV1, [1 => (RiverInputDeviceV1, ())]);
}
impl Dispatch<RiverInputDeviceV1, ()> for AppState {
    fn event(
        _: &mut Self,
        _: &RiverInputDeviceV1,
        _: crate::protocol::river_input::river_input_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<RiverXkbKeyboardV1, ()> for AppState {
    fn event(
        _: &mut Self,
        _: &RiverXkbKeyboardV1,
        _: KbEvent,
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
        _: crate::protocol::river_wm::river_node_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<RiverLayerShellOutputV1, ()> for AppState {
    fn event(
        state: &mut Self,
        _proxy: &RiverLayerShellOutputV1,
        event: LayerOutEvent,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            LayerOutEvent::NonExclusiveArea {
                x,
                y,
                width,
                height,
            } => {
                println!("-> 可用区域更新: {}x{} @ {},{}", width, height, x, y);
                state.current_width = width;
                state.current_height = height;
                if let Some(out_data) = state.outputs.values_mut().next() {
                    out_data.usable_area = Geometry {
                        x,
                        y,
                        w: width,
                        h: height,
                    };
                }
            }
        }
    }
}
// 层级表面管理器本身（Waybar 的总开关）
impl Dispatch<RiverLayerShellV1, ()> for AppState {
    fn event(
        _: &mut Self,
        _: &RiverLayerShellV1,
        _: crate::protocol::river_layer_shell::river_layer_shell_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

// 层级表面关联的输入处理（比如点击 Waybar 上的按钮）
impl Dispatch<RiverLayerShellSeatV1, ()> for AppState {
    fn event(
        _: &mut Self,
        _: &RiverLayerShellSeatV1,
        _: LayerSeatEvent,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
