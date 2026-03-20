use crate::protocol::river_wm::river_seat_v1::{Modifiers, RiverSeatV1};
use crate::protocol::river_xkb::river_xkb_bindings_v1::RiverXkbBindingsV1;
use crate::wm::{actions::Action, AppState, BindingMode, KeyBinding, PointerBinding};
use tracing::{error, info, warn};
use wayland_client::QueueHandle;
use xkbcommon::xkb;

/// 将 "alt_shift" 拆分为位掩码
fn parse_mod_group(group: &str) -> Modifiers {
    if group.to_lowercase() == "none" {
        return Modifiers::empty();
    }
    let parts: Vec<&str> = group.split(|c| c == '_' || c == '+' || c == '-').collect();
    let mut mask = Modifiers::empty();
    for p in parts {
        match p.to_lowercase().trim() {
            "shift" => mask |= Modifiers::Shift,
            "ctrl" | "control" => mask |= Modifiers::Ctrl,
            "alt" | "mod1" => mask |= Modifiers::Mod1,
            "super" | "mod4" | "logo" => mask |= Modifiers::Mod4,
            _ => warn!("警告：未知的修饰符标签 {}", p),
        }
    }
    mask
}

/// 辅助函数：真正向 River 注册绑定并存入 state
fn commit_binding(
    state: &mut AppState,
    mgr: &RiverXkbBindingsV1,
    seat: &RiverSeatV1,
    qh: &QueueHandle<AppState>,
    key_name: &str,
    mods: Modifiers,
    actions: Vec<Action>,
    mode: BindingMode,
) {
    // 1. 尝试按原样查找 (例如 "Return", "space", "BackSpace")
    let mut keysym = xkb::keysym_from_name(key_name, xkb::KEYSYM_NO_FLAGS);

    // 2. 如果没找到 (返回 KEY_NoSymbol)，尝试转为全小写再找一次
    // 注意这里使用 .raw() 和 xkb::keysyms::KEY_NoSymbol
    if keysym.raw() == xkb::keysyms::KEY_NoSymbol {
        keysym = xkb::keysym_from_name(&key_name.to_lowercase(), xkb::KEYSYM_NO_FLAGS);
    }

    // 3. 最终检查，如果还是找不到，则报错并跳过
    if keysym.raw() == xkb::keysyms::KEY_NoSymbol {
        error!(
            "-> [Shortcut key error] Unable to recognize the key name: '{}', please check whether the name in the TOML configuration is correct",
            key_name
        );
        return;
    }

    // 注册绑定
    let binding_obj = mgr.get_xkb_binding(seat, keysym.raw(), mods, qh, ());

    state.key_bindings.push(KeyBinding {
        obj: binding_obj,
        actions,
        mode,
    });
}

/// 核心递归解析函数：把 TOML 的嵌套结构变成 Vec<Action> 并注册
fn process_entry(
    state: &mut AppState,
    mgr: &RiverXkbBindingsV1,
    seat: &RiverSeatV1,
    qh: &QueueHandle<AppState>,
    key_or_mod: &str,
    current_mods: Modifiers,
    entry: &crate::config::KeyBindingEntry,
    mode: BindingMode,
) {
    let slot_id = format!("{}_{}", current_mods.bits(), key_or_mod);
    match entry {
        // 情况 1：单个动作
        crate::config::KeyBindingEntry::Action(cfg) => {
            let actions = vec![Action::from_config(
                &cfg.action,
                &cfg.args,
                &cfg.cmd,
                &cfg.unit,
                &slot_id,
            )];
            commit_binding(
                state,
                mgr,
                seat,
                qh,
                key_or_mod,
                current_mods,
                actions,
                mode,
            );
        }
        // 情况 2：动作列表 [ {action=...}, {action=...} ]
        crate::config::KeyBindingEntry::List(cfgs) => {
            let actions = cfgs
                .iter()
                .map(|cfg| {
                    Action::from_config(&cfg.action, &cfg.args, &cfg.cmd, &cfg.unit, &slot_id)
                })
                .collect();
            commit_binding(
                state,
                mgr,
                seat,
                qh,
                key_or_mod,
                current_mods,
                actions,
                mode,
            );
        }
        // 情况 3：修饰符分组 [keybindings.alt]
        crate::config::KeyBindingEntry::Group(sub_map) => {
            // 解析这一层增加的修饰符
            let extra_mods = parse_mod_group(key_or_mod);
            let combined_mods = current_mods | extra_mods;

            // 递归处理子项
            for (sub_key, sub_entry) in sub_map {
                process_entry(
                    state,
                    mgr,
                    seat,
                    qh,
                    sub_key,
                    combined_mods,
                    sub_entry,
                    mode,
                );
            }
        }
    }
}

/// 辅助：将鼠标按键名称转为 Linux input event code
fn parse_pointer_button(name: &str) -> Option<u32> {
    match name.to_uppercase().as_str() {
        "BTN_LEFT" | "272" => Some(272),
        "BTN_RIGHT" | "273" => Some(273),
        "BTN_MIDDLE" | "274" => Some(274),
        "BTN_SIDE" | "275" => Some(275),
        "BTN_EXTRA" | "276" => Some(276),
        _ => name.parse::<u32>().ok(), // 允许用户直接写数字，例如 "272"
    }
}

