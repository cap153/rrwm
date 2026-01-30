use crate::wm::actions::Action;
use crate::wm::layout::Direction;
use crate::protocol::river_wm::river_seat_v1::Modifiers;
use serde::Deserialize;
use std::path::PathBuf;
use std::fs;

// 1. 对应 [input.keyboard] 部分
#[derive(Deserialize, Debug, Clone)]
pub struct KeyboardConfig {
    pub layout: String,
    pub variant: Option<String>,
    pub options: Option<String>,
    pub model: Option<String>,
}

// 2. 对应 [input] 部分
#[derive(Deserialize, Debug, Clone)]
pub struct InputConfig {
    pub keyboard: Option<KeyboardConfig>,
}

// 3. 根配置结构体
#[derive(Deserialize, Debug, Clone)]
pub struct Config {
    pub input: Option<InputConfig>,
    // 以后这里可以加 keybindings 等
}

impl Config {
    /// 获取配置文件路径：~/.config/river/rrwm.toml
    pub fn get_path() -> PathBuf {
        let home = std::env::var("HOME").expect("找不到 HOME 环境变量");
        PathBuf::from(home).join(".config").join("river").join("rrwm.toml")
    }

    /// 加载配置文件
    pub fn load() -> Self {
        let path = Self::get_path();
        
        if let Ok(content) = fs::read_to_string(&path) {
            match toml::from_str::<Config>(&content) {
                Ok(config) => {
                    println!("-> 已加载配置文件: {:?}", path);
                    return config;
                }
                Err(e) => {
                    eprintln!("-> 配置文件解析失败: {}，将使用默认设置", e);
                }
            }
        } else {
            println!("-> 未找到配置文件 {:?}，将使用默认设置", path);
        }

        // 如果文件不存在或解析失败，返回一个全空的配置
        Config { input: None }
    }
}

// --- 保留原有的默认快捷键逻辑，用于过渡 ---

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
