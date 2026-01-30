use crate::wm::actions::Action;
use crate::wm::layout::Direction;
use crate::protocol::river_wm::river_seat_v1::Modifiers;
use serde::Deserialize;
use std::collections::HashMap;

// 1. 键盘布局配置 (Colemak 用户最关心的部分)
#[derive(Deserialize, Debug, Clone)]
pub struct KeyboardConfig {
    pub layout: String,
    pub variant: Option<String>,
    pub options: Option<String>,
    pub model: Option<String>,
}

// 2. 快捷键配置 (对应你设计的 TOML 格式)
#[derive(Deserialize, Debug, Clone)]
pub struct BindingConfig {
    pub mods: Vec<String>,
    pub action: String,
    pub args: Option<Vec<String>>,
    pub cmd: Option<String>,
}

// 3. 总配置结构体
#[derive(Deserialize, Debug)]
pub struct Config {
    #[serde(default = "default_keyboard")]
    pub keyboard: KeyboardConfig,
    pub keybindings: HashMap<String, BindingConfig>,
}

// 默认键盘布局配置 (兜底方案)
fn default_keyboard() -> KeyboardConfig {
    KeyboardConfig {
        layout: "us".to_string(),
        variant: None,
        options: None,
        model: None,
    }
}



pub struct DefaultBinding {
    pub mods: Modifiers,
    pub key: &'static str,
    pub action: Action,
}

pub fn get_default_bindings() -> Vec<DefaultBinding> {
    vec![
        DefaultBinding { mods: Modifiers::Mod1, key: "n", action: Action::Focus(Direction::Left) },
        DefaultBinding { mods: Modifiers::Mod1, key: "i", action: Action::Focus(Direction::Right) },
        DefaultBinding { mods: Modifiers::Mod1, key: "u", action: Action::Focus(Direction::Up) },
        DefaultBinding { mods: Modifiers::Mod1, key: "e", action: Action::Focus(Direction::Down) },
        DefaultBinding { mods: Modifiers::Mod1, key: "q", action: Action::CloseFocused },
        DefaultBinding { 
            mods: Modifiers::Mod1, 
            key: "Return", 
            action: Action::Spawn("kitty".to_string()) 
        },
    ]
}
