pub mod actions;
pub mod binds;
pub mod layout;
use self::actions::Action;
use self::layout::{calculate_layout, Geometry, LayoutNode, SplitType};
use crate::protocol::river_input::river_input_device_v1::{
    Event as InputDeviceEvent, RiverInputDeviceV1,
};
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
use crate::protocol::wlr_output_management::{
    zwlr_output_configuration_head_v1::ZwlrOutputConfigurationHeadV1,
    zwlr_output_configuration_v1::{Event as ConfigResEvent, ZwlrOutputConfigurationV1},
    zwlr_output_head_v1::{Event as HeadEvent, ZwlrOutputHeadV1},
    zwlr_output_manager_v1::{Event as MgrEvent, ZwlrOutputManagerV1},
    zwlr_output_mode_v1::{Event as ModeEvent, ZwlrOutputModeV1},
};
use std::collections::HashMap;
use std::io::Write;
use std::os::unix::io::AsFd;
use std::os::unix::net::{UnixListener, UnixStream};
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
    pub ls_output: Option<RiverLayerShellOutputV1>,
    // pub raw_output: RiverOutputV1,
}

#[derive(Clone)]
pub struct WindowData {
    pub id: ObjectId,
    pub window: RiverWindowV1,
    pub node: Option<RiverNodeV1>,
    pub tags: u32,
    pub app_id: Option<String>,
}

pub struct ModeInfo {
    pub obj: ZwlrOutputModeV1,
    pub width: i32,
    pub height: i32,
    pub refresh: i32,
}

pub struct HeadInfo {
    pub obj: ZwlrOutputHeadV1,
    pub name: String,
    pub modes: Vec<ModeInfo>,
    pub current_mode: Option<ObjectId>, // 记录当前生效的是哪个模式
}

