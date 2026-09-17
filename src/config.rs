use crate::protocol::river_wm::river_seat_v1::Modifiers;
use crate::wm::actions::Action;
use crate::wm::layout::Direction;
use serde::de;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use tracing::{error, info, warn};

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

#[derive(Deserialize, Debug, Clone)]
pub struct AnimationsConfig {
    pub enable: Option<String>,
    pub duration: Option<String>,
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

// 内部结构体，仅用于反序列化（含 resize_color / resize-color 别名）。
#[derive(Deserialize, Debug, Clone, Default, PartialEq)]
struct BorderParamsInner {
    enabled: Option<String>,
    width: Option<String>,
    color: Option<String>,
    #[serde(rename = "resize_color", alias = "resize-color")]
    resize_color: Option<String>,
}

// 定义边框具体参数。所有字段均为可选：
// - [window.active].border 使用完整字段；
// - [window.rule].border 的覆盖对象形式只写需要覆盖的字段，其余继承全局。
// `enabled` 仅接受 "true" / "false"（大小写不敏感）；非法值（如 "abc"）在反序列化阶段
// 直接报错，避免错误配置静默关闭边框（与 shorthand 仅接受 "false" 的严格策略保持一致）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BorderParams {
    pub enabled: Option<String>,
    pub width: Option<String>,
    pub color: Option<String>,
    pub resize_color: Option<String>,
}

impl<'de> Deserialize<'de> for BorderParams {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let inner = BorderParamsInner::deserialize(deserializer)?;
        if let Some(e) = inner.enabled.as_deref() {
            if !e.eq_ignore_ascii_case("true") && !e.eq_ignore_ascii_case("false") {
                return Err(de::Error::custom(format!(
                    "invalid border enabled: {:?}, expected \"true\" or \"false\"",
                    e
                )));
            }
        }
        Ok(BorderParams {
            enabled: inner.enabled,
            width: inner.width,
            color: inner.color,
            resize_color: inner.resize_color,
        })
    }
}

// 定义 active 分组
#[derive(Deserialize, Debug, Clone)]
pub struct ActiveConfig {
    pub border: Option<BorderParams>,
}

/// 单条窗口规则中的 border 配置：
/// - `Disabled` 等价于 `border = "false"`；
/// - `Override` 是一个字段级覆盖对象，未给出的字段继续继承 [window.active].border。
#[derive(Debug, Clone, PartialEq)]
pub enum WindowBorderRule {
    Disabled,
    Override(BorderParams),
}

impl<'de> Deserialize<'de> for WindowBorderRule {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use std::fmt;

        struct WindowBorderRuleVisitor;
        impl<'de> de::Visitor<'de> for WindowBorderRuleVisitor {
            type Value = WindowBorderRule;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str(r#"either the string "false" or a table of BorderParams"#)
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                if v.eq_ignore_ascii_case("false") {
                    Ok(WindowBorderRule::Disabled)
                } else {
                    Err(E::custom(format!(
                        "invalid border shorthand: {:?}, only \"false\" is accepted",
                        v
                    )))
                }
            }

            fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
                self.visit_str(&v)
            }

            fn visit_map<A: de::MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
                let params = BorderParams::deserialize(de::value::MapAccessDeserializer::new(map))?;
                Ok(WindowBorderRule::Override(params))
            }
        }

        deserializer.deserialize_any(WindowBorderRuleVisitor)
    }
}

/// 运行时解析结果：最终生效的边框数值。
#[derive(Debug, Clone)]
pub struct ResolvedBorder {
    pub enabled: bool,
    pub width: u32,
    pub color: String,
    pub resize_color: String,
}

