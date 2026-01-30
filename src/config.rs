// src/config.rs
use crate::wm::actions::Action;
use crate::wm::layout::Direction;
use crate::protocol::river_wm::river_seat_v1::Modifiers;

pub struct DefaultBinding {
    pub mods: Modifiers,
    pub key: &'static str,
    pub action: Action,
}

pub fn get_default_bindings() -> Vec<DefaultBinding> {
    vec![
        // 注意：MOD4 改为 Mod4
        DefaultBinding { mods: Modifiers::Mod4, key: "j", action: Action::Focus(Direction::Left) },
        DefaultBinding { mods: Modifiers::Mod4, key: "l", action: Action::Focus(Direction::Right) },
        DefaultBinding { mods: Modifiers::Mod4, key: "i", action: Action::Focus(Direction::Up) },
        DefaultBinding { mods: Modifiers::Mod4, key: "k", action: Action::Focus(Direction::Down) },
        DefaultBinding { mods: Modifiers::Mod4, key: "q", action: Action::CloseFocused },
        DefaultBinding { 
            mods: Modifiers::Mod4, 
            key: "Return", 
            action: Action::Spawn("kitty".to_string()) 
        },
    ]
}
