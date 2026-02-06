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
use crate::protocol::river_wm::river_output_v1::Event as OutEvent;
use crate::protocol::river_wm::river_seat_v1::Event as SeatEvent;
use crate::protocol::river_wm::{
    river_node_v1::RiverNodeV1, river_output_v1::RiverOutputV1, river_seat_v1::RiverSeatV1,
    river_window_manager_v1::RiverWindowManagerV1, river_window_v1::RiverWindowV1,
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
use log::{error, info, warn};
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
    pub actions: Vec<Action>,
}

#[derive(Debug, Clone)]
pub struct OutputData {
    pub width: i32,
    pub height: i32,
    pub usable_area: Geometry,
    pub full_area: Geometry,
    pub ls_output: Option<RiverLayerShellOutputV1>,
    pub tags: u32,
    pub base_tag: u32,
}

#[derive(Clone)]
pub struct WindowData {
    pub id: ObjectId,
    pub window: RiverWindowV1,
    pub node: Option<RiverNodeV1>,
    pub tags: u32,
    pub app_id: Option<String>,
    pub output: Option<String>,
    pub is_fullscreen: bool,
    pub layout_retry_count: u8,
    pub last_proposed_w: i32,
    pub last_proposed_h: i32,
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

pub struct RiverOutputInfo {
    pub obj: crate::protocol::river_wm::river_output_v1::RiverOutputV1,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

pub struct AppState {
    pub config: crate::config::Config,
    pub needs_reload: bool,
    pub river_wm: Option<RiverWindowManagerV1>,
    pub windows: Vec<WindowData>,
    pub outputs: HashMap<String, OutputData>,
    pub main_seat: Option<RiverSeatV1>,
    pub current_width: i32,
    pub current_height: i32,
    pub tag_focus_history: HashMap<(String, u32), ObjectId>,
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
    pub last_output_serial: u32,
    pub layout_roots: HashMap<(String, u32), LayoutNode>,
    pub focused_output: Option<String>,
    pub pending_pointer_warp: Option<(i32, i32)>,
    pub last_sent_json: String,
    pub anonymous_ls_outputs: Vec<RiverLayerShellOutputV1>,
    pub wl_name_to_monitor_name: HashMap<u32, String>,
    pub active_river_outputs: Vec<RiverOutputInfo>,
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
                    info!(
                        "[ID:{}] Discovered Display Manager (wlr-output-management)",
                        name
                    );
                    let manager = proxy.bind::<ZwlrOutputManagerV1, _, _>(name, 4, qh, ());
                    state.output_manager = Some(manager);
                }
                "river_layer_shell_v1" => {
                    info!("[ID:{}] Binding: Hierarchical Surface Manager (waybar/swww permission is enabled)", name);
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
                info!("-> Found new input device (Seat)");
                state.main_seat = Some(id.clone());
                // 2. 清理点：不再手动注册默认键，而是统一调用 binds 模块
                // 它会自动处理 TOML 配置或使用保底默认值
                self::binds::setup_keybindings(state, qh);
            }
            WmEvent::Window { id } => {
                // 默认分配到当前活跃显示器，如果没有活跃显示器，就暂时不分配
                let current_out = state.focused_output.clone();
                // 仅预登记，不执行切割，也不分配焦点
                info!(
                    "-> Found new window, waiting to be assigned AppId: {:?}",
                    id.id()
                );
                state.windows.push(WindowData {
                    id: id.id(),
                    window: id.clone(),
                    node: None,
                    tags: state.focused_tags, // 预分配到当前标签
                    app_id: None,
                    output: current_out,
                    is_fullscreen: false,
                    layout_retry_count: 0,
                    last_proposed_w: 0,
                    last_proposed_h: 0,
                });
            }
            WmEvent::ManageStart => {
                // 1. 基础工作：处理 IPC 和广播状态
                state.handle_ipc_connections();
                state.broadcast_status();

                // --- 新增：物理焦点生效逻辑 ---
                if let Some((x, y)) = state.pending_pointer_warp.take() {
                    if let Some(seat) = &state.main_seat {
                        info!("-> [Physics Focus] Executing mouse teleport within management sequence: {},{}", x, y);
                        seat.pointer_warp(x, y);
                    }
                }

                // 同步当前活跃显示器的标签给全局（供状态栏/逻辑判定参考）
                if let Some(out_id) = &state.focused_output {
                    if let Some(out_data) = state.outputs.get(out_id) {
                        // 打印日志，看看当前活跃屏幕是谁，它认为自己在看哪个 Tag
                        info!(
                            "-> [Rendering Check] Active screen: {:?}, Tag mask: {:b}",
                            out_id, out_data.tags
                        );
                        state.focused_tags = out_data.tags;
                    }
                }

                // 2. 智能焦点恢复
                // 逻辑：如果当前没有焦点窗口，或者焦点窗口由于所在 Tag 被隐藏，则尝试恢复
                let needs_restore = match &state.focused_window {
                    Some(f_id) => !state
                        .windows
                        .iter()
                        .any(|w| &w.id == f_id && (w.tags & state.focused_tags) != 0),
                    None => true,
                };

                if needs_restore {
                    // 优先看当前活跃显示器下有没有历史焦点
                    let history_id = state
                        .focused_output
                        .as_ref()
                        .and_then(|out_id| {
                            state
                                .tag_focus_history
                                .get(&(out_id.clone(), state.focused_tags))
                        })
                        .cloned();
                    if let Some(hid) = history_id {
                        state.focused_window = Some(hid);
                    } else {
                        // 实在不行，抓取当前活跃标签页下的任意一个可见窗口
                        state.focused_window = state
                            .windows
                            .iter()
                            .find(|w| (w.tags & state.focused_tags) != 0)
                            .map(|w| w.id.clone());
                    }
                }
                // --- 遍历所有窗口，根据 is_fullscreen 标志，强制同步 Wayland 状态 ---
                for w in &state.windows {
                    if w.is_fullscreen {
                        // 坐标匹配法
                        // 1. 获取窗口当前所在显示器的“身份证”名字
                        let mut target_river_output = None;
                        if let Some(out_name) = &w.output {
                            // 2. 查阅身份证，获取其物理坐标 (full_area.x, full_area.y)
                            // 注意：必须用 full_area (显示器原点)，不能用 usable_area (可能被 Waybar 挤占了)
                            if let Some(out_data) = state.outputs.get(out_name) {
                                let target_x = out_data.full_area.x;
                                let target_y = out_data.full_area.y;

                                // 3. 在“活人箱子”里寻找坐标完全一致的 RiverOutputV1 对象
                                if let Some(info) = state
                                    .active_river_outputs
                                    .iter()
                                    .find(|i| i.x == target_x && i.y == target_y)
                                {
                                    target_river_output = Some(info.obj.clone());
                                }
                            }
                        }

                        // 4. 如果找到了真身，执行全屏；如果没找到（比如屏幕刚拔掉），则不做操作防崩
                        if let Some(out_obj) = target_river_output {
                            w.window.fullscreen(&out_obj);
                            w.window.inform_fullscreen();
                        }
                    } else {
                        // 如果不是全屏，强制退出全屏（确保状态同步）
                        w.window.exit_fullscreen();
                        w.window.inform_not_fullscreen();
                    }
                }
                // 3. 显隐控制：遍历所有窗口
                for w_data in &state.windows {
                    if let Some(ref app_id) = w_data.app_id {
                        if app_id.contains("fcitx") {
                            continue;
                        }

                        let is_visible = if let Some(win_out_id) = &w_data.output {
                            if let Some(out_data) = state.outputs.get(win_out_id) {
                                // 窗口所属显示器正在看这个窗口的标签
                                (w_data.tags & out_data.tags) != 0
                            } else {
                                false
                            }
                        } else {
                            false
                        };

                        if is_visible {
                            w_data.window.show();
                        } else {
                            w_data.window.hide();
                        }
                    }
                }

                // 4. 焦点确认：告诉 River 真正把键盘给谁
                if let Some(f_id) = &state.focused_window {
                    if let Some(w_data) = state.windows.iter().find(|w| &w.id == f_id) {
                        if (w_data.tags & state.focused_tags) != 0 {
                            if let Some(seat) = &state.main_seat {
                                // 如果处于重试状态，我们玩个把戏：奇数次清除焦点，偶数次给焦点
                                // 这模拟了用户的“切换焦点”操作，能有效治愈 Electron/mpv 的尺寸冻结症
                                if w_data.layout_retry_count > 0 {
                                    if w_data.layout_retry_count % 2 != 0 {
                                        // info!("奇数次：假装失去焦点");
                                        // 奇数次：假装失去焦点
                                        seat.clear_focus();
                                    } else {
                                        // info!("偶数次：重新获得焦点");
                                        // 偶数次：重新获得焦点
                                        seat.focus_window(&w_data.window);
                                    }
                                } else {
                                    // 正常情况：直接给焦点
                                    seat.focus_window(&w_data.window);
                                }
                            }
                        }
                    }
                }

                // 5. 布局计算：遍历所有显示器，各算各的树
                state.last_geometry.clear(); // 清空旧的几何记录，准备重新记录

                // 这里不再用 .next()，而是迭代所有的 outputs
                for (out_id, out_data) in &state.outputs {
                    let tree_key = (out_id.clone(), out_data.tags);

                    if let Some(root) = state.layout_roots.get(&tree_key) {
                        let mut results = Vec::new();
                        calculate_layout(root, out_data.usable_area, &mut results);

                        // --- A. 获取基础配置 ---
                        let win_cfg = state.config.window.as_ref();
                        let border_cfg = win_cfg
                            .and_then(|c| c.active.as_ref())
                            .and_then(|a| a.border.as_ref());
                        let config_width = border_cfg.map(|b| b.width).unwrap_or(0);
                        let config_color =
                            border_cfg.map(|b| b.color.as_str()).unwrap_or("#ffffff");
                        let (br, bg, bb, ba) = Self::parse_color(config_color);
                        let is_smart = win_cfg.map(|c| c.smart_borders).unwrap_or(false);
                        let window_count = results.len();

                        for (window, geom) in results {
                            // 我们需要用 iter_mut，因为要更新 last_proposed 字段
                            if let Some(w_data) =
                                state.windows.iter_mut().find(|w| w.id == window.id())
                            {
                                let is_focused =
                                    state.focused_window.as_ref() == Some(&window.id());
                                let current_win_width = if is_focused {
                                    if is_smart && window_count <= 1 {
                                        0
                                    } else {
                                        config_width
                                    }
                                } else {
                                    0
                                };

                                // 计算收缩后的内容尺寸
                                let shrunk_w = (geom.w - (current_win_width as i32 * 2)).max(1);
                                let shrunk_h = (geom.h - (current_win_width as i32 * 2)).max(1);

                                // --- 存入收缩后的几何信息，用于 Dimensions 比对 ---
                                state.last_geometry.insert(
                                    window.id(),
                                    crate::wm::layout::Geometry {
                                        x: geom.x,
                                        y: geom.y,
                                        w: shrunk_w,
                                        h: shrunk_h,
                                    },
                                );

                                // 设置边框
                                window.set_borders(
                                    crate::protocol::river_wm::river_window_v1::Edges::all(),
                                    current_win_width as i32,
                                    br,
                                    bg,
                                    bb,
                                    ba,
                                );

                                // --- 只有尺寸变了才发送建议，防止应用被 Resize 淹没 ---
                                if w_data.last_proposed_w != shrunk_w
                                    || w_data.last_proposed_h != shrunk_h
                                {
                                    window.propose_dimensions(shrunk_w, shrunk_h);
                                    w_data.last_proposed_w = shrunk_w;
                                    w_data.last_proposed_h = shrunk_h;
                                }

                                window.set_tiled(
                                    crate::protocol::river_wm::river_window_v1::Edges::all(),
                                );
                            }
                        }
                    }
                }
                // 6. 后续清理：Waybar 激活与快捷键使能
                if let Some(focused_name) = &state.focused_output {
                    if let Some(out_data) = state.outputs.get(focused_name) {
                        if let Some(ls_out) = &out_data.ls_output {
                            // 告诉 River：接下来任何没指定位置的层级窗口（如 fuzzel），请放到这个屏幕
                            ls_out.set_default();
                        }
                    }
                }
                for kb in &state.key_bindings {
                    kb.obj.enable();
                }
                proxy.manage_finish();
            }
            WmEvent::RenderStart => {
                // 这里也要拿到配置
                let win_cfg = state.config.window.as_ref();
                let border_cfg = win_cfg
                    .and_then(|c| c.active.as_ref())
                    .and_then(|a| a.border.as_ref());
                let config_width = border_cfg.map(|b| b.width).unwrap_or(0);
                let is_smart = win_cfg.map(|c| c.smart_borders).unwrap_or(false);

                for (out_name, out_data) in &state.outputs {
                    let tree_key = (out_name.clone(), out_data.tags);
                    if let Some(root) = state.layout_roots.get(&tree_key) {
                        let mut results = Vec::new();
                        calculate_layout(root, out_data.usable_area, &mut results);
                        let window_count = results.len();

                        for (window, geom) in results {
                            if let Some(w_data) =
                                state.windows.iter_mut().find(|w| w.id == window.id())
                            {
                                if w_data.node.is_none() {
                                    w_data.node = Some(window.get_node(qh, ()));
                                }
                                if let Some(node) = &w_data.node {
                                    // --- 核心：重新计算当前窗口的偏移量 ---
                                    let is_focused =
                                        state.focused_window.as_ref() == Some(&window.id());
                                    let current_win_width = if is_focused {
                                        if is_smart && window_count <= 1 {
                                            0
                                        } else {
                                            config_width
                                        }
                                    } else {
                                        0
                                    };

                                    // 物理位置 x, y 也要加上偏移，才能在格子里居中
                                    node.set_position(
                                        geom.x + current_win_width as i32,
                                        geom.y + current_win_width as i32,
                                    );
                                    node.place_top();
                                }
                            }
                        }
                    }
                }
                proxy.render_finish();
            }