pub struct AppState {
    pub config: crate::config::Config,
    pub needs_reload: bool,
    pub river_wm: Option<RiverWindowManagerV1>,
    pub windows: Vec<WindowData>,
    pub outputs: HashMap<ObjectId, OutputData>,
    pub main_seat: Option<RiverSeatV1>,
    pub current_width: i32,
    pub current_height: i32,
    pub layout_roots: HashMap<u32, LayoutNode>,
    pub tag_focus_history: HashMap<u32, ObjectId>,
    pub last_geometry: HashMap<ObjectId, Geometry>,
    pub focused_window: Option<ObjectId>,
    pub focused_tags: u32,
    pub xkb_manager: Option<RiverXkbBindingsV1>,
    pub key_bindings: Vec<KeyBinding>,
    pub input_manager: Option<RiverInputManagerV1>,
    pub xkb_config: Option<RiverXkbConfigV1>,
    pub keyboards: Vec<RiverXkbKeyboardV1>,
    pub current_keymap: Option<RiverXkbKeymapV1>,
    pub layer_shell_manager: Option<RiverLayerShellV1>,
    pub device_names: HashMap<ObjectId, String>,
    pub ipc_listener: Option<UnixListener>,
    pub ipc_clients: Vec<UnixStream>,
    pub output_manager: Option<ZwlrOutputManagerV1>,
    pub heads: Vec<HeadInfo>,
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
                "zwlr_output_manager_v1" => {
                    println!("[ID:{}] 发现显示器管理器 (wlr-output-management)", name);
                    let manager = proxy.bind::<ZwlrOutputManagerV1, _, _>(name, 4, qh, ());
                    state.output_manager = Some(manager);
                }
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
                // 仅预登记，不执行切割，也不分配焦点
                println!("-> 发现新窗口，等待分配 AppId: {:?}", id.id());
                state.windows.push(WindowData {
                    id: id.id(),
                    window: id.clone(),
                    node: None,
                    tags: state.focused_tags, // 预分配到当前标签
                    app_id: None,
                });
            }
            WmEvent::ManageStart => {
                // --- IPC 广播逻辑 ---
                // 看看有没有新的 Waybar 想听广播
                state.handle_ipc_connections();
                // 向所有在线的听众播报当前的标签和窗口状态
                state.broadcast_status();

                // --- 智能焦点恢复 ---
                // 逻辑：如果当前焦点窗口不存在，或者当前焦点窗口在当前标签页不可见，才尝试恢复历史
                let needs_restore = match &state.focused_window {
                    Some(f_id) => {
                        // 检查当前焦点是否在当前标签页
                        !state
                            .windows
                            .iter()
                            .any(|w| &w.id == f_id && (w.tags & state.focused_tags) != 0)
                    }
                    None => true,
                };

                if needs_restore {
                    if let Some(history_id) = state.tag_focus_history.get(&state.focused_tags) {
                        state.focused_window = Some(history_id.clone());
                    } else {
                        // 如果历史也没有，尝试抓取当前标签页下的第一个窗口
                        state.focused_window = state
                            .windows
                            .iter()
                            .find(|w| (w.tags & state.focused_tags) != 0)
                            .map(|w| w.id.clone());
                    }
                }

                // B. 显隐控制：只显示属于当前活跃标签的窗口
                for w_data in &state.windows {
                    // 只有拿到 AppId 且不是输入法的窗口，我们才去管它的死活
                    if let Some(ref app_id) = w_data.app_id {
                        if app_id.contains("fcitx") {
                            continue; // 输入法窗口保持隐身，交给 River 默认处理
                        }

                        if (w_data.tags & state.focused_tags) != 0 {
                            w_data.window.show();
                        } else {
                            w_data.window.hide();
                        }
                    }
                }

                // C. 焦点确认：告诉 River 真正把键盘给谁
                if let Some(f_id) = &state.focused_window {
                    if let Some(w_data) = state.windows.iter().find(|w| &w.id == f_id) {
                        // 只有当窗口真的可见时才请求焦点
                        if (w_data.tags & state.focused_tags) != 0 {
                            if let Some(seat) = &state.main_seat {
                                seat.focus_window(&w_data.window);
                            }
                        }
                    }
                }

                // D. 布局计算：只计算并渲染当前标签的树
                if let Some(root) = state.layout_roots.get(&state.focused_tags) {
                    let mut results = Vec::new();
                    let screen = if let Some(out_data) = state.outputs.values().next() {
                        out_data.usable_area
                    } else {
                        Geometry {
                            x: 0,
                            y: 0,
                            w: state.current_width,
                            h: state.current_height,
                        }
                    };

                    calculate_layout(root, screen, &mut results);

                    state.last_geometry.clear();
                    for (window, geom) in results {
                        // 再次检查 ID 确保它是我们记录在案的有效窗口
                        if let Some(w_info) = state.windows.iter().find(|w| w.id == window.id()) {
                            if w_info.app_id.is_some() {
                                state.last_geometry.insert(window.id(), geom);
                                window.propose_dimensions(geom.w, geom.h);
                                window.set_tiled(Edges::all());
                            }
                        }
                    }
                }

                // --- 激活层级表面默认输出 ---
                for out_data in state.outputs.values() {
                    if let Some(ls_out) = &out_data.ls_output {
                        // 在管理事务中正式宣布：这个显示器可以画 Waybar 和壁纸了！
                        ls_out.set_default();
                    }
                }
                for kb in &state.key_bindings {
                    kb.obj.enable();
                }
                proxy.manage_finish();
            }
            WmEvent::RenderStart => {
                // 修正点：从哈希表中获取当前活跃标签的布局树
                if let Some(root) = state.layout_roots.get(&state.focused_tags) {
                    let mut results = Vec::new();
                    let screen = if let Some(out_data) = state.outputs.values().next() {
                        out_data.usable_area
                    } else {
                        Geometry {
                            x: 0,
                            y: 0,
                            w: state.current_width,
                            h: state.current_height,
                        }
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
                println!("-> 发现新显示器，正在准备层级管理...");

                let mut ls_out = None;
                if let Some(ls_mgr) = &state.layer_shell_manager {
                    // 只调用一次！并保存这个对象
                    ls_out = Some(ls_mgr.get_output(&id, qh, ()));
                }

                state.outputs.insert(
                    id.id(),
                    OutputData {
                        width: 0, // 等待 Dimensions 事件填充
                        height: 0,
                        usable_area: Geometry {
                            x: 0,
                            y: 0,
                            w: 0,
                            h: 0,
                        },
                        ls_output: ls_out,
                        // raw_output: id.clone(), // 存下这个“人”，以后补证用
                    },
                );
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

            // 找到现有的 OutputData 并更新它
            if let Some(data) = state.outputs.get_mut(&proxy.id()) {
                data.width = width;
                data.height = height;
                // 如果 usable_area 还没初始化，就先设为全屏
                if data.usable_area.w == 0 {
                    data.usable_area = Geometry {
                        x: 0,
                        y: 0,
                        w: width,
                        h: height,
                    };
                }
            }
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
            let id = window.id();
            println!("-> 鼠标点击窗口: {:?}", id);

            // 1. 更新全局焦点
            state.focused_window = Some(id.clone());

            // 2. 关键点：找到窗口所属的 Tag，并同步更新该 Tag 的焦点历史
            if let Some(w_info) = state.windows.iter().find(|w| w.id == id) {
                state.tag_focus_history.insert(w_info.tags, id.clone());
            }

            proxy.focus_window(&window);
        }
    }
}

