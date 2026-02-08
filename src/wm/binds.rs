use crate::protocol::river_wm::river_seat_v1::{Modifiers, RiverSeatV1};
use crate::protocol::river_xkb::river_xkb_bindings_v1::RiverXkbBindingsV1;
use crate::wm::{actions::Action, AppState, KeyBinding};
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
// src/wm/binds.rs

fn commit_binding(
    state: &mut AppState,
    mgr: &RiverXkbBindingsV1,
    seat: &RiverSeatV1,
    qh: &QueueHandle<AppState>,
    key_name: &str,
    mods: Modifiers,
    actions: Vec<Action>,
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
) {
    match entry {
        // 情况 1：单个动作
        crate::config::KeyBindingEntry::Action(cfg) => {
            let actions = vec![Action::from_config(&cfg.action, &cfg.args, &cfg.cmd)];
            commit_binding(state, mgr, seat, qh, key_or_mod, current_mods, actions);
        }
        // 情况 2：动作列表 [ {action=...}, {action=...} ]
        crate::config::KeyBindingEntry::List(cfgs) => {
            let actions = cfgs
                .iter()
                .map(|cfg| Action::from_config(&cfg.action, &cfg.args, &cfg.cmd))
                .collect();
            commit_binding(state, mgr, seat, qh, key_or_mod, current_mods, actions);
        }
        // 情况 3：修饰符分组 [keybindings.alt]
        crate::config::KeyBindingEntry::Group(sub_map) => {
            // 解析这一层增加的修饰符
            let extra_mods = parse_mod_group(key_or_mod);
            let combined_mods = current_mods | extra_mods;

            // 递归处理子项
            for (sub_key, sub_entry) in sub_map {
                process_entry(state, mgr, seat, qh, sub_key, combined_mods, sub_entry);
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

    if let Some(entries) = state.config.keybindings.clone() {
        info!("-> Registering shortcut keys from configuration file...");
        for (key_or_mod, entry) in &entries {
            process_entry(
                state,
                &xkb_mgr,
                &seat,
                qh,
                key_or_mod,
                Modifiers::empty(),
                entry,
            );
        }
    } else {
        warn!("-> 未发现快捷键配置，加载默认 Colemak 导航键位...");
        let defaults = crate::config::get_default_bindings();
        for b in defaults {
            commit_binding(state, &xkb_mgr, &seat, qh, b.key, b.mods, vec![b.action]);
        }
    }
}