/// 辅助：真正向 River 注册鼠标绑定并存入 state
fn commit_pointer_binding(
    state: &mut AppState,
    _mgr: &RiverXkbBindingsV1,
    seat: &RiverSeatV1,
    qh: &QueueHandle<AppState>,
    button_name: &str,
    mods: Modifiers,
    actions: Vec<Action>,
    mode: BindingMode,
) {
    let button_code = match parse_pointer_button(button_name) {
        Some(code) => code,
        None => {
            error!(
                "-> [Pointer Binding Error] Unknown button name: {}",
                button_name
            );
            return;
        }
    };

    // --- 【核心修复】 ---
    // 调用 River 协议注册鼠标指针事件
    // 注意：get_pointer_binding 是 RiverSeatV1 的方法，不是 XkbBindings 的方法！
    let binding_obj = seat.get_pointer_binding(button_code, mods, qh, ());

    state.pointer_bindings.push(PointerBinding {
        obj: binding_obj,
        actions,
        mode,
    });
}

/// 核心：递归解析鼠标配置的 TOML 结构
fn process_pointer_entry(
    state: &mut AppState,
    mgr: &RiverXkbBindingsV1,
    seat: &RiverSeatV1,
    qh: &QueueHandle<AppState>,
    btn_or_mod: &str,
    current_mods: Modifiers,
    entry: &crate::config::KeyBindingEntry,
    mode: BindingMode,
) {
    let slot_id = format!("ptr_{}_{}", current_mods.bits(), btn_or_mod);
    match entry {
        crate::config::KeyBindingEntry::Action(cfg) => {
            let actions = vec![Action::from_config(
                &cfg.action,
                &cfg.args,
                &cfg.cmd,
                &cfg.unit,
                &slot_id,
            )];
            commit_pointer_binding(
                state,
                mgr,
                seat,
                qh,
                btn_or_mod,
                current_mods,
                actions,
                mode,
            );
        }
        crate::config::KeyBindingEntry::List(cfgs) => {
            let actions = cfgs
                .iter()
                .map(|cfg| {
                    Action::from_config(&cfg.action, &cfg.args, &cfg.cmd, &cfg.unit, &slot_id)
                })
                .collect();
            commit_pointer_binding(
                state,
                mgr,
                seat,
                qh,
                btn_or_mod,
                current_mods,
                actions,
                mode,
            );
        }
        crate::config::KeyBindingEntry::Group(sub_map) => {
            let extra_mods = parse_mod_group(btn_or_mod);
            let combined_mods = current_mods | extra_mods;
            for (sub_key, sub_entry) in sub_map {
                process_pointer_entry(
                    state,
                    mgr,
                    seat,
                    qh,
                    sub_key,
                    combined_mods,
                    sub_entry,
                    mode,
                );
            }
        }
    }
}

pub fn setup_keybindings(state: &mut AppState, qh: &QueueHandle<AppState>) {
    let seat = match &state.main_seat {
        Some(s) => s.clone(),
        None => return,
    };
    let xkb_mgr = match &state.xkb_manager {
        Some(m) => m.clone(),
        None => return,
    };

    // --- 加载 Normal 绑定 ---
    if let Some(entries) = state.config.keybindings.clone() {
        info!("-> Registering [Normal] keybindings...");
        for (key_or_mod, entry) in &entries {
            process_entry(
                state,
                &xkb_mgr,
                &seat,
                qh,
                key_or_mod,
                Modifiers::empty(),
                entry,
                BindingMode::Normal,
            );
        }
    } else {
        // 默认键位也是 Normal
        warn!("-> No keybindings found, loading defaults...");
        let defaults = crate::config::get_default_bindings();
        for b in defaults {
            commit_binding(
                state,
                &xkb_mgr,
                &seat,
                qh,
                b.key,
                b.mods,
                vec![b.action],
                BindingMode::Normal,
            );
        }
    }

    // --- 加载 Resize 绑定 ---
    if let Some(entries) = state.config.resize.clone() {
        info!("-> Registering [Resize] keybindings...");
        for (key_or_mod, entry) in &entries {
            process_entry(
                state,
                &xkb_mgr,
                &seat,
                qh,
                key_or_mod,
                Modifiers::empty(),
                entry,
                BindingMode::Resize,
            );
        }
    }
    // --- 新增加载 Pointer 鼠标绑定 ---
    if let Some(entries) = state.config.pointer.clone() {
        info!("-> Registering [Pointer] bindings...");
        for (key_or_mod, entry) in &entries {
            process_pointer_entry(
                state,
                &xkb_mgr,
                &seat,
                qh,
                key_or_mod,
                Modifiers::empty(),
                entry,
                BindingMode::Normal, // 目前鼠标绑定默认都在 Normal 模式下工作
            );
        }
    }
}