// --- 5. 监听每个具体窗口发出的事件 ---
impl Dispatch<RiverWindowV1, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &RiverWindowV1, // 这里的 proxy 就是发来“关闭信号”的那个窗口
        event: crate::protocol::river_wm::river_window_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use crate::protocol::river_wm::river_window_v1::Event as WinEvent;

        match event {
            // 当窗口被关闭（比如在终端里输了 exit）
            WinEvent::Closed => {
                let id = proxy.id();
                println!("-> 窗口已关闭: {:?}", id);

                // 1. 查找这个窗口所在的标签
                if let Some(w_info) = state.windows.iter().find(|w| w.id == id) {
                    let win_tag = w_info.tags;

                    // 2. 从对应标签的树中移除它（智能填充逻辑）
                    if let Some(root) = state.layout_roots.remove(&win_tag) {
                        // LayoutNode::remove_at 是你在 layout.rs 里实现的递归删除
                        if let Some(new_root) = LayoutNode::remove_at(root, &id) {
                            state.layout_roots.insert(win_tag, new_root);
                        }
                    }

                    // 3. 焦点记忆管理
                    if state.tag_focus_history.get(&win_tag) == Some(&id) {
                        state.tag_focus_history.remove(&win_tag);
                        // 如果同标签下还有别的窗口，把焦点记在别人身上
                        if let Some(other) = state
                            .windows
                            .iter()
                            .find(|w| w.id != id && (w.tags & win_tag) != 0)
                        {
                            state.tag_focus_history.insert(win_tag, other.id.clone());
                        }
                    }
                }

                // 4. 从全局扁平列表中移除
                state.windows.retain(|w| w.id != id);
                state.last_geometry.remove(&id);

                // 此时不需要做任何事，River 随后会自动发 ManageStart
            }
            WinEvent::AppId { app_id } => {
                let id = proxy.id();
                println!("-> 窗口 ID {:?} 获得 AppId: {:?}", id, app_id);

                // 1. 更新内存里的 app_id
                if let Some(w_info) = state.windows.iter_mut().find(|w| w.id == id) {
                    w_info.app_id = app_id.clone();
                }

                // 2. 过滤黑名单：如果是 fcitx5 相关窗口，直接无视，不参与平铺
                if let Some(ref id_str) = app_id {
                    if id_str.contains("fcitx") {
                        println!("-> [跳过] 输入法窗口不参与平铺逻辑");
                        return;
                    }
                } else {
                    // 如果没有 app_id，为了系统稳定，我们也先不平铺它
                    return;
                }

                // 3. 执行平铺逻辑 (只有从未被平铺过的窗口才执行)
                // 检查该窗口是否已经存在于任何布局树中
                let already_tiled = state.layout_roots.values().any(|root| {
                    fn tree_contains(node: &LayoutNode, target: &ObjectId) -> bool {
                        match node {
                            LayoutNode::Window(w) => &w.id == target,
                            LayoutNode::Container {
                                left_child,
                                right_child,
                                ..
                            } => {
                                tree_contains(left_child, target)
                                    || tree_contains(right_child, target)
                            }
                        }
                    }
                    tree_contains(root, &id)
                });

                if !already_tiled {
                    let current_tag = state.focused_tags;
                    // 拿到该窗口的数据副本
                    let w_data = state.windows.iter().find(|w| w.id == id).cloned().unwrap();

                    if !state.layout_roots.contains_key(&current_tag) {
                        // 标签页的第一个窗口，设为根
                        state
                            .layout_roots
                            .insert(current_tag, LayoutNode::Window(w_data));
                    } else if let Some(mut root) = state.layout_roots.remove(&current_tag) {
                        // 执行 Cosmic 分裂
                        let target_id = state
                            .tag_focus_history
                            .get(&current_tag)
                            .cloned()
                            .unwrap_or_else(|| id.clone());

                        let split = if let Some(geo) = state.last_geometry.get(&target_id) {
                            if geo.w > geo.h {
                                SplitType::Vertical
                            } else {
                                SplitType::Horizontal
                            }
                        } else {
                            SplitType::Vertical
                        };

                        root.insert_at(&target_id, w_data, split);
                        state.layout_roots.insert(current_tag, root);
                    }

                    // 4. 关键：更新焦点记忆
                    // 只有在这里更新，才能确保焦点始终落在“有身份”的正经窗口上
                    state.focused_window = Some(id.clone());
                    state.tag_focus_history.insert(current_tag, id.clone());

                    if let Some(seat) = &state.main_seat {
                        seat.focus_window(proxy);
                    }
                }
            }
            _ => {}
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
        qh: &QueueHandle<Self>,
    ) {
        if let BindingEvent::Pressed = event {
            if let Some(kb) = state.key_bindings.iter().find(|b| b.obj.id() == proxy.id()) {
                state.perform_action(kb.action.clone());
            }

            // --- 核心重载逻辑 ---
            if state.needs_reload {
                println!("-> 执行快捷键热重载...");
                // 1. 销毁旧对象：告诉 River 别再监听这些按键了
                // drain(..) 会清空数组并返回里面的元素
                for kb in state.key_bindings.drain(..) {
                    kb.obj.destroy();
                }
                // 2. 创建新对象：根据新 config 重新注册
                self::binds::setup_keybindings(state, qh);
                // 3. 强制通知：由于新绑定的 enable() 必须在 manage 序列执行
                // 我们调用 manage_dirty() 强行让 River 发起一次 ManageStart
                if let Some(wm) = &state.river_wm {
                    wm.manage_dirty();
                }
                state.needs_reload = false;
                println!("-> 热重载完成！");
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
            // 1. 存入列表，等待后续验证身份
            state.keyboards.push(id.clone());

            // 2. 如果已经有缓存的键位图，什么都不做！
            // 等待 KbEvent::InputDevice 事件触发时再去应用
            if state.current_keymap.is_some() {
                return;
            }

            // 3. 只有第一次才执行生成逻辑 (生成 temp_file 等)
            if let Some(kb_cfg) = state
                .config
                .input
                .as_ref()
                .and_then(|i| i.keyboard.as_ref())
            {
                println!("-> 首次发现硬件，生成布局映射: {}...", kb_cfg.layout);

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
                    let _ = temp_file.write_all(keymap_str.as_bytes());

                    if let Some(mgr) = &state.xkb_config {
                        let river_keymap =
                            mgr.create_keymap(temp_file.as_fd(), KeymapFormat::TextV1, qh, ());
                        state.current_keymap = Some(river_keymap);
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

// --- A. 经理 Dispatch：负责发现“接口(Head)”和处理“报告完毕(Done)” ---
impl Dispatch<ZwlrOutputManagerV1, ()> for AppState {
    fn event(
        state: &mut Self,
        _proxy: &ZwlrOutputManagerV1,
        event: MgrEvent,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            MgrEvent::Head { head } => {
                // 发现新接口，先存起来，后续通过事件填内容
                state.heads.push(HeadInfo {
                    obj: head,
                    name: String::new(),
                    modes: Vec::new(),
                    current_mode: None,
                });
            }
            MgrEvent::Done { serial } => {
                println!(
                    "-> 显示器硬件报告完毕 (Serial: {})，开始匹配配置...",
                    serial
                );
                state.apply_output_configs(qh, serial);
            }
            _ => {}
        }
    }
    wayland_client::event_created_child!(AppState, ZwlrOutputManagerV1, [
        0 => (ZwlrOutputHeadV1, ())
    ]);
}

// --- B. 接口 Dispatch：负责收集名字、当前模式、支持的分辨率列表 ---
impl Dispatch<ZwlrOutputHeadV1, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &ZwlrOutputHeadV1,
        event: HeadEvent,
        _: &(),
        _: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let Some(head) = state.heads.iter_mut().find(|h| h.obj.id() == proxy.id()) {
            match event {
                HeadEvent::Name { name } => head.name = name,
                HeadEvent::CurrentMode { mode } => head.current_mode = Some(mode.id()),
                HeadEvent::Mode { mode } => {
                    head.modes.push(ModeInfo {
                        obj: mode,
                        width: 0,
                        height: 0,
                        refresh: 0,
                    });
                }
                _ => {}
            }
        }
    }
    wayland_client::event_created_child!(AppState, ZwlrOutputHeadV1, [
        3 => (ZwlrOutputModeV1, ())
    ]);
}

// --- C. 模式 Dispatch：负责记录分辨率和刷新率 ---
impl Dispatch<ZwlrOutputModeV1, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &ZwlrOutputModeV1,
        event: ModeEvent,
        _: &(),
        _: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // 找到属于哪个 Head，并存入其模式列表
        for head in &mut state.heads {
            if let Some(mode_info) = head.modes.iter_mut().find(|m| m.obj.id() == proxy.id()) {
                match event {
                    ModeEvent::Size { width, height } => {
                        mode_info.width = width;
                        mode_info.height = height;
                    }
                    ModeEvent::Refresh { refresh } => {
                        mode_info.refresh = refresh;
                    }
                    _ => {}
                }
                return;
            }
        }

        // 如果是第一次见到的模式对象，根据 XML 协议，mode 对象是在 Head 接口下创建的
        // 这里简化处理：因为 generate_client_code 已经帮我们处理了层级
        // 我们在 HeadEvent::Mode 里预创建 ModeInfo 会更优雅。
    }
}

