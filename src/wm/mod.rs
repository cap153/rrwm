pub mod actions;
pub mod layout;

use self::actions::Action;
use self::layout::{calculate_layout, Geometry, LayoutNode, SplitType};
use crate::protocol::river_input::river_input_device_v1::RiverInputDeviceV1;
use crate::protocol::river_input::river_input_manager_v1::RiverInputManagerV1;
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

pub struct KeyBinding {
    pub obj: RiverXkbBindingV1,
    pub action: Action,
}

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
            // 使用 match 代替 if，确保逻辑唯一性
            match interface.as_str() {
                "river_window_manager_v1" => {
                    println!("[ID:{}] 绑定：窗口管理器", name);
                    // 注意：这里必须指定版本为 3
                    let wm = proxy.bind::<RiverWindowManagerV1, _, _>(name, 3, qh, ());
                    state.river_wm = Some(wm);
                }
                "river_xkb_bindings_v1" => {
                    println!("[ID:{}] 绑定：快捷键管理器", name);
                    // 注意：这里版本是 2
                    let xkb = proxy.bind::<RiverXkbBindingsV1, _, _>(name, 2, qh, ());
                    state.xkb_manager = Some(xkb);
                }
                "river_input_manager_v1" => {
                    println!("[ID:{}] 绑定：输入管理器", name);
                    let manager = proxy.bind::<RiverInputManagerV1, _, _>(name, 1, qh, ());
                    state.input_manager = Some(manager);
                }
                "river_xkb_config_v1" => {
                    println!("[ID:{}] 绑定：XKB 配置器", name);
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
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        use crate::protocol::river_wm::river_window_manager_v1::Event as WmEvent;
        match event {
            WmEvent::Output { id: _ } => {
                // 由 Dispatch<RiverOutputV1> 处理详细分辨率
            }
            // src/wm/mod.rs 里的 WmEvent::Seat 分支
            WmEvent::Seat { id } => {
                println!("-> 发现新输入设备 (Seat)");
                state.main_seat = Some(id.clone());

                if let Some(xkb_mgr) = &state.xkb_manager {
                    // 这里 qh 已经在 event 函数参数里了，直接用
                    let defaults = crate::config::get_default_bindings();

                    for b in defaults {
                        // 将 key 字符串转为 keysym 并取原始 u32 值
                        let keysym = xkb::keysym_from_name(b.key, xkb::KEYSYM_NO_FLAGS).raw();

                        // 注意这里的 b.mods，它已经是 Modifiers 类型了
                        let binding_obj = xkb_mgr.get_xkb_binding(&id, keysym, b.mods, qh, ());

                        state.key_bindings.push(KeyBinding {
                            obj: binding_obj,
                            action: b.action,
                        });
                    }
                }
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
                            // 无焦点时默认切分上一个窗口
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
        event: crate::protocol::river_wm::river_output_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use crate::protocol::river_wm::river_output_v1::Event as OutEvent;
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

// --- 4. 监听 RiverSeatV1 (处理鼠标交互焦点) ---
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
        event: crate::protocol::river_wm::river_window_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use crate::protocol::river_wm::river_window_v1::Event as WinEvent;
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

// 处理快捷键全局管理器
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

// 处理具体的按键按下/抬起事件
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
            // 查找是哪个 Action 绑定的这个对象 ID
            if let Some(kb) = state.key_bindings.iter().find(|b| b.obj.id() == proxy.id()) {
                state.perform_action(kb.action.clone());
            }
        }
    }
}
// 1. 处理 XKB 配置管理器
impl Dispatch<RiverXkbConfigV1, ()> for AppState {
    fn event(
        state: &mut Self,
        _proxy: &RiverXkbConfigV1,
        event: ConfigEvent,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let ConfigEvent::XkbKeyboard { id } = event {
            // 1. 尝试从配置中获取键盘布局设置 (类似 JS 的 config.input?.keyboard)
            if let Some(kb_cfg) = state
                .config
                .input
                .as_ref()
                .and_then(|i| i.keyboard.as_ref())
            {
                println!(
                    "-> 检测到配置，准备为键盘加载布局: {} ({:?})",
                    kb_cfg.layout, kb_cfg.variant
                );

                let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);

                // 2. 将 TOML 里的字符串传给 xkbcommon

                // 先把这些可选参数提取出来，强制它们变成 &str
                let rules = "evdev".to_string();
                let model = kb_cfg.model.clone().unwrap_or_else(|| "pc105".to_string());
                let layout = kb_cfg.layout.clone();
                let variant = kb_cfg.variant.clone().unwrap_or_default();
                let options = kb_cfg.options.clone();

                let keymap = xkb::Keymap::new_from_names(
                    &context,
                    &rules,   // 传入 &String，匹配 &S
                    &model,   // 传入 &String，匹配 &S
                    &layout,  // 传入 &String，匹配 &S
                    &variant, // 传入 &String，匹配 &S
                    options,  // 传入 Option<String>，匹配 Option<S>
                    xkb::KEYMAP_COMPILE_NO_FLAGS,
                );

                if let Some(keymap) = keymap {
                    let keymap_str = keymap.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1);
                    let mut temp_file = tempfile::tempfile().expect("无法创建临时文件");
                    temp_file
                        .write_all(keymap_str.as_bytes())
                        .expect("无法写入键位图");

                    if let Some(mgr) = &state.xkb_config {
                        // 3. 递交“钥匙”
                        mgr.create_keymap(temp_file.as_fd(), KeymapFormat::TextV1, qh, ());
                        state.keyboards.push(id);
                    }
                }
            } else {
                println!("-> 配置文件中未设置键盘布局，保持系统默认（QWERTY）。");
            }
        }
    }
    // 注册子对象：键盘
    wayland_client::event_created_child!(AppState, RiverXkbConfigV1, [
        1 => (RiverXkbKeyboardV1, ())
    ]);
}

// 2. 处理 Keymap 对象的成功/失败反馈
impl Dispatch<RiverXkbKeymapV1, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &RiverXkbKeymapV1,
        event: KeymapEvent,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            KeymapEvent::Success => {
                println!("-> 键位图创建成功，正在应用到所有键盘...");
                for kb in &state.keyboards {
                    kb.set_keymap(proxy); // 终于把 Colemak 设置上去了！
                }
            }
            KeymapEvent::Failure { error_msg } => {
                eprintln!("-> 键位图加载失败: {}", error_msg);
            }
        }
    }
}

// 3. 其他必须有的空 Dispatch 实现，防止崩溃
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
    wayland_client::event_created_child!(AppState, RiverInputManagerV1, [
        1 => (RiverInputDeviceV1, ())
    ]);
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
