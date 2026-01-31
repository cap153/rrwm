// src/wm/binds.rs

use crate::protocol::river_wm::river_seat_v1::{Modifiers, RiverSeatV1};
use crate::protocol::river_xkb::river_xkb_bindings_v1::RiverXkbBindingsV1;
use crate::wm::{actions::Action, AppState, KeyBinding};
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
            _ => println!("警告：未知的修饰符标签 {}", p),
        }
    }
    mask
}

/// 辅助函数：注册单个绑定
fn register_single_binding(
    state: &mut AppState,
    mgr: &RiverXkbBindingsV1,
    seat: &RiverSeatV1,
    qh: &QueueHandle<AppState>,
    key_name: &str,
    mods: Modifiers,
    cfg: &crate::config::ActionConfig,
) {
    let keysym = xkb::keysym_from_name(key_name, xkb::KEYSYM_NO_FLAGS).raw();
    let action = Action::from_config(&cfg.action, &cfg.args, &cfg.cmd);

    let binding_obj = mgr.get_xkb_binding(seat, keysym, mods, qh, ());

    state.key_bindings.push(KeyBinding {
        obj: binding_obj,
        action,
    });
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

    // 克隆一份配置，避免在修改 key_bindings 时产生借用冲突
    if let Some(entries) = state.config.keybindings.clone() {
        println!("-> 正在从配置文件注册快捷键...");
        for (key_or_mod, entry) in entries {
            match entry {
                // 情况 1：直接定义的按键 (无修饰符)
                crate::config::KeyBindingEntry::Action(cfg) => {
                    register_single_binding(
                        state,
                        &xkb_mgr,
                        &seat,
                        qh,
                        &key_or_mod,
                        Modifiers::empty(),
                        &cfg,
                    );
                }
                // 情况 2：分组按键
                crate::config::KeyBindingEntry::Group(keys) => {
                    let mods = parse_mod_group(&key_or_mod);
                    for (key_name, cfg) in keys {
                        register_single_binding(state, &xkb_mgr, &seat, qh, &key_name, mods, &cfg);
                    }
                }
            }
        }
    } else {
        println!("-> 未发现快捷键配置，加载默认 Colemak 导航键位...");
        let defaults = crate::config::get_default_bindings();
        for b in defaults {
            let keysym = xkb::keysym_from_name(b.key, xkb::KEYSYM_NO_FLAGS).raw();
            let binding_obj = xkb_mgr.get_xkb_binding(&seat, keysym, b.mods, qh, ());
            state.key_bindings.push(KeyBinding {
                obj: binding_obj,
                action: b.action,
            });
        }
    }
}