impl Config {
    /// 将「全局 [window.active].border」与「该窗口匹配到的 rule 的 border 覆盖」
    /// 合并为最终生效的边框参数。
    ///
    /// 继承语义：
    /// - 基础值来自 [window.active].border：
    ///   * 未写 `enabled` → true（保持原有默认开）；`enabled="true"` → true；`enabled="false"` → false。
    ///   * width 默认 0，color 默认 "#ffffff"，resize_color 默认 "#ff0000"。
    /// - `Disabled` 强制关闭（enabled=false）。
    /// - `Override` 逐字段覆盖；其中 `enabled` 为 None 时继承基础 enabled 状态。
    pub fn resolve_window_border(&self, rule: Option<&WindowBorderRule>) -> ResolvedBorder {
        let active = self
            .window
            .as_ref()
            .and_then(|w| w.active.as_ref())
            .and_then(|a| a.border.as_ref());

        let base_enabled = active
            .and_then(|b| b.enabled.as_deref())
            .map(|s| s.eq_ignore_ascii_case("true"))
            .unwrap_or(true);
        let base_width = active
            .and_then(|b| b.width.as_deref())
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);
        let base_color = active
            .and_then(|b| b.color.as_deref())
            .unwrap_or("#ffffff")
            .to_string();
        let base_resize_color = active
            .and_then(|b| b.resize_color.as_deref())
            .unwrap_or("#ff0000")
            .to_string();

        let mut enabled = base_enabled;
        let mut width = base_width;
        let mut color = base_color;
        let mut resize_color = base_resize_color;

        if let Some(r) = rule {
            match r {
                WindowBorderRule::Disabled => {
                    enabled = false;
                }
                WindowBorderRule::Override(params) => {
                    if let Some(e) = params.enabled.as_deref() {
                        enabled = e.to_lowercase() == "true";
                    }
                    if let Some(w) = params.width.as_deref() {
                        if let Ok(parsed) = w.parse::<u32>() {
                            width = parsed;
                        }
                    }
                    if let Some(c) = params.color.as_deref() {
                        color = c.to_string();
                    }
                    if let Some(rc) = params.resize_color.as_deref() {
                        resize_color = rc.to_string();
                    }
                }
            }
        }

        ResolvedBorder {
            enabled,
            width,
            color,
            resize_color,
        }
    }
}

// --- 定义单条匹配规则 ---
#[derive(Deserialize, Debug, Clone)]
pub struct WindowRuleMatch {
    pub appid: Option<String>,
    pub title: Option<String>,
    pub icon: Option<String>,
    pub width: Option<String>,
    pub height: Option<String>,
    pub floating: Option<String>,
    pub fullscreen: Option<String>,
    pub border: Option<WindowBorderRule>,
}

// --- 定义 rule 分组 ---
#[derive(Deserialize, Debug, Clone)]
pub struct WindowRuleConfig {
    // 因为 match 是关键字，所以字段名用 matches
    #[serde(rename = "match")]
    pub matches: Option<Vec<WindowRuleMatch>>,
}

// 定义 window 分组
#[derive(Deserialize, Debug, Clone)]
pub struct WindowConfig {
    #[serde(alias = "smart-borders", default)]
    pub smart_borders: String,
    pub gaps: Option<String>,
    pub active: Option<ActiveConfig>,
    pub rule: Option<WindowRuleConfig>,
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
    pub unit: Option<String>,
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
    pub pointer: Option<HashMap<String, KeyBindingEntry>>,
    pub resize: Option<HashMap<String, KeyBindingEntry>>,
    pub waybar: Option<WaybarConfig>,
    pub animations: Option<AnimationsConfig>,
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
            resize: None,
            pointer: None,
            waybar: None,
            animations: None,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_rule(toml_str: &str) -> Result<WindowRuleMatch, toml::de::Error> {
        toml::from_str::<WindowRuleMatch>(toml_str)
    }

    // 测试 1：字符串 shorthand `border = "false"` 必须解析为 Disabled
    #[test]
    fn shorthand_false_is_disabled() {
        let r = parse_rule(
            r#"appid = "kitty"
border = "false""#,
        )
        .unwrap();
        assert_eq!(r.border, Some(WindowBorderRule::Disabled));
    }

