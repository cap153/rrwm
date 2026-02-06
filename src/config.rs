use crate::protocol::river_wm::river_seat_v1::Modifiers;
use crate::wm::actions::Action;
use crate::wm::layout::Direction;
use log::{error, info, warn};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

// 1. 定义显示器位置
#[derive(Deserialize, Debug, Clone)]
pub struct PositionConfig {
    pub x: String,
    pub y: String,
}

// 2. 每个显示器的具体配置
#[derive(Deserialize, Debug, Clone)]
pub struct OutputConfig {
    #[serde(alias = "focus-at-startup")]
    pub focus_at_startup: Option<String>,
    pub mode: Option<String>,
    pub scale: Option<String>,
    pub transform: Option<String>,
    pub position: Option<PositionConfig>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct WaybarConfig {
    pub tag_icons: Option<Vec<String>>,
    pub focused_style: Option<String>,
    pub occupied_style: Option<String>,
    pub empty_style: Option<String>,
}

// 1. 对应 [input.keyboard] 部分
#[derive(Deserialize, Debug, Clone)]
pub struct KeyboardConfig {
    pub layout: String,
    pub variant: Option<String>,
    pub options: Option<String>,
    pub model: Option<String>,
    pub numlock: Option<String>,
}

// 定义边框具体参数
#[derive(Deserialize, Debug, Clone)]
pub struct BorderParams {
    pub width: String,
    pub color: String,
}

// 定义 active 分组
#[derive(Deserialize, Debug, Clone)]
pub struct ActiveConfig {
    pub border: Option<BorderParams>,
}

// 定义 window 分组
#[derive(Deserialize, Debug, Clone)]
pub struct WindowConfig {
    #[serde(alias = "smart-borders", default)]
    pub smart_borders: String,
    pub gaps: Option<String>,
    pub active: Option<ActiveConfig>,
}

// 2. 对应 [input] 部分
#[derive(Deserialize, Debug, Clone)]
pub struct InputConfig {
    pub keyboard: Option<KeyboardConfig>,
}

// 3. 对应具体的动作配置
#[derive(Deserialize, Debug, Clone)]
pub struct ActionConfig {
    pub action: String,
    pub args: Option<Vec<String>>,
    pub cmd: Option<String>,
}

// 4. 处理混合结构（直接按键 vs 分组按键）
#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum KeyBindingEntry {
    /// 对应直接定义的单个按键，如 q = { action = "..." }
    Action(ActionConfig),
    /// 对应动作列表，如 c = [ { action = "..." }, { action = "..." } ]
    List(Vec<ActionConfig>),
    /// Box<KeyBindingEntry> 以支持递归，既可以写单个动作，也可以写动作列表
    Group(HashMap<String, Box<KeyBindingEntry>>),
}

// 5. 根配置结构体
#[derive(Deserialize, Debug, Clone)]
pub struct Config {
    pub input: Option<InputConfig>,
    pub keybindings: Option<HashMap<String, KeyBindingEntry>>,
    pub waybar: Option<WaybarConfig>,
    pub output: Option<HashMap<String, OutputConfig>>,
    pub window: Option<WindowConfig>,
}

impl Config {
    /// 获取配置文件路径：~/.config/river/rrwm.toml
    pub fn get_path() -> PathBuf {
        let home = std::env::var("HOME").expect("The HOME environment variable was not found.");
        PathBuf::from(home)
            .join(".config")
            .join("river")
            .join("rrwm.toml")
    }

    /// 加载配置文件
    pub fn load() -> Self {
        let path = Self::get_path();

        if let Ok(content) = fs::read_to_string(&path) {
            match toml::from_str::<Config>(&content) {
                Ok(config) => {
                    info!("-> Configuration file loaded: {:?}", path);
                    return config;
                }
                Err(e) => {
                    error!(
                        "-> Configuration file parsing failed: {}, Default settings will be used",
                        e
                    );
                }
            }
        } else {
            warn!(
                "-> Configuration file not found {:?}，Default settings will be used",
                path
            );
        }

        // 修复点 2：补全 keybindings 字段初始化
        Config {
            input: None,
            keybindings: None,
            waybar: None,
            output: None,
            window: None,
        }
    }
}

// --- 保留原有的默认快捷键逻辑 ---

pub struct DefaultBinding {
    pub mods: Modifiers,
    pub key: &'static str,
    pub action: Action,
}

pub fn get_default_bindings() -> Vec<DefaultBinding> {
    vec![
        DefaultBinding {
            mods: Modifiers::Mod1,
            key: "n",
            action: Action::Focus(Direction::Left),
        },
        DefaultBinding {
            mods: Modifiers::Mod1,
            key: "i",
            action: Action::Focus(Direction::Right),
        },
        DefaultBinding {
            mods: Modifiers::Mod1,
            key: "u",
            action: Action::Focus(Direction::Up),
        },
        DefaultBinding {
            mods: Modifiers::Mod1,
            key: "e",
            action: Action::Focus(Direction::Down),
        },
        DefaultBinding {
            mods: Modifiers::Mod1,
            key: "q",
            action: Action::CloseFocused,
        },
        DefaultBinding {
            mods: Modifiers::Mod1,
            key: "Return",
            action: Action::Spawn(vec!["kitty".to_string()]),
        },
    ]
}