// 配置事务的 Dispatch (处理成功/失败的回执)
impl Dispatch<ZwlrOutputConfigurationV1, ()> for AppState {
    fn event(
        _: &mut Self,
        _proxy: &ZwlrOutputConfigurationV1,
        event: ConfigResEvent,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ConfigResEvent::Succeeded => println!("-> [成功] 显示器配置已生效"),
            ConfigResEvent::Failed => eprintln!("-> [失败] River 拒绝了该显示器配置"),
            ConfigResEvent::Cancelled => eprintln!("-> [取消] 配置由于硬件热插拔已失效，请重试"),
        }
    }
}

impl Dispatch<RiverInputDeviceV1, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &RiverInputDeviceV1,
        event: InputDeviceEvent,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            InputDeviceEvent::Name { name } => {
                println!("-> 发现输入设备名称: ID {:?} = {}", proxy.id(), name);
                state.device_names.insert(proxy.id(), name);
            }
            _ => {}
        }
    }
}
// src/wm/mod.rs

impl Dispatch<RiverXkbKeyboardV1, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &RiverXkbKeyboardV1,
        event: KbEvent,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            KbEvent::InputDevice { device } => {
                // 1. 查名字
                let name = state
                    .device_names
                    .get(&device.id())
                    .cloned()
                    .unwrap_or_default();
                let name_lower = name.to_lowercase();

                // 2. 黑名单过滤：如果是虚拟键盘，直接忽略
                if name_lower.contains("fcitx") || name_lower.contains("virtual") {
                    println!("-> [忽略] 检测到虚拟键盘: {} (ID: {:?})", name, proxy.id());
                    // 甚至可以从 state.keyboards 里把它删掉，免得以后误伤
                    state.keyboards.retain(|k| k.id() != proxy.id());
                    return;
                }

                println!(
                    "-> [配置] 检测到物理键盘: {} (ID: {:?})，应用布局...",
                    name,
                    proxy.id()
                );

                // 3. 只有通过检查的，才应用布局
                if let Some(keymap) = &state.current_keymap {
                    proxy.set_keymap(keymap);
                }
            }

            KbEvent::Removed => {
                // 清理逻辑
                let id = proxy.id();
                state.keyboards.retain(|k| k.id() != id);
            }
            _ => {}
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
// src/wm/mod.rs
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

// 配置子表单的 Dispatch (通常不需要处理事件)
impl Dispatch<ZwlrOutputConfigurationHeadV1, ()> for AppState {
    fn event(
        _: &mut Self,
        _: &ZwlrOutputConfigurationHeadV1,
        _: crate::protocol::wlr_output_management::zwlr_output_configuration_head_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