    // 测试 1b：大小写不敏感
    #[test]
    fn shorthand_false_case_insensitive() {
        let r = parse_rule(
            r#"appid = "kitty"
border = "FALSE""#,
        )
        .unwrap();
        assert_eq!(r.border, Some(WindowBorderRule::Disabled));
    }

    // 测试 2：对象形式 disabled
    #[test]
    fn object_disabled() {
        let r = parse_rule(
            r#"appid = "kitty"
border = { enabled = "false" }"#,
        )
        .unwrap();
        assert_eq!(
            r.border,
            Some(WindowBorderRule::Override(BorderParams {
                enabled: Some("false".into()),
                width: None,
                color: None,
                resize_color: None,
            }))
        );
    }

    // 测试 3：部分覆盖 width
    #[test]
    fn partial_override_width() {
        let r = parse_rule(
            r#"appid = "kitty"
border = { width = "1" }"#,
        )
        .unwrap();
        assert_eq!(
            r.border,
            Some(WindowBorderRule::Override(BorderParams {
                enabled: None,
                width: Some("1".into()),
                color: None,
                resize_color: None,
            }))
        );
    }

    // 测试 4：完整覆盖
    #[test]
    fn full_override() {
        let r = parse_rule(
            r##"appid = "kitty"
border = { enabled = "true", width = "2", color = "#50fa7b", resize_color = "#ffb86c" }"##,
        )
        .unwrap();
        assert_eq!(
            r.border,
            Some(WindowBorderRule::Override(BorderParams {
                enabled: Some("true".into()),
                width: Some("2".into()),
                color: Some("#50fa7b".into()),
                resize_color: Some("#ffb86c".into()),
            }))
        );
    }

    // 测试 5：非法 shorthand 必须失败
    #[test]
    fn invalid_shorthand_errors() {
        let r = parse_rule(
            r#"appid = "kitty"
border = "foo""#,
        );
        assert!(r.is_err());
    }

    // 测试 6：已有 [window.active].border 配置不能回归
    #[test]
    fn existing_active_border_parse() {
        let cfg: Config = toml::from_str(
            r##"
            [window.active]
            border = { width = "2", color = "#bd93f9", resize_color = "#ff5555" }
        "##,
        )
        .unwrap();
        let b = cfg.window.unwrap().active.unwrap().border.unwrap();
        assert_eq!(b.width, Some("2".into()));
        assert_eq!(b.color, Some("#bd93f9".into()));
        assert_eq!(b.resize_color, Some("#ff5555".into()));
    }

    // 测试 7：继承语义——全局 active border + 部分覆盖 width
    #[test]
    fn resolve_inherits_active() {
        let cfg: Config = toml::from_str(
            r##"
            [window.active]
            border = { width = "2", color = "#bd93f9", resize_color = "#ff5555" }

            [window.rule]
            match = [ { appid = "kitty", border = { width = "1" } } ]
        "##,
        )
        .unwrap();
        let rule = &cfg
            .window
            .as_ref()
            .unwrap()
            .rule
            .as_ref()
            .unwrap()
            .matches
            .as_ref()
            .unwrap()[0];
        let rb = cfg.resolve_window_border(rule.border.as_ref());
        assert!(rb.enabled);
        assert_eq!(rb.width, 1);
        assert_eq!(rb.color, "#bd93f9");
        assert_eq!(rb.resize_color, "#ff5555");
    }

    // 测试 8：Disabled 强制关闭
    #[test]
    fn resolve_disabled() {
        let cfg: Config = toml::from_str(
            r##"
            [window.active]
            border = { width = "2", color = "#bd93f9", resize_color = "#ff5555" }

            [window.rule]
            match = [ { appid = "stools", border = "false" } ]
        "##,
        )
        .unwrap();
        let rule = &cfg
            .window
            .as_ref()
            .unwrap()
            .rule
            .as_ref()
            .unwrap()
            .matches
            .as_ref()
            .unwrap()[0];
        let rb = cfg.resolve_window_border(rule.border.as_ref());
        assert!(!rb.enabled);
    }