            WmEvent::Output { id } => {
                info!("-> Found new physical output interface: {:?}", id.id());
                // 先初始化为 0，等待后续 Dimensions/Position 事件更新
                state.active_river_outputs.push(RiverOutputInfo {
                    obj: id.clone(),
                    x: 0,
                    y: 0,
                    w: 0,
                    h: 0,
                });
                // --- 绑定 LayerShell 输出对象 ---
                if let Some(ls_mgr) = &state.layer_shell_manager {
                    // 创建监听对象并放入暂存区
                    let ls_out = ls_mgr.get_output(&id, qh, ());
                    state.anonymous_ls_outputs.push(ls_out);
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
// [impl Dispatch<RiverOutputV1, ()> for AppState]
impl Dispatch<RiverOutputV1, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &RiverOutputV1,
        event: OutEvent,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // 1. 找到列表中对应的那个 info
        if let Some(info) = state
            .active_river_outputs
            .iter_mut()
            .find(|i| i.obj.id() == proxy.id())
        {
            match event {
                OutEvent::Dimensions { width, height } => {
                    info!(
                        "-> [Hardware] Output {:?} resolution: {}x{}",
                        proxy.id(),
                        width,
                        height
                    );
                    info.w = width;
                    info.h = height;
                    if let Some(wm) = &state.river_wm {
                        wm.manage_dirty();
                    }
                }
                OutEvent::Position { x, y } => {
                    // --- 记录坐标，用于匹配名字 ---
                    info.x = x;
                    info.y = y;
                }
                OutEvent::Removed => {
                    proxy.destroy();
                }
                _ => {}
            }
        }

        // 单独处理移除逻辑，避免 borrow checker 问题
        if let OutEvent::Removed = event {
            state
                .active_river_outputs
                .retain(|i| i.obj.id() != proxy.id());
        }
    }
}

// --- 4. 监听 RiverSeatV1 (点击聚焦) ---
impl Dispatch<RiverSeatV1, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &RiverSeatV1,
        event: SeatEvent,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            // --- 新增：处理鼠标坐标变化，实现焦点随鼠标跨屏 ---
            SeatEvent::PointerPosition { x, y } => {
                let mut found_name = None;
                // 遍历所有显示器，检查坐标落在谁的领地里
                for (name, data) in &state.outputs {
                    let g = data.usable_area;
                    // 判定坐标 (x, y) 是否在显示器的逻辑矩形内
                    if x >= g.x && x < g.x + g.w && y >= g.y && y < g.y + g.h {
                        found_name = Some(name.clone());
                        break;
                    }
                }
                if let Some(name) = found_name {
                    // 只有当显示器真的变了，才执行切换，避免日志刷屏
                    if state.focused_output.as_ref() != Some(&name) {
                        info!("-> [Focus] The mouse crosses the physical boundary and automatically locks the monitor: {}", name);
                        state.focused_output = Some(name);
                        if let Some(wm) = &state.river_wm {
                            wm.manage_dirty();
                        }
                    }
                }
            }