    // 测试 9：[window.active].border.enabled="false" 必须真正关闭全局边框
    #[test]
    fn active_enabled_false() {
        let cfg: Config = toml::from_str(
            r##"
            [window.active]
            border = { enabled = "false", width = "2", color = "#bd93f9", resize_color = "#ff5555" }
        "##,
        )
        .unwrap();
        let rb = cfg.resolve_window_border(None);
        assert!(!rb.enabled);
        assert_eq!(rb.width, 2);
        assert_eq!(rb.color, "#bd93f9");
        assert_eq!(rb.resize_color, "#ff5555");
    }

    // 测试 10：对象形式的 enabled 非法值必须在加载阶段报错（不静默变成 false）
    #[test]
    fn invalid_enabled_errors() {
        let r = toml::from_str::<Config>(
            r##"
            [window.rule]
            match = [ { appid = "foo", border = { enabled = "invalid" } } ]
        "##,
        );
        assert!(r.is_err());
    }

    // 测试 11：没有 [window.active].border，且 rule 部分覆盖 width
    // → 全局基础 enabled 默认 true、width 默认 0；rule 继承 enabled 并覆盖 width=1
    #[test]
    fn no_active_border_with_rule_width() {
        let cfg: Config = toml::from_str(
            r##"
            [window]
            gaps = "2"

            [window.rule]
            match = [ { appid = "kitty", border = { width = "1" } } ]
        "##,
        )
        .unwrap();
        let rule = &cfg
            .window
            .as_ref()
            .unwrap()
            .rule
            .as_ref()
            .unwrap()
            .matches
            .as_ref()
            .unwrap()[0];
        let rb = cfg.resolve_window_border(rule.border.as_ref());
        // 全局基础：enabled=true, width=0；rule 覆盖 width=1，enabled 继承 true
        assert!(rb.enabled);
        assert_eq!(rb.width, 1);
    }

    // 测试 12：[window.active].border 不写 enabled → 默认 enabled=true
    #[test]
    fn active_border_default_enabled_true() {
        let cfg: Config = toml::from_str(
            r##"
            [window.active]
            border = { width = "2", color = "#bd93f9", resize_color = "#ff5555" }
        "##,
        )
        .unwrap();
        let rb = cfg.resolve_window_border(None);
        assert!(rb.enabled);
        assert_eq!(rb.width, 2);
    }

    // 测试 13：[window.active].border.enabled="true" 显式开启
    #[test]
    fn active_border_enabled_true() {
        let cfg: Config = toml::from_str(
            r##"
            [window.active]
            border = { enabled = "true", width = "2", color = "#bd93f9", resize_color = "#ff5555" }
        "##,
        )
        .unwrap();
        let rb = cfg.resolve_window_border(None);
        assert!(rb.enabled);
        assert_eq!(rb.width, 2);
    }

    // 测试 11：rule 部分覆盖 resize_color，普通色仍继承全局
    #[test]
    fn resize_color_override_inherits_normal() {
        let cfg: Config = toml::from_str(
            r##"
            [window.active]
            border = { width = "2", color = "#111111", resize_color = "#222222" }

            [window.rule]
            match = [ { appid = "foo", border = { resize_color = "#333333" } } ]
        "##,
        )
        .unwrap();
        let rule = &cfg
            .window
            .as_ref()
            .unwrap()
            .rule
            .as_ref()
            .unwrap()
            .matches
            .as_ref()
            .unwrap()[0];
        let rb = cfg.resolve_window_border(rule.border.as_ref());
        assert!(rb.enabled);
        assert_eq!(rb.color, "#111111");
        assert_eq!(rb.resize_color, "#333333");
    }
}