            SeatEvent::WindowInteraction { window } => {
                let id = window.id();
                info!("-> Mouse click window: {:?}", id);
                state.focused_window = Some(id.clone());
                if let Some(w_info) = state.windows.iter().find(|w| w.id == id) {
                    state.focused_window = Some(id.clone());
                    // 同步更新当前活跃显示器
                    if let Some(out_id) = &w_info.output {
                        state.focused_output = Some(out_id.clone());
                        state
                            .tag_focus_history
                            .insert((out_id.clone(), w_info.tags), id.clone());
                    }
                }
                proxy.focus_window(&window);
            }
            _ => (),
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
                if let Some(w_info) = state.windows.iter().find(|w| w.id == id) {
                    let win_tag = w_info.tags;
                    if let Some(out_id) = &w_info.output {
                        let tree_key = (out_id.clone(), win_tag); // Key 1: 布局树

                        // 从树中移除
                        if let Some(root) = state.layout_roots.remove(&tree_key) {
                            if let Some(new_root) = LayoutNode::remove_at(root, &id) {
                                state.layout_roots.insert(tree_key.clone(), new_root);
                            }
                        }

                        // 焦点记忆管理
                        let history_key = (out_id.clone(), win_tag); // Key 2: 焦点历史

                        // 使用 Key 2: history_key 查找
                        if state.tag_focus_history.get(&history_key) == Some(&id) {
                            state.tag_focus_history.remove(&history_key);

                            // 找接班人：必须是同一个显示器 (out_id) 且同一个标签
                            if let Some(other) = state.windows.iter().find(|w| {
                                w.id != id
                                    && (w.tags & win_tag) != 0
                                    && w.output.as_ref() == Some(out_id)
                            }) {
                                // 使用元组键 tree_key
                                state.tag_focus_history.insert(tree_key, other.id.clone());
                            }
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
                info!("-> Window ID {:?} gets AppId: {:?}", id, app_id);

                // 1. 更新内存里的 app_id 并确定归属显示器
                let mut out_id_to_use = None;
                if let Some(w_info) = state.windows.iter_mut().find(|w| w.id == id) {
                    w_info.app_id = app_id.clone();

                    // 如果窗口还没分配显示器，就给它当前活跃的显示器
                    if w_info.output.is_none() {
                        w_info.output = state
                            .focused_output
                            .clone()
                            .or_else(|| state.outputs.keys().next().cloned());
                    }
                    out_id_to_use = w_info.output.clone();
                }

                // 2. 过滤黑名单：fcitx 或没有有效显示器则跳过
                if let Some(ref id_str) = app_id {
                    if id_str.contains("fcitx") {
                        return;
                    }
                } else {
                    // 如果没有 app_id，为了系统稳定，我们也先不平铺它
                    return;
                }

                let out_id = match out_id_to_use {
                    Some(o) => o,
                    None => return, // 还没准备好显示器，先不平铺
                };

                // 3. 执行平铺逻辑
                // 检查窗口是否已在任何一棵树里（防止重复插入）
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
                    let w_data = state.windows.iter().find(|w| w.id == id).cloned().unwrap();

                    // 构造元组键：(显示器, 标签)
                    let tree_key = (out_id.clone(), current_tag);

                    if !state.layout_roots.contains_key(&tree_key) {
                        state
                            .layout_roots
                            .insert(tree_key, LayoutNode::Window(w_data));
                    } else if let Some(mut root) = state.layout_roots.remove(&tree_key) {
                        // 找到该显示器/标签下的焦点历史，决定切分位置
                        let target_id = state
                            .tag_focus_history
                            .get(&tree_key)
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
                        state.layout_roots.insert(tree_key, root);
                    }

                    // 4. 更新全局状态
                    state.focused_window = Some(id.clone());
                    state.focused_output = Some(out_id.clone());
                    state
                        .tag_focus_history
                        .insert((out_id, current_tag), id.clone());

                    if let Some(seat) = &state.main_seat {
                        seat.focus_window(proxy);
                    }
                    if let Some(wm) = &state.river_wm {
                        wm.manage_dirty();
                    }
                }
            }
            // --- 处理全屏请求 ---
            WinEvent::FullscreenRequested { output: _ } => {
                // 注：虽然应用可能建议了 output，但为了稳健，我们强制它在当前所在的显示器全屏
                // 这样可以避免应用瞎指挥导致窗口跳到别的屏幕去
                let id = proxy.id();
                info!("-> [Event] Window {:?} requested Fullscreen (F11)", id);

                if let Some(w) = state.windows.iter_mut().find(|w| w.id == id) {
                    // 1. 只更新意图状态
                    w.is_fullscreen = true;
                    // 2. 请求调度
                    if let Some(wm) = &state.river_wm {
                        wm.manage_dirty();
                    }
                }
            }
            // --- 处理退出全屏请求 ---
            WinEvent::ExitFullscreenRequested => {
                let id = proxy.id();
                info!("-> [Event] Window {:?} requested Exit Fullscreen", id);

                if let Some(w) = state.windows.iter_mut().find(|w| w.id == id) {
                    w.is_fullscreen = false;
                    if let Some(wm) = &state.river_wm {
                        wm.manage_dirty();
                    }
                }
            }
            // --- Dimensions 处理逻辑 ---
            WinEvent::Dimensions { width, height } => {
                if let Some(w_idx) = state.windows.iter().position(|w| w.id == proxy.id()) {
                    let w = &mut state.windows[w_idx];

                    if !w.is_fullscreen {
                        if let Some(geo) = state.last_geometry.get(&proxy.id()) {
                            let dw = (width as i32 - geo.w).abs();
                            let dh = (height as i32 - geo.h).abs();

                            // 误差检测
                            if dw > 2 || dh > 2 {
                                // --- 修改点 1: 增加重试次数到 50 ---
                                if w.layout_retry_count < 50 {
                                    info!(
                                        "-> Window {:?} size mismatch (Got {}x{}, Expected {}x{}), forcing relayout (Retry {}/50)...",
                                        proxy.id(), width, height, geo.w, geo.h, w.layout_retry_count + 1
                                    );
                                    w.layout_retry_count += 1;

                                    if let Some(wm) = &state.river_wm {
                                        wm.manage_dirty();
                                    }
                                } else {
                                    // 只有到了 50 次（大约持续半秒到一秒的疯狂抗拒）才放弃
                                    if w.layout_retry_count == 50 {
                                        warn!("-> Window {:?} refuses to accept layout geometry, giving up enforcement.", proxy.id());
                                        w.layout_retry_count += 1;
                                    }
                                }
                            } else {
                                // 尺寸符合预期，重置计数器
                                w.layout_retry_count = 0;
                            }
                        }
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
            // 先查找并克隆动作列表，立即结束对 state 的不可变借用
            let actions_to_run = state
                .key_bindings
                .iter()
                .find(|b| b.obj.id() == proxy.id())
                .map(|b| b.actions.clone());

            // 现在 state 已经“自由”了，我们可以安全地调用 perform_action(&mut self)
            if let Some(actions) = actions_to_run {
                for action in actions {
                    state.perform_action(action.clone());

                    if let Action::ReloadConfiguration = action {
                        let serial = state.last_output_serial;
                        state.apply_output_configs(qh, serial);
                    }
                }
            }

            // --- 核心重载逻辑 ---
            if state.needs_reload {
                info!("-> Perform shortcut hot reload...");
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
                info!("-> Hot reload completed!");
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
                info!(
                    "-> Discover hardware for the first time and generate layout mapping: {}...",
                    kb_cfg.layout
                );

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

// src/wm/mod.rs

impl Dispatch<RiverLayerShellOutputV1, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &RiverLayerShellOutputV1,
        event: LayerOutEvent,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            // src/wm/mod.rs -> LayerOutEvent
            LayerOutEvent::NonExclusiveArea {
                x,
                y,
                width,
                height,
            } => {
                // 计算该 Bar 的中心点坐标
                let bar_cx = x + (width / 2);
                let bar_cy = y + (height / 2);

                let mut matched_name = None;
                for (name, out_data) in &state.outputs {
                    let g = out_data.full_area;
                    // 严格的中心点包含判定，不再需要 +/- 10 容错
                    if g.w > 0
                        && bar_cx >= g.x
                        && bar_cx < g.x + g.w
                        && bar_cy >= g.y
                        && bar_cy < g.y + g.h
                    {
                        matched_name = Some(name.clone());
                        break;
                    }
                }

                if let Some(name) = matched_name {
                    if let Some(out_data) = state.outputs.get_mut(&name) {
                        info!(
                            "-> [Bar space] Display {} reserved space: {}x{} @ {},{}",
                            name, width, height, x, y
                        );
                        out_data.usable_area = Geometry {
                            x,
                            y,
                            w: width,
                            h: height,
                        };
                        out_data.ls_output = Some(proxy.clone());
                        if let Some(wm) = &state.river_wm {
                            wm.manage_dirty();
                        }
                    }
                } else {
                    warn!(
                        "-> [Bar] Reservation request {}x{} @ {},{} received but no display matched (probably not configured yet)",
                        width, height, x, y
                    );
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
                state.last_output_serial = serial; // 记住这个号，以后热重载要用
                info!("-> Monitor hardware report completed (Serial: {})", serial);
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
                HeadEvent::Name { name } => {
                    head.name = name.clone();
                    // 这里原本的 entry().or_insert() 逻辑保持不变
                    state.outputs.entry(name.clone()).or_insert(OutputData {
                        width: 0,
                        height: 0,
                        usable_area: Geometry {
                            x: 0,
                            y: 0,
                            w: 0,
                            h: 0,
                        },
                        full_area: Geometry {
                            x: 0,
                            y: 0,
                            w: 0,
                            h: 0,
                        }, // 初始化
                        ls_output: None,
                        tags: 1,
                        base_tag: 1,
                    });
                    if state.focused_output.is_none() {
                        state.focused_output = Some(name);
                    }
                }
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
    }
}

// 配置事务的 Dispatch (处理成功/失败的回执)
impl Dispatch<ZwlrOutputConfigurationV1, ()> for AppState {
    fn event(
        state: &mut Self,
        _proxy: &ZwlrOutputConfigurationV1,
        event: ConfigResEvent,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ConfigResEvent::Succeeded => {
                info!("-> [Success] The display configuration has taken effect and the layout is being refreshed...");
                // 强行触发一次 ManageStart，让 BSP 树重新计算
                if let Some(wm) = &state.river_wm {
                    wm.manage_dirty();
                }
            }
            ConfigResEvent::Failed => error!("-> [FAILED] River rejected this monitor configuration"),
            ConfigResEvent::Cancelled => error!("-> [Cancel] The configuration has expired due to hardware hot swap, please try again."),
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
                info!("-> Found input device name: ID {:?} = {}", proxy.id(), name);
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
                    info!(
                        "-> [Ignore] Virtual keyboard detected: {} (ID: {:?})",
                        name,
                        proxy.id()
                    );
                    // 甚至可以从 state.keyboards 里把它删掉，免得以后误伤
                    state.keyboards.retain(|k| k.id() != proxy.id());
                    return;
                }

                info!(
                    "-> [Configuration] Physical keyboard detected: {} (ID: {:?}), applying layout...",
                    name,
                    proxy.id()
                );

                // 3. 只有通过检查的，才应用布局
                if let Some(keymap) = &state.current_keymap {
                    proxy.set_keymap(keymap);
                }
                // --- 应用 Numlock 设置 ---
                if let Some(kb_cfg) = state
                    .config
                    .input
                    .as_ref()
                    .and_then(|i| i.keyboard.as_ref())
                {
                    if let Some(nl) = &kb_cfg.numlock {
                        if nl == "true" {
                            proxy.numlock_enable();
                            info!("-> [Keyboard] {} Numlock is on", name);
                        } else if nl == "false" {
                            proxy.numlock_disable();
                            info!("-> [Keyboard] {} Numlock turned off", name);
                        }
                    }
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
