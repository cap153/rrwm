use crate::protocol::wlr_output_management::zwlr_output_mode_v1::ZwlrOutputModeV1;
use crate::wm::layout::{Direction, Geometry, LayoutNode, SplitType};
use crate::wm::AppState;
use crate::wm::OutputData;
use serde::Serialize;
use std::io::{Read, Write};
use tracing::{error, info, warn};
use wayland_backend::client::ObjectId; // 修复点：引入 ObjectId 类型
use wayland_client::protocol::wl_output::Transform; // 旋转枚举
use wayland_client::{Proxy, QueueHandle};

// 定义发送给 Bar 的状态数据包
#[derive(Serialize, Clone)]
pub struct RrwmStatus {
    pub focused_tags: u32,     // 当前正在看哪个标签 (掩码)
    pub occupied_tags: u32,    // 哪些标签里有窗口 (掩码)
    pub active_window: String, // 当前聚焦的窗口标题 (比如 "Kitty")
}

#[derive(Serialize, Clone)]
pub struct WaybarResponse {
    pub text: String,
    pub tooltip: String,
    pub class: String,
}

#[derive(Debug, PartialEq)]
enum MoveHint {
    Leftmost,   // 强制出现在最左边
    Rightmost,  // 强制出现在最右边
    Topmost,    // 强制出现在最上方
    Bottommost, // 强制出现在最下方
}

// --- 调整方向枚举 ---
#[derive(Debug, Clone, Copy)]
pub enum ResizeAxis {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone)]
pub enum Action {
    CloseFocused,
    ToggleFullscreen,
    ToggleFloat,      // 当前聚焦的窗口切换悬浮状态
    SwitchFocusFloat, // 在悬浮和平铺窗口之间切换焦点
    Focus(Direction),
    FocusTag(u32),           // 切换到某个标签掩码
    MoveToTag(u32),          // 将窗口移动到某个标签掩码
    Move(Direction),         // 统一处理方向性移动
    FocusOutput(Direction),  // 处理 left_output / right_output
    MoveToOutput(Direction), // 处理 left_output / right_output
    Spawn(Vec<String>),      // 纯净启动：[程序名, 参数1, 参数2]
    Shell(String),           // Shell 启动：一整串命令字符串
    ReloadConfiguration,     // 重载配置
    ToggleResizeMode,
    ExitResizeMode,
    Resize(ResizeAxis, i32),  // 轴向, 增量(像素)
    MoveStep(Direction, i32), // 方向, 步进(像素) - 用于 Resize 模式下的移动
}

impl Action {
    /// 核心逻辑：把 TOML 里的字符串配置变成代码里的枚举
    pub fn from_config(
        name: &str,
        args: &Option<Vec<String>>,
        cmd: &Option<String>,
        unit: &Option<String>,
    ) -> Self {
        // 解析 unit，默认为 10 (如果配置了 resize 动作但没写 unit)
        let step = unit
            .as_ref()
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or(10);

        match name.to_lowercase().as_str() {
            // --- 内部指令：关闭窗口 ---
            "close_window" | "close_focused" => Action::CloseFocused,
            // --- 内部指令：全屏切换 ---
            "fullscreen" | "toggle_fullscreen" => Action::ToggleFullscreen,
            // --- 内部指令：悬浮窗切换 ---
            "toggle_window_floating" | "toggle_float" => Action::ToggleFloat,
            // --- 内部指令：悬浮窗/平铺焦点切换 ---
            "switch_focus_between_floating_and_tiling" | "switch_float_tiling" => {
                Action::SwitchFocusFloat
            }
            // --- 内部指令：重载配置 ---
            "reload_configuration" => Action::ReloadConfiguration,

            // --- Resize 模式控制 ---
            "toggle_resize_mode" => Action::ToggleResizeMode,
            "exit_resize_mode" => Action::ExitResizeMode,

            // --- 尺寸调整指令 ---
            "shrink_width" => Action::Resize(ResizeAxis::Horizontal, -step),
            "grow_width" => Action::Resize(ResizeAxis::Horizontal, step),
            "shrink_height" => Action::Resize(ResizeAxis::Vertical, -step),
            "grow_height" => Action::Resize(ResizeAxis::Vertical, step),

            "focus" => {
                let arg = args
                    .as_ref()
                    .and_then(|v| v.get(0))
                    .map(|s| s.as_str())
                    .unwrap_or("right");
                match arg {
                    "left_output" => Action::FocusOutput(Direction::Left),
                    "right_output" => Action::FocusOutput(Direction::Right),
                    "up_output" => Action::FocusOutput(Direction::Up),
                    "down_output" => Action::FocusOutput(Direction::Down),
                    "left" => Action::Focus(Direction::Left),
                    "right" => Action::Focus(Direction::Right),
                    "up" => Action::Focus(Direction::Up),
                    "down" => Action::Focus(Direction::Down),
                    _ => {
                        if let Ok(idx) = arg.parse::<u32>() {
                            Action::FocusTag(1 << (idx.saturating_sub(1)))
                        } else {
                            Action::Focus(Direction::Right)
                        }
                    }
                }
            }
            "move" => {
                let arg = args
                    .as_ref()
                    .and_then(|v| v.get(0))
                    .map(|s| s.as_str())
                    .unwrap_or("1");
                match arg {
                    "left_output" => Action::MoveToOutput(Direction::Left),
                    "right_output" => Action::MoveToOutput(Direction::Right),
                    "up_output" => Action::MoveToOutput(Direction::Up),
                    "down_output" => Action::MoveToOutput(Direction::Down),

                    // --- 如果提供了 unit，则是 MoveStep，否则是 Move ---
                    "left" => {
                        if unit.is_some() {
                            Action::MoveStep(Direction::Left, step)
                        } else {
                            Action::Move(Direction::Left)
                        }
                    }
                    "right" => {
                        if unit.is_some() {
                            Action::MoveStep(Direction::Right, step)
                        } else {
                            Action::Move(Direction::Right)
                        }
                    }
                    "up" => {
                        if unit.is_some() {
                            Action::MoveStep(Direction::Up, step)
                        } else {
                            Action::Move(Direction::Up)
                        }
                    }
                    "down" => {
                        if unit.is_some() {
                            Action::MoveStep(Direction::Down, step)
                        } else {
                            Action::Move(Direction::Down)
                        }
                    }

                    _ => {
                        if let Ok(idx) = arg.parse::<u32>() {
                            Action::MoveToTag(1 << (idx.saturating_sub(1)))
                        } else {
                            Action::Move(Direction::Right)
                        }
                    }
                }
            }
            // "spawn" 模式：直接启动，不经过 sh
            "spawn" => Action::Spawn(args.clone().unwrap_or_default()),
            // "shell" 模式：交给 sh -c 处理复杂逻辑
            "shell" => Action::Shell(cmd.clone().unwrap_or_default()),
            _ => {
                warn!("Warning: Unknown action name {}", name);
                Action::Shell("true".to_string())
            }
        }
    }
}
impl AppState {
    fn cycle_output_focus(&mut self, dir: Direction) {
        let current_out = match &self.focused_output {
            Some(id) => id.clone(),
            None => return,
        };

        let mut sorted: Vec<_> = self.outputs.iter().collect();
        sorted.sort_by_key(|(_, data)| match dir {
            Direction::Left | Direction::Right => data.usable_area.x,
            Direction::Up | Direction::Down => data.usable_area.y,
        });

        if let Some(pos) = sorted.iter().position(|(id, _)| **id == current_out) {
            let next_idx = match dir {
                Direction::Right | Direction::Down => (pos + 1) % sorted.len(),
                Direction::Left | Direction::Up => (pos + sorted.len() - 1) % sorted.len(),
            };

            if next_idx == pos {
                return;
            }

            let (next_id, next_data) = sorted[next_idx];
            let next_id = next_id.clone();
            let next_monitor_tags = next_data.tags;

            info!(
                "-> [Cross-screen jump] {} (Tag mask: {:b}) -> {} (Tag mask: {:b})",
                current_out, self.focused_tags, next_id, next_monitor_tags
            );

            // --- 【全屏霸权判断】 ---
            // 检查目标显示器是否有全屏窗口
            let fullscreen_win = self
                .windows
                .iter()
                .find(|w| {
                    w.output.as_ref() == Some(&next_id)
                        && (w.tags & next_monitor_tags) != 0
                        && w.is_fullscreen
                })
                .map(|w| w.id.clone());

            // 确定目标窗口：全屏优先 -> 其次找平铺边缘 -> 否则为空
            let target_win = if let Some(fs_id) = fullscreen_win {
                Some(fs_id)
            } else {
                // 原有的平铺边缘查找逻辑
                let landing_dir = match dir {
                    Direction::Up => Direction::Down,
                    Direction::Down => Direction::Up,
                    Direction::Left => Direction::Right,
                    Direction::Right => Direction::Left,
                };
                let tree_key = (next_id.clone(), next_monitor_tags);
                if let Some(root) = self.layout_roots.get(&tree_key) {
                    Some(Self::find_edge_in_tree(root, landing_dir))
                } else {
                    None
                }
            };

            // 3. 执行焦点和鼠标瞬移
            if let Some(win_id) = target_win {
                info!("-> [Focus] Lock target output window: {:?}", win_id);
                self.focused_window = Some(win_id.clone());
                // 更新该屏幕的历史记录，防止切走再切回来时状态丢失
                self.tag_focus_history
                    .insert((next_id.clone(), next_monitor_tags), win_id.clone());

                // 鼠标瞬移逻辑
                // 如果是全屏窗口，直接飞到屏幕中心，无需依赖 last_geometry
                let is_fs = self
                    .windows
                    .iter()
                    .find(|w| w.id == win_id)
                    .map(|w| w.is_fullscreen)
                    .unwrap_or(false);

                if is_fs {
                    let cx = next_data.usable_area.x + (next_data.usable_area.w / 2);
                    let cy = next_data.usable_area.y + (next_data.usable_area.h / 2);
                    self.pending_pointer_warp = Some((cx, cy));
                } else if let Some(geom) = self.last_geometry.get(&win_id) {
                    let cx = geom.x + (geom.w / 2);
                    let cy = geom.y + (geom.h / 2);
                    self.pending_pointer_warp = Some((cx, cy));
                }
            } else {
                // 如果目标屏幕是空的，去屏幕中心
                let cx = next_data.usable_area.x + (next_data.usable_area.w / 2);
                let cy = next_data.usable_area.y + (next_data.usable_area.h / 2);
                self.pending_pointer_warp = Some((cx, cy));
                self.focused_window = None;
            }

            // 4. 更新全局状态
            self.focused_output = Some(next_id);
            self.focused_tags = next_monitor_tags;

            if let Some(wm) = &self.river_wm {
                wm.manage_dirty();
            }
        }
    }
    /// 将窗口从一个物理显示器搬到另一个物理显示器（保持在当前 Tag）
    fn move_window_to_output(&mut self, win_id: &ObjectId, dir: Direction) {
        // 1. 获取窗口元数据
        let (old_out_name, win_tags) = match self.windows.iter().find(|w| &w.id == win_id) {
            Some(w) => (w.output.clone(), w.tags),
            None => return,
        };
        let old_out_name = match old_out_name {
            Some(n) => n,
            None => return,
        };

        // 2. 寻找目标显示器 (按轴排序逻辑保持不变)
        let mut sorted: Vec<_> = self.outputs.iter().collect();
        sorted.sort_by_key(|(_, data)| match dir {
            Direction::Left | Direction::Right => data.usable_area.x,
            Direction::Up | Direction::Down => data.usable_area.y,
        });

        if let Some(pos) = sorted.iter().position(|(name, _)| **name == old_out_name) {
            let next_idx = match dir {
                Direction::Right | Direction::Down => (pos + 1) % sorted.len(),
                Direction::Left | Direction::Up => (pos + sorted.len() - 1) % sorted.len(),
            };
            if next_idx == pos {
                return;
            }

            let (next_out_name, next_out_data) = sorted[next_idx];
            let next_out_name = next_out_name.clone();
            let target_monitor_tags = next_out_data.tags;

            // --- 【核心逻辑】根据方向决定“着陆位置” ---
            // 向右推 -> 从左边入 (Leftmost)
            // 向左推 -> 从右边入 (Rightmost)
            // 向下推 -> 从顶端入 (Topmost)
            // 向上推 -> 从底端入 (Bottommost)
            let hint = match dir {
                Direction::Right => MoveHint::Leftmost,
                Direction::Left => MoveHint::Rightmost,
                Direction::Down => MoveHint::Topmost,
                Direction::Up => MoveHint::Bottommost,
            };

            info!(
                "-> [Cross-screen transfer] The window is moved from {} to {} (location: {:?})",
                old_out_name, next_out_name, hint
            );

            // 3. 从旧树移除
            let old_key = (old_out_name.clone(), win_tags);
            if let Some(root) = self.layout_roots.remove(&old_key) {
                if let Some(new_root) = LayoutNode::remove_at(root, win_id) {
                    self.layout_roots.insert(old_key, new_root);
                }
            }

            // 4. 更新元数据
            let mut win_data = None;
            if let Some(w) = self.windows.iter_mut().find(|w| &w.id == win_id) {
                w.output = Some(next_out_name.clone());
                w.tags = target_monitor_tags;
                win_data = Some(w.clone());
            }

            // 5. 【修正】执行多向插入
            if let Some(wd) = win_data {
                let new_key = (next_out_name.clone(), target_monitor_tags);
                if let Some(old_root) = self.layout_roots.remove(&new_key) {
                    let new_root = match hint {
                        MoveHint::Leftmost => LayoutNode::Container {
                            split_type: SplitType::Vertical,
                            ratio: 0.5,
                            left_child: Box::new(LayoutNode::Window(wd)),
                            right_child: Box::new(old_root),
                        },
                        MoveHint::Rightmost => LayoutNode::Container {
                            split_type: SplitType::Vertical,
                            ratio: 0.5,
                            left_child: Box::new(old_root),
                            right_child: Box::new(LayoutNode::Window(wd)),
                        },
                        MoveHint::Topmost => LayoutNode::Container {
                            split_type: SplitType::Horizontal,
                            ratio: 0.5,
                            left_child: Box::new(LayoutNode::Window(wd)),
                            right_child: Box::new(old_root),
                        },
                        MoveHint::Bottommost => LayoutNode::Container {
                            split_type: SplitType::Horizontal,
                            ratio: 0.5,
                            left_child: Box::new(old_root),
                            right_child: Box::new(LayoutNode::Window(wd)),
                        },
                    };
                    self.layout_roots.insert(new_key.clone(), new_root);
                } else {
                    self.layout_roots
                        .insert(new_key.clone(), LayoutNode::Window(wd));
                }

                // 6. 状态同步
                self.focused_output = Some(next_out_name);
                self.focused_tags = target_monitor_tags;
                self.focused_window = Some(win_id.clone());
                self.tag_focus_history.insert(new_key, win_id.clone());

                if let Some(wm) = &self.river_wm {
                    wm.manage_dirty();
                }

                // 鼠标直接跳到目标显示器中心
                let cx = next_out_data.usable_area.x + (next_out_data.usable_area.w / 2);
                let cy = next_out_data.usable_area.y + (next_out_data.usable_area.h / 2);
                self.pending_pointer_warp = Some((cx, cy));
            }
        }
    }
    /// 核心：悬浮窗口的定向焦点查找（线性排序 + 跨 Tag 穿透）
    fn focus_floating_in_direction(&mut self, dir: Direction) {
        let f_id = match self.focused_window.clone() {
            Some(id) => id,
            None => return,
        };

        let (current_out, current_tags) = {
            if let Some(w) = self.windows.iter().find(|w| w.id == f_id) {
                (w.output.clone(), w.tags)
            } else {
                return;
            }
        };
        let current_out = match current_out {
            Some(o) => o,
            None => return,
        };

        // 1. 收集当前 Tag 的所有悬浮窗口
        let mut candidates: Vec<&crate::wm::WindowData> = self
            .windows
            .iter()
            .filter(|w| {
                w.is_floating
                    && !w.is_fullscreen
                    && w.output.as_ref() == Some(&current_out)
                    && (w.tags & current_tags) != 0
            })
            .collect();

        // 2. 根据方向线性排序 (解决重叠)
        match dir {
            Direction::Left | Direction::Right => {
                candidates.sort_by(|a, b| {
                    a.float_geo
                        .x
                        .cmp(&b.float_geo.x)
                        .then_with(|| a.float_geo.y.cmp(&b.float_geo.y))
                        .then_with(|| a.id.protocol_id().cmp(&b.id.protocol_id()))
                });
            }
            Direction::Up | Direction::Down => {
                candidates.sort_by(|a, b| {
                    a.float_geo
                        .y
                        .cmp(&b.float_geo.y)
                        .then_with(|| a.float_geo.x.cmp(&b.float_geo.x))
                        .then_with(|| a.id.protocol_id().cmp(&b.id.protocol_id()))
                });
            }
        }

        let cur_idx = match candidates.iter().position(|w| w.id == f_id) {
            Some(i) => i,
            None => return,
        };

        // 3. 核心行为：判定是内部移动还是跨 Tag 穿透
        match dir {
            Direction::Left => {
                if cur_idx > 0 {
                    // 内部切换
                    let target = candidates[cur_idx - 1];
                    self.focused_window = Some(target.id.clone());
                    if let Some(seat) = &self.main_seat {
                        seat.focus_window(&target.window);
                    }
                } else {
                    // 撞左墙 -> 跨 Tag 穿透
                    self.restrict_focus_to_floating = true; // 标记：我要找悬浮窗
                    self.pending_focus_dir = Some(dir);
                    self.cycle_tag(-1, dir);
                }
            }
            Direction::Right => {
                if cur_idx < candidates.len() - 1 {
                    let target = candidates[cur_idx + 1];
                    self.focused_window = Some(target.id.clone());
                    if let Some(seat) = &self.main_seat {
                        seat.focus_window(&target.window);
                    }
                } else {
                    // 撞右墙 -> 跨 Tag 穿透
                    self.restrict_focus_to_floating = true;
                    self.pending_focus_dir = Some(dir);
                    self.cycle_tag(1, dir);
                }
            }
            Direction::Up => {
                // 上下方向维持内部 Wrap 逻辑，不跨 Tag
                let target = if cur_idx == 0 {
                    candidates[candidates.len() - 1]
                } else {
                    candidates[cur_idx - 1]
                };
                self.focused_window = Some(target.id.clone());
                if let Some(seat) = &self.main_seat {
                    seat.focus_window(&target.window);
                }
            }
            Direction::Down => {
                let target = if cur_idx == candidates.len() - 1 {
                    candidates[0]
                } else {
                    candidates[cur_idx + 1]
                };
                self.focused_window = Some(target.id.clone());
                if let Some(seat) = &self.main_seat {
                    seat.focus_window(&target.window);
                }
            }
        }

        if let Some(wm) = &self.river_wm {
            wm.manage_dirty();
        }
    }
    pub fn apply_output_configs(&mut self, qh: &QueueHandle<Self>, serial: u32) {
        let mgr = match &self.output_manager {
            Some(m) => m,
            None => return,
        };

        let config_obj = mgr.create_configuration(serial, qh, ());

        struct FinalConfig {
            name: String,
            id: ObjectId,
            x: i32,
            y: i32,
            w: i32,
            h: i32,
            scale: f64,
            transform: Transform,
            mode: Option<ZwlrOutputModeV1>,
        }

        let mut calculated: Vec<FinalConfig> = Vec::new();
        let mut cursor_x = 0;
        let mut target_output_name: Option<String> = None;
        let mut startup_focus_found = false;

        info!("-> Calculating multi-monitor independent layout (based on name index)...");

        // --- 第一轮：计算几何数据与名字映射 ---
        for head in &self.heads {
            let name = head.name.clone();
            let cfg = self.config.output.as_ref().and_then(|m| m.get(&name));

            // 初始化 OutputData 时补全 full_area 字段
            self.outputs.entry(name.clone()).or_insert_with(|| {
                info!("[Initialization] New monitor record found: {}", name);
                OutputData {
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
                    },
                    ls_output: None,
                    tags: 1,
                    base_tag: 1,
                }
            });

            if let Some(c) = cfg {
                if c.focus_at_startup.as_deref() == Some("true") {
                    if !startup_focus_found {
                        target_output_name = Some(name.clone());
                        startup_focus_found = true;
                    }
                }
            }

            let scale = cfg
                .and_then(|c| c.scale.as_ref())
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(1.0);
            let (log_w, target_mode) = self.get_output_geometry(head, cfg, scale);
            let transform = Self::parse_transform(cfg);

            let (phys_w, phys_h) = if let Some(m) = &target_mode {
                head.modes
                    .iter()
                    .find(|mi| mi.obj.id() == m.id())
                    .map(|mi| (mi.width, mi.height))
                    .unwrap_or((1920, 1080))
            } else {
                (1920, 1080)
            };

            let log_h = match transform {
                Transform::_90 | Transform::_270 | Transform::Flipped90 | Transform::Flipped270 => {
                    (phys_w as f64 / scale).ceil() as i32
                }
                _ => (phys_h as f64 / scale).ceil() as i32,
            };

            let (x, y) = if let Some(pos) = cfg.and_then(|c| c.position.as_ref()) {
                (pos.x.parse().unwrap_or(0), pos.y.parse().unwrap_or(0))
            } else {
                let x = cursor_x;
                (x, 0)
            };

            calculated.push(FinalConfig {
                name: name.clone(),
                id: head.obj.id(),
                x,
                y,
                w: log_w,
                h: log_h,
                scale,
                transform,
                mode: target_mode,
            });

            cursor_x = cursor_x.max(x + log_w);
        }

        // --- 第二轮：提交物理配置并更新内存 ---
        for res in &calculated {
            if let Some(head_info) = self.heads.iter().find(|h| h.obj.id() == res.id) {
                let head_config = config_obj.enable_head(&head_info.obj, qh, ());
                head_config.set_position(res.x, res.y);
                head_config.set_scale(res.scale);
                head_config.set_transform(res.transform);
                if let Some(m) = &res.mode {
                    head_config.set_mode(m);
                }

                if let Some(out_data) = self.outputs.get_mut(&res.name) {
                    // 只有当旧的 full_area 有效时才计算，防止新插入的显示器出现计算错误
                    let (pad_x, pad_y, pad_w, pad_h) =
                        if out_data.full_area.w > 0 && out_data.full_area.h > 0 {
                            (
                                out_data.usable_area.x - out_data.full_area.x, // 左边距
                                out_data.usable_area.y - out_data.full_area.y, // 上边距
                                out_data.full_area.w - out_data.usable_area.w, // 总宽度差 (左右边距之和)
                                out_data.full_area.h - out_data.usable_area.h, // 总高度差 (上下边距之和)
                            )
                        } else {
                            (0, 0, 0, 0)
                        };

                    // 更新全屏尺寸
                    let new_full = Geometry {
                        x: res.x,
                        y: res.y,
                        w: res.w,
                        h: res.h,
                    };
                    out_data.full_area = new_full;

                    // 将旧的边距应用到新的尺寸上，计算新的 usable_area
                    out_data.usable_area = Geometry {
                        x: new_full.x + pad_x,
                        y: new_full.y + pad_y,
                        w: new_full.w - pad_w,
                        h: new_full.h - pad_h,
                    };
                }
            }
        }

        config_obj.apply();

        if let Some(wm) = &self.river_wm {
            wm.manage_dirty();
        }

        // --- 第三轮：鼠标瞬移排队 ---
        let final_target_name =
            target_output_name.or_else(|| calculated.first().map(|c| c.name.clone()));
        if let Some(t_name) = final_target_name {
            if let Some(target_res) = calculated.iter().find(|c| c.name == t_name) {
                let center_x = target_res.x + (target_res.w / 2);
                let center_y = target_res.y + (target_res.h / 2);
                self.pending_pointer_warp = Some((center_x, center_y));
                self.focused_output = Some(t_name);
            }
        }
    }

    fn get_output_geometry(
        &self,
        head_info: &crate::wm::HeadInfo,
        cfg: Option<&crate::config::OutputConfig>,
        scale: f64,
    ) -> (i32, Option<ZwlrOutputModeV1>) {
        let mut target_mode = None;
        let mut phys_w = 1920;
        let mut phys_h = 1080;

        if let Some(mode_str) = cfg.and_then(|c| c.mode.as_ref()) {
            if let Some((w, h, r)) = Self::parse_mode_string(mode_str) {
                if let Some(m) = head_info
                    .modes
                    .iter()
                    .find(|m| m.width == w && m.height == h && (r == 0 || m.refresh == r))
                {
                    target_mode = Some(m.obj.clone());
                    phys_w = m.width;
                    phys_h = m.height;
                }
            }
        }

        // 如果没配，从当前模式里拿物理宽高
        if target_mode.is_none() {
            if let Some(curr) = &head_info.current_mode {
                if let Some(m) = head_info.modes.iter().find(|m| &m.obj.id() == curr) {
                    phys_w = m.width;
                    phys_h = m.height;
                }
            }
        }

        let transform = Self::parse_transform(cfg);

        // 根据旋转角度对调物理宽高
        let (log_w, _log_h) = match transform {
            Transform::_90 | Transform::_270 | Transform::Flipped90 | Transform::Flipped270 => (
                (phys_h as f64 / scale).ceil() as i32,
                (phys_w as f64 / scale).ceil() as i32,
            ),
            _ => (
                (phys_w as f64 / scale).ceil() as i32,
                (phys_h as f64 / scale).ceil() as i32,
            ),
        };

        (log_w, target_mode)
    }

    /// 辅助：解析 "3840x2160@60.000"
    fn parse_mode_string(s: &str) -> Option<(i32, i32, i32)> {
        let parts: Vec<&str> = s.split('@').collect();
        let res: Vec<&str> = parts[0].split('x').collect();
        if res.len() != 2 {
            return None;
        }

        let w = res[0].trim().parse().ok()?;
        let h = res[1].trim().parse().ok()?;

        let r_mhz = if parts.len() > 1 {
            (parts[1].trim().parse::<f32>().ok().unwrap_or(60.0) * 1000.0) as i32
        } else {
            0
        };
        Some((w, h, r_mhz))
    }

    /// 辅助：解析旋转字符串
    fn parse_transform(cfg: Option<&crate::config::OutputConfig>) -> Transform {
        if let Some(trans_str) = cfg.and_then(|c| c.transform.as_ref()) {
            match trans_str.as_str() {
                "90" => Transform::_90,
                "180" => Transform::_180,
                "270" => Transform::_270,
                "flipped" => Transform::Flipped,
                "flipped-90" => Transform::Flipped90,
                "flipped-180" => Transform::Flipped180,
                "flipped-270" => Transform::Flipped270,
                _ => Transform::Normal,
            }
        } else {
            Transform::Normal
        }
    }
    /// 辅助函数：将 "#RRGGBB" 或 "#RRGGBBAA" 转换为 River 需要的 (r, g, b, a)
    /// River 使用预乘 Alpha (Pre-multiplied Alpha)
    pub fn parse_color(hex: &str) -> (u32, u32, u32, u32) {
        let hex = hex.trim_start_matches('#');
        let (r, g, b, a) = if hex.len() == 6 {
            let r = u32::from_str_radix(&hex[0..2], 16).unwrap_or(0);
            let g = u32::from_str_radix(&hex[2..4], 16).unwrap_or(0);
            let b = u32::from_str_radix(&hex[4..6], 16).unwrap_or(0);
            (r, g, b, 255)
        } else if hex.len() == 8 {
            let r = u32::from_str_radix(&hex[0..2], 16).unwrap_or(0);
            let g = u32::from_str_radix(&hex[2..4], 16).unwrap_or(0);
            let b = u32::from_str_radix(&hex[4..6], 16).unwrap_or(0);
            let a = u32::from_str_radix(&hex[6..8], 16).unwrap_or(255);
            (r, g, b, a)
        } else {
            (0, 0, 0, 255)
        };

        // --- 将 0-255 缩放到 0-0xFFFFFFFF ---
        // River 期待的是完整的 32 位颜色分量
        let r32 = r * 0x01010101;
        let g32 = g * 0x01010101;
        let b32 = b * 0x01010101;
        let a32 = a * 0x01010101;

        // 执行预乘
        let pr = ((r32 as u64 * a32 as u64) / 0xffffffff) as u32;
        let pg = ((g32 as u64 * a32 as u64) / 0xffffffff) as u32;
        let pb = ((b32 as u64 * a32 as u64) / 0xffffffff) as u32;

        (pr, pg, pb, a32)
    }

    /// 辅助：判断是否可以进入 Resize 模式
    fn can_enter_resize_mode(&self) -> bool {
        // 1. 必须有焦点窗口
        let f_id = match &self.focused_window {
            Some(id) => id,
            None => return false,
        };

        // 2. 获取窗口数据
        let w_data = match self.windows.iter().find(|w| w.id == *f_id) {
            Some(w) => w,
            None => return false,
        };

        // 3. 全屏窗口不允许 Resize
        if w_data.is_fullscreen {
            return false;
        }

        // 4. 悬浮窗口：总是允许 (可以调整大小或移动)
        if w_data.is_floating {
            return true;
        }

        // 5. 平铺窗口：只有当数量 > 1 时才允许
        if let Some(out_name) = &w_data.output {
            let mask = self.focused_tags; // 或者用 w_data.tags
            let tiling_count = self
                .windows
                .iter()
                .filter(|w| {
                    w.output.as_ref() == Some(out_name)
                        && (w.tags & mask) != 0
                        && !w.is_floating
                        && !w.is_fullscreen
                })
                .count();

            return tiling_count > 1;
        }

        false
    }

    pub fn perform_action(&mut self, action: Action) {
        match action {
            // --- 切换 Resize 模式 ---
            Action::ToggleResizeMode => {
                if self.is_resize_mode {
                    // 如果已经在模式中，则退出
                    self.is_resize_mode = false;
                    info!("-> [Mode] Exit Resize Mode");
                    if let Some(wm) = &self.river_wm {
                        wm.manage_dirty();
                    }
                } else {
                    // 尝试进入
                    if self.can_enter_resize_mode() {
                        self.is_resize_mode = true;
                        info!("-> [Mode] Enter Resize Mode");
                        if let Some(wm) = &self.river_wm {
                            wm.manage_dirty();
                        }
                    } else {
                        warn!("-> [Mode] Cannot enter Resize Mode: No suitable window focused.");
                    }
                }
            }

            // --- 退出 Resize 模式 ---
            Action::ExitResizeMode => {
                if self.is_resize_mode {
                    self.is_resize_mode = false;
                    info!("-> [Mode] Force Exit Resize Mode");
                    if let Some(wm) = &self.river_wm {
                        wm.manage_dirty();
                    }
                }
            }

            // --- 占位：稍后实现具体的调整逻辑 ---
            Action::Resize(axis, delta) => {
                if self.is_resize_mode {
                    info!(
                        "-> [Resize] Axis: {:?}, Delta: {}px (Pending logic)",
                        axis, delta
                    );
                    // TODO: 实现平铺/悬浮的调整算法
                }
            }

            Action::MoveStep(dir, step) => {
                if self.is_resize_mode {
                    info!(
                        "-> [Resize Move] Dir: {:?}, Step: {}px (Pending logic)",
                        dir, step
                    );
                    // TODO: 实现悬浮窗的移动算法
                }
            }
            // --- 切换悬浮状态 ---
            Action::ToggleFloat => {
                if let Some(f_id) = self.focused_window.clone() {
                    // 1. 获取必要信息：窗口数据、所在显示器、Tag
                    let (mut win_idx, mut out_name_opt, mut win_tags) = (None, None, 0);
                    if let Some(idx) = self.windows.iter().position(|w| w.id == f_id) {
                        win_idx = Some(idx);
                        out_name_opt = self.windows[idx].output.clone();
                        win_tags = self.windows[idx].tags;
                    }

                    if let (Some(idx), Some(out_name)) = (win_idx, out_name_opt) {
                        let is_now_floating = !self.windows[idx].is_floating;
                        self.windows[idx].is_floating = is_now_floating;

                        let tree_key = (out_name.clone(), win_tags);

                        if is_now_floating {
                            // --- Case A: 平铺 -> 悬浮 ---
                            info!("-> [Action] Window {:?} Switch to floating mode", f_id);

                            // 1. 从平铺树中移除 (保持不变)
                            if let Some(root) = self.layout_roots.remove(&tree_key) {
                                if let Some(new_root) = LayoutNode::remove_at(root, &f_id) {
                                    self.layout_roots.insert(tree_key.clone(), new_root);
                                }
                            }

                            // 2. 计算悬浮几何信息 (Geometry)
                            if let Some(out_data) = self.outputs.get(&out_name) {
                                let screen = out_data.usable_area;
                                let w = (screen.w as f32 * 0.6) as i32;
                                let h = (screen.h as f32 * 0.6) as i32;

                                // 基准中心点 (Slot 0 的位置)
                                let base_x = screen.x + (screen.w - w) / 2;
                                let base_y = screen.y + (screen.h - h) / 2;
                                let step = 25;

                                // --- 智能空位查找算法 ---
                                let mut final_slot = 0;

                                // 我们尝试从 0 到 10 个位置
                                for slot in 0..10 {
                                    let test_x = base_x + (slot * step);
                                    let test_y = base_y + (slot * step);

                                    // 检查是否有任何现存的悬浮窗口占用了这个位置
                                    // 排除掉自己 (f_id)
                                    let collision = self.windows.iter().any(|other| {
                                        other.id != f_id
                                            && other.is_floating
                                            && other.output.as_ref() == Some(&out_name)
                                            && (other.tags & win_tags) != 0 // 只有同一 Tag 下的窗口才算作“障碍物”
                                            && (other.float_geo.x - test_x).abs() < 5
                                            && (other.float_geo.y - test_y).abs() < 5
                                    });

                                    if !collision {
                                        final_slot = slot;
                                        break; // 找到空位，停止查找
                                    }
                                }

                                let offset = final_slot * step;

                                // 存入 float_geo
                                self.windows[idx].float_geo = Geometry {
                                    x: base_x + offset,
                                    y: base_y + offset,
                                    w,
                                    h,
                                };
                            }
                        } else {
                            // --- Case B: 悬浮 -> 平铺 ---
                            info!("-> [Action] Window {:?} Switch to Tiling mode", f_id);

                            // 如果树为空，作为根；否则插入到当前焦点历史或随机位置
                            let w_data = self.windows[idx].clone();

                            if !self.layout_roots.contains_key(&tree_key) {
                                self.layout_roots
                                    .insert(tree_key, LayoutNode::Window(w_data));
                            } else if let Some(mut root) = self.layout_roots.remove(&tree_key) {
                                // 尝试插入到某个“参考窗口”旁边（比如最后活跃的平铺窗口
                                let target_id = self
                                    .tag_focus_history
                                    .get(&tree_key)
                                    .cloned()
                                    .unwrap_or(f_id.clone());

                                // 如果 insert_at 返回 false（没找到 target），我们就把 root 和新窗口组成一个新的 Container
                                if !root.insert_at(&target_id, w_data.clone(), SplitType::Vertical)
                                {
                                    // 没找到插入点，强行合并
                                    let new_root = LayoutNode::Container {
                                        split_type: SplitType::Vertical,
                                        ratio: 0.5,
                                        left_child: Box::new(root),
                                        right_child: Box::new(LayoutNode::Window(w_data)),
                                    };
                                    self.layout_roots.insert(tree_key, new_root);
                                } else {
                                    self.layout_roots.insert(tree_key, root);
                                }
                            }
                        }

                        // 强制刷新
                        if let Some(wm) = &self.river_wm {
                            wm.manage_dirty();
                        }
                    }
                }
            }

            // --- 在悬浮和平铺层之间切换焦点 ---
            Action::SwitchFocusFloat => {
                if let Some(f_id) = self.focused_window.clone() {
                    let mut current_is_floating = false;
                    let mut current_out = None;

                    if let Some(w) = self.windows.iter().find(|w| w.id == f_id) {
                        current_is_floating = w.is_floating;
                        current_out = w.output.clone();
                    }

                    if let Some(out_name) = current_out {
                        // 目标：在同屏幕、同 Tag 下，找 is_floating 状态相反的窗口
                        // 优先找：TagFocusHistory 里记录的（最近活跃的）
                        // 其次找：列表里第一个符合条件的

                        let target_is_floating = !current_is_floating;
                        let mask = self.focused_tags;

                        let candidate = self
                            .windows
                            .iter()
                            .filter(|w| {
                                w.output.as_ref() == Some(&out_name) && (w.tags & mask) != 0
                            })
                            .filter(|w| w.is_floating == target_is_floating)
                            .map(|w| w.id.clone())
                            .next(); // 简单起见，取第一个。进阶可以结合历史记录。

                        if let Some(target_id) = candidate {
                            info!(
                                "-> [Focus] Cross-layer switching: {:?} -> {:?}",
                                f_id, target_id
                            );
                            self.focused_window = Some(target_id.clone());

                            // 别忘了告诉 Seat
                            if let Some(seat) = &self.main_seat {
                                if let Some(w_data) =
                                    self.windows.iter().find(|w| w.id == target_id)
                                {
                                    seat.focus_window(&w_data.window);
                                }
                            }

                            if let Some(wm) = &self.river_wm {
                                wm.manage_dirty();
                            }
                        }
                    }
                }
            }
            Action::ToggleFullscreen => {
                if let Some(f_id) = self.focused_window.clone() {
                    if let Some(w) = self.windows.iter_mut().find(|w| w.id == f_id) {
                        // 1. 切换内存状态
                        w.is_fullscreen = !w.is_fullscreen;
                        info!(
                            "-> [Action] Toggle fullscreen state for window {:?}: {}",
                            f_id, w.is_fullscreen
                        );

                        // 2. 告诉 River 我们状态变了，请尽快发起 ManageStart 让我们执行渲染
                        if let Some(wm) = &self.river_wm {
                            wm.manage_dirty();
                        }
                    }
                }
            }
            Action::ReloadConfiguration => {
                info!("-> Reloading configuration manually...");
                self.config = crate::config::Config::load();
                self.needs_reload = true;
                // self.current_keymap = None; // 启动了fcitx5的情况下重载布局会导致崩溃，
                info!("-> The configuration has been reloaded and the new layout will take effect the next time the keyboard is accessed or manually triggered");
            }
            Action::FocusOutput(dir) => self.cycle_output_focus(dir),
            Action::MoveToOutput(dir) => {
                if let Some(f_id) = self.focused_window.clone() {
                    self.move_window_to_output(&f_id, dir);
                }
            }
            // --- 标签切换逻辑 ---
            Action::FocusTag(mask) => {
                // 逻辑：修改“当前活跃显示器”的真值
                if let Some(out_id) = &self.focused_output {
                    if let Some(out_data) = self.outputs.get_mut(out_id) {
                        info!(
                            "-> [Action] Switch the label of monitor {:?} to: {:b}",
                            out_id, mask
                        );
                        out_data.tags = mask;
                        // 同步影子变量，确保本次渲染周期内逻辑一致
                        self.focused_tags = mask;
                    }
                }
                if let Some(wm) = &self.river_wm {
                    wm.manage_dirty();
                }
            }

            // --- 编号移动 (Super+Shift+数字) ---
            Action::MoveToTag(target_mask) => {
                if let Some(f_id) = self.focused_window.clone() {
                    // 固定出现在左边
                    self.move_window_to_tag(&f_id, target_mask, true, MoveHint::Leftmost);
                }
            }
            // --- 方向性移动 (Super+Shift+n/i/u/e) ---
            Action::Move(dir) => {
                if let Some(f_id) = self.focused_window.clone() {
                    self.move_window_locally(&f_id, dir);
                }
            }
            // 直接启动逻辑：更轻量，无 Shell 开销
            Action::Spawn(cmd_list) => {
                if cmd_list.is_empty() {
                    return;
                }
                info!("-> [Spawn] Start process: {:?}", cmd_list);

                std::process::Command::new(&cmd_list[0])
                    .args(&cmd_list[1..])
                    .spawn()
                    .map_err(|e| error!("-> Spawn failed: {}", e))
                    .ok();
            }

            // Shell 启动逻辑：支持环境变量和管道
            Action::Shell(cmd_str) => {
                if cmd_str.is_empty() {
                    return;
                }
                info!("-> [Shell] Execute command: {}", cmd_str);

                std::process::Command::new("sh")
                    .arg("-c")
                    .arg(cmd_str)
                    .spawn()
                    .map_err(|e| error!("-> Shell execution failed: {}", e))
                    .ok();
            }
            Action::Focus(dir) => {
                let mut is_floating_focus = false;
                let mut is_fullscreen_focus = false;
                if let Some(f_id) = &self.focused_window {
                    if let Some(w) = self.windows.iter().find(|w| w.id == *f_id) {
                        if w.is_fullscreen {
                            is_fullscreen_focus = true;
                        } else if w.is_floating {
                            is_floating_focus = true;
                        }
                    }
                }
                if is_fullscreen_focus {
                    // --- 【新增：全屏模式焦点逻辑】 ---
                    // 全屏窗口独占当前 Tag，左右移动直接跨 Tag，上下忽略
                    match dir {
                        Direction::Left => {
                            // 同样需要标记意图，以便下个 Tag 能正确处理边缘查找（如果下个 Tag 没全屏窗的话）
                            self.restrict_focus_to_tiling = true;
                            self.pending_focus_dir = Some(dir);
                            self.cycle_tag(-1, dir);
                        }
                        Direction::Right => {
                            self.restrict_focus_to_tiling = true;
                            self.pending_focus_dir = Some(dir);
                            self.cycle_tag(1, dir);
                        }
                        _ => { /* 全屏状态下，Up/Down 通常不处理，或者留给应用自己处理 */
                        }
                    }
                } else if is_floating_focus {
                    // --- 悬浮模式焦点逻辑 ---
                    self.focus_floating_in_direction(dir);
                } else {
                    // --- 平铺模式焦点逻辑 ---

                    let mut moved_locally = false;
                    if let Some(f_id) = &self.focused_window {
                        if let Some(new_focus) = self.find_neighbor(f_id, dir) {
                            self.focused_window = Some(new_focus.clone());
                            if let Some(out_id) = self
                                .windows
                                .iter()
                                .find(|w| w.id == new_focus)
                                .and_then(|w| w.output.clone())
                            {
                                self.tag_focus_history
                                    .insert((out_id, self.focused_tags), new_focus);
                            }
                            moved_locally = true;
                        }
                    }
                    if !moved_locally {
                        // 只有在本地没动（准备跨 Tag 或撞墙）时，才设置意图
                        self.restrict_focus_to_tiling = true;
                        self.pending_focus_dir = Some(dir);
                        match dir {
                            // 将 dir 传进去，让 cycle_tag 知道是从哪边“撞墙”的
                            Direction::Right => self.cycle_tag(1, dir),
                            Direction::Left => self.cycle_tag(-1, dir),
                            _ => {}
                        }
                    }
                }
            }
            Action::CloseFocused => {
                if let Some(f_id) = &self.focused_window {
                    if let Some(w_data) = self.windows.iter().find(|w| &w.id == f_id) {
                        w_data.window.close();
                    }
                }
            }
        }
    }

    // --- 根据 Tag 查找动态图标 ---
    fn get_dynamic_icon(&self, tag_index: u32) -> Option<String> {
        let mask = 1 << tag_index;
        // 以前端展示为主，基于当前聚焦的显示器来判断
        let out_name = self.focused_output.as_ref()?;
        // 优先找焦点历史记录（用户最后操作过的那个窗口）
        let win_id = self
            .tag_focus_history
            .get(&(out_name.clone(), mask))
            .cloned()
            .or_else(|| {
                // 如果没有历史（比如刚启动），找该 Tag 下任意一个窗口
                self.windows
                    .iter()
                    .find(|w| w.output.as_ref() == Some(out_name) && (w.tags & mask) != 0)
                    .map(|w| w.id.clone())
            });

        let id = win_id?;
        let w = self.windows.iter().find(|w| w.id == id)?;
        let app_id = w.app_id.as_deref()?;

        // 安全获取配置链：config -> window -> rule -> matches
        let rules = self
            .config
            .window
            .as_ref()?
            .rule
            .as_ref()?
            .matches
            .as_ref()?;

        for rule in rules {
            // 忽略大小写
            if app_id.to_lowercase().contains(&rule.appid.to_lowercase()) {
                return Some(rule.icon.clone());
            }
        }

        None
    }
    /// 辅助：统一生成给 Waybar 的状态数据
    fn get_waybar_response_json(&self) -> String {
        let occupied = self.get_occupied_tags();
        let waybar_cfg = self.config.waybar.as_ref();

        let mut tag_strings = Vec::new();

        // 1. 计算显示范围
        let max_occupied_idx = if occupied == 0 {
            0
        } else {
            32 - occupied.leading_zeros() - 1
        };
        let focused_idx = if self.focused_tags == 0 {
            0
        } else {
            32 - self.focused_tags.leading_zeros() - 1
        };
        let visual_bound = (max_occupied_idx.max(focused_idx) + 1).min(31);

        // 2. 循环生成每个标签的样式
        for i in 0..=visual_bound {
            let mask = 1 << i;

            // --- 优先尝试获取动态图标 ---
            let mut icon = self.get_dynamic_icon(i);

            // 如果没有动态规则匹配，回退到 [waybar] tag_icons
            if icon.is_none() {
                icon = waybar_cfg
                    .and_then(|c| c.tag_icons.as_ref())
                    .and_then(|icons| icons.get(i as usize))
                    .cloned();
            }

            // 最后的保底：阿拉伯数字
            let final_icon = icon.unwrap_or_else(|| (i + 1).to_string());

            // --- 确定当前状态对应的样式前缀 ---
            let style_prefix = if (self.focused_tags & mask) != 0 {
                waybar_cfg.and_then(|c| c.focused_style.as_ref())
            } else if (occupied & mask) != 0 {
                waybar_cfg.and_then(|c| c.occupied_style.as_ref())
            } else {
                waybar_cfg.and_then(|c| c.empty_style.as_ref())
            };

            // --- 应用样式 ---
            let styled_icon = match style_prefix {
                Some(prefix) => format!("{}{}</span>", prefix, final_icon),
                None => final_icon,
            };

            tag_strings.push(styled_icon);
        }

        // 3. 构造最终的 Waybar 响应
        let response = WaybarResponse {
            text: tag_strings.join("  "),
            tooltip: format!("Focus: {}", self.get_active_window_title()),
            class: "rrwm-status".to_string(),
        };

        serde_json::to_string(&response).unwrap_or_default()
    }

    /// 核心：处理指令 Socket 连接 (如 rrwm --appid)
    pub fn handle_command_connections(&mut self) {
        if let Some(ref listener) = self.cmd_listener {
            // accept() 是非阻塞的
            while let Ok((mut stream, _)) = listener.accept() {
                // 1. 读取指令
                let mut buf = [0; 1024];
                // 尝试读取，如果客户端连接了但没发数据，这里可能会 WouldBlock。
                // 但对于本地 CLI 工具，通常数据是随连接瞬间到达的。
                // 为了鲁棒性，我们简单尝试读取，读不到就忽略。
                if let Ok(n) = stream.read(&mut buf) {
                    let command = String::from_utf8_lossy(&buf[..n]).trim().to_string();

                    // 2. 路由指令
                    let response = match command.as_str() {
                        "ls_clients" => self.get_app_ids_report(),
                        _ => "Unknown command\n".to_string(),
                    };

                    // 3. 写回响应并关闭连接
                    let _ = stream.write_all(response.as_bytes());
                }
            }
        }
    }

    /// 辅助：生成 AppID 报告字符串
    fn get_app_ids_report(&self) -> String {
        let mut report = String::from("ID\tAppID\t\tTitle/Tag\n");
        report.push_str("--\t-----\t\t---------\n");

        for w in &self.windows {
            let app_id = w.app_id.as_deref().unwrap_or("<Unknown>");
            let id_raw = w.id.protocol_id(); // 获取 Wayland 对象 ID
                                             // 这里我们还可以加上 tags 或者是否全屏等信息，方便调试
            let extra = if w.is_fullscreen { "[Fullscreen]" } else { "" };

            report.push_str(&format!(
                "{}\t{}\t\tTag:{:b} {}\n",
                id_raw, app_id, w.tags, extra
            ));
        }

        if self.windows.is_empty() {
            report.push_str("(No windows)\n");
        }

        report
    }

    /// 核心：处理 IPC 连接
    pub fn handle_ipc_connections(&mut self) {
        if let Some(ref listener) = self.ipc_listener {
            while let Ok((mut stream, _)) = listener.accept() {
                //info!("-> IPC: Discover new listeners (Bar/Script)");
                let mut json = self.get_waybar_response_json();
                json.push('\n');
                let _ = Write::write_all(&mut stream, json.as_bytes());

                self.ipc_clients.push(stream);
            }
        }
    }

    /// 核心：向所有听众广播状态（增加缓存拦截）
    pub fn broadcast_status(&mut self) {
        if self.ipc_clients.is_empty() {
            return;
        }

        let json_content = self.get_waybar_response_json();

        // 【节流】只有内容变化时才真正写入 Socket
        if json_content == self.last_sent_json {
            return;
        }
        self.last_sent_json = json_content.clone();

        let mut packet = json_content;
        packet.push('\n');

        self.ipc_clients
            .retain_mut(|client| std::io::Write::write_all(client, packet.as_bytes()).is_ok());
    }

    /// 计算哪些标签有窗口
    pub fn get_occupied_tags(&self) -> u32 {
        let mut mask = 0u32;
        for w in &self.windows {
            if w.app_id.is_some() {
                mask |= w.tags;
            }
        }
        mask
    }

    /// 获取焦点窗口标题
    pub fn get_active_window_title(&self) -> String {
        if let Some(f_id) = &self.focused_window {
            if let Some(w) = self.windows.iter().find(|w| &w.id == f_id) {
                return w.app_id.clone().unwrap_or_else(|| "Unknown".to_string());
            }
        }
        "".to_string()
    }

    /// 搬迁窗口至新 Tag
    fn move_window_to_tag(
        &mut self,
        win_id: &ObjectId,
        target_mask: u32,
        follow: bool,
        hint: MoveHint,
    ) {
        let (old_tag, out_id) = match self.windows.iter().find(|w| &w.id == win_id) {
            Some(w) => (w.tags, w.output.clone()),
            None => return,
        };
        let out_id = match out_id {
            Some(id) => id,
            None => return,
        };
        if old_tag == target_mask {
            return;
        }

        let old_key = (out_id.clone(), old_tag);
        let new_key = (out_id.clone(), target_mask);

        // 1. 接班人逻辑 (使用 old_key)
        if self.tag_focus_history.get(&old_key) == Some(win_id) {
            let replacement = self
                .windows
                .iter()
                .find(|w| {
                    &w.id != win_id && w.output.as_ref() == Some(&out_id) && (w.tags & old_tag) != 0
                })
                .map(|w| w.id.clone());

            if let Some(rid) = replacement {
                self.tag_focus_history.insert(old_key.clone(), rid); // 【修正】使用 old_key.clone()
            } else {
                self.tag_focus_history.remove(&old_key);
            }
        }

        // 2. 从旧树中移除 (使用 old_key)
        if let Some(root) = self.layout_roots.remove(&old_key) {
            if let Some(new_root) = LayoutNode::remove_at(root, win_id) {
                self.layout_roots.insert(old_key, new_root); // 【修正】这里用 old_key 原件就行
            }
        }

        // 3. 更新窗口数据副本
        let mut win_data_opt = None;
        if let Some(w_info) = self.windows.iter_mut().find(|w| &w.id == win_id) {
            w_info.tags = target_mask;
            win_data_opt = Some(w_info.clone());
        }

        // 4. 插入新树
        if let Some(w_data) = win_data_opt {
            if let Some(old_root) = self.layout_roots.remove(&new_key) {
                // 还原完整的 match 逻辑
                let new_root = match hint {
                    MoveHint::Leftmost => LayoutNode::Container {
                        split_type: SplitType::Vertical,
                        ratio: 0.5,
                        left_child: Box::new(LayoutNode::Window(w_data)),
                        right_child: Box::new(old_root),
                    },
                    MoveHint::Rightmost => LayoutNode::Container {
                        split_type: SplitType::Vertical,
                        ratio: 0.5,
                        left_child: Box::new(old_root),
                        right_child: Box::new(LayoutNode::Window(w_data)),
                    },
                    MoveHint::Topmost => LayoutNode::Container {
                        split_type: SplitType::Horizontal,
                        ratio: 0.5,
                        left_child: Box::new(LayoutNode::Window(w_data)),
                        right_child: Box::new(old_root),
                    },
                    MoveHint::Bottommost => LayoutNode::Container {
                        split_type: SplitType::Horizontal,
                        ratio: 0.5,
                        left_child: Box::new(old_root),
                        right_child: Box::new(LayoutNode::Window(w_data)),
                    },
                };
                // 插入：使用 new_key.clone()
                self.layout_roots.insert(new_key.clone(), new_root);
            } else {
                // 目标 Tag 是空的，直接做根节点
                // 插入：使用 new_key.clone()
                self.layout_roots
                    .insert(new_key.clone(), LayoutNode::Window(w_data));
            }
        }

        // 5. 状态同步
        self.tag_focus_history.insert(new_key, win_id.clone());

        if follow {
            // 我们之前在函数开头已经拿到了 out_id (String 类型)
            if let Some(out_data) = self.outputs.get_mut(&out_id) {
                info!(
                    "-> [Follow] Monitor {} Switch perspective to new tab mask: {:b}",
                    out_id, target_mask
                );
                out_data.tags = target_mask;

                // 同步给影子变量，确保后续渲染和状态栏逻辑一致
                self.focused_tags = target_mask;
            }

            self.focused_window = Some(win_id.clone());
            // 确保当前活跃显示器也是这一个
            self.focused_output = Some(out_id);
        }
        if let Some(wm) = &self.river_wm {
            wm.manage_dirty();
        }
    }

    /// 相对标签移动（向左/向右一个 Tag）（增加动态边界感应）
    fn move_window_relative(&mut self, win_id: &ObjectId, delta: i32, hint: MoveHint) {
        // 1. 获取该窗口所属显示器及其名字
        let out_id = match self
            .windows
            .iter()
            .find(|w| &w.id == win_id)
            .and_then(|w| w.output.clone())
        {
            Some(id) => id,
            None => return,
        };

        // 2. 获取当前显示器的标签状态
        let current_tags = self.outputs.get(&out_id).map(|d| d.tags).unwrap_or(1);
        let current_idx = current_tags.trailing_zeros();

        // 3. 计算该显示器的动态边界
        let occupied = self.get_occupied_tags_for_monitor(&out_id);
        let max_occupied_idx = if occupied == 0 {
            0
        } else {
            32 - occupied.leading_zeros() - 1
        };

        // 边界 = 最远有窗口的 Tag 索引 + 1 (留出一个空位)
        // 限制在 0-31 之间
        let bound_idx = (max_occupied_idx + 1).min(31);

        // 4. 计算目标索引
        let next_idx = if delta > 0 {
            // 向右移：超过边界回到 Tag 1
            if current_idx >= bound_idx {
                0
            } else {
                current_idx + 1
            }
        } else {
            // 向左移：从 Tag 1 跨越则跳到边界空位
            if current_idx == 0 {
                bound_idx
            } else {
                current_idx - 1
            }
        };

        let next_mask = 1 << next_idx;

        // 5. 执行搬迁，且视角跟随 (follow = true)
        info!(
            "-> [Cross-tag transfer] window moved from Tag {} to Tag {}",
            current_idx + 1,
            next_idx + 1
        );
        self.move_window_to_tag(win_id, next_mask, true, hint);
    }

    /// 本地移动：在同一 Tag 内重新排列
    fn move_window_locally(&mut self, win_id: &ObjectId, dir: Direction) {
        let out_id = match self
            .windows
            .iter()
            .find(|w| &w.id == win_id)
            .and_then(|w| w.output.clone())
        {
            Some(id) => id,
            None => return,
        };
        // 1. 尝试在当前方向寻找邻居
        if let Some(neighbor_id) = self.find_neighbor(win_id, dir) {
            info!(
                "-> Discover neighbor {:?} and perform location exchange",
                neighbor_id
            );
            let tree_key = (out_id.clone(), self.focused_tags);
            // 执行树内交换
            if let Some(root) = self.layout_roots.get_mut(&tree_key) {
                LayoutNode::swap_windows(root, win_id, &neighbor_id);
            }
            // 交换后，焦点依然跟着原来的窗口
            self.focused_window = Some(win_id.clone());
            self.tag_focus_history.insert(tree_key, win_id.clone());
        } else {
            // 2. 边界判定：如果水平方向没邻居了，执行跨标签流转（bspwm 风格）
            match dir {
                Direction::Left => {
                    info!("-> The left boundary has been reached, move across tags to the previous Tag");
                    self.move_window_relative(win_id, -1, MoveHint::Rightmost);
                }
                Direction::Right => {
                    info!(
                        "-> The right boundary has been reached, move across tags to the next Tag"
                    );
                    self.move_window_relative(win_id, 1, MoveHint::Leftmost);
                }
                _ => {
                    info!("-> The upper and lower boundaries have been reached, and cross-label processing will not be processed for the time being.");
                }
            }
        }
        // --- 手动触发重新排版 ---
        if let Some(wm) = &self.river_wm {
            wm.manage_dirty();
        }
    }
    /// 获取特定显示器上哪些标签有窗口
    pub fn get_occupied_tags_for_monitor(&self, out_name: &str) -> u32 {
        let mut mask = 0u32;
        for w in &self.windows {
            if w.output.as_deref() == Some(out_name) && w.app_id.is_some() {
                mask |= w.tags;
            }
        }
        mask
    }
    /// 递归查找 BSP 树的物理边缘窗口
    pub fn find_edge_in_tree(node: &LayoutNode, dir: Direction) -> ObjectId {
        match node {
            LayoutNode::Window(w) => w.id.clone(),
            LayoutNode::Container {
                split_type,
                left_child,
                right_child,
                ..
            } => {
                match (split_type, dir) {
                    // 如果是垂直分割，找左边缘就进左儿子，找右边缘就进右儿子
                    (SplitType::Vertical, Direction::Left) => {
                        Self::find_edge_in_tree(left_child, dir)
                    }
                    (SplitType::Vertical, Direction::Right) => {
                        Self::find_edge_in_tree(right_child, dir)
                    }
                    // 如果是水平分割，找上边缘进左(上)儿，找下边缘进右(下)儿
                    (SplitType::Horizontal, Direction::Up) => {
                        Self::find_edge_in_tree(left_child, dir)
                    }
                    (SplitType::Horizontal, Direction::Down) => {
                        Self::find_edge_in_tree(right_child, dir)
                    }
                    // 如果分割方向和我们要找的方向垂直（例如垂直分割时找顶端），
                    // 则两边都算顶端，我们默认进右/下侧（通常是最新激活侧）
                    _ => Self::find_edge_in_tree(right_child, dir),
                }
            }
        }
    }
    /// 智能动态流转：增加方向感知和边缘焦点锁定
    fn cycle_tag(&mut self, delta: i32, dir: Direction) {
        let out_id = match &self.focused_output {
            Some(id) => id.clone(),
            None => return,
        };

        let current_tags = match self.outputs.get(&out_id) {
            Some(d) => d.tags,
            None => return,
        };

        let current_idx = current_tags.trailing_zeros();
        let occupied = self.get_occupied_tags();
        let max_occupied_idx = if occupied == 0 {
            0
        } else {
            32 - occupied.leading_zeros() - 1
        };
        let bound_idx = (max_occupied_idx + 1).min(31);

        let next_idx = if delta > 0 {
            if current_idx >= bound_idx {
                0
            } else {
                current_idx + 1
            }
        } else {
            if current_idx == 0 {
                bound_idx
            } else {
                current_idx - 1
            }
        };

        let next_mask = 1 << next_idx;

        if next_mask != current_tags {
            info!(
                "-> [Transfer] Display {} : Tag {} -> {}",
                out_id,
                current_idx + 1,
                next_idx + 1
            );

            if let Some(out_data) = self.outputs.get_mut(&out_id) {
                out_data.tags = next_mask;
                self.focused_tags = next_mask;
            }

            // --- 基于树的边缘焦点重定向 ---
            if !self.restrict_focus_to_tiling && !self.restrict_focus_to_floating {
                let tree_key = (out_id.clone(), next_mask);
                let edge_win = if let Some(root) = self.layout_roots.get(&tree_key) {
                    let look_dir = if dir == Direction::Right {
                        Direction::Left
                    } else {
                        Direction::Right
                    };
                    Some(Self::find_edge_in_tree(root, look_dir))
                } else {
                    None
                };

                if let Some(win_id) = edge_win {
                    info!("-> [Focus] Auto-lock tiling edge: {:?}", win_id);
                    self.focused_window = Some(win_id.clone());
                    self.tag_focus_history.insert(tree_key, win_id);
                } else {
                    self.focused_window = None;
                }
            } else {
                // 如果有标记，清空当前焦点，交给 ManageStart 的“策略宇宙”去处理
                self.focused_window = None;
            }
        }
    }

    /// 邻居查找
    fn find_neighbor(&self, current_id: &ObjectId, dir: Direction) -> Option<ObjectId> {
        // 1. 先拿到当前聚焦窗口的元数据，确定它属于哪个显示器
        let current_w_data = self.windows.iter().find(|w| &w.id == current_id)?;
        let current_out_name = &current_w_data.output;

        let cur_geo = self.last_geometry.get(current_id)?;

        // 地理围栏：只在同一个显示器内寻找邻居
        self.windows
            .iter()
            .filter(|w| {
                &w.id != current_id
                    && (w.tags & self.focused_tags) != 0
                    && &w.output == current_out_name
            })
            .filter_map(|w| {
                let g = self.last_geometry.get(&w.id)?;

                // 判定是否在方向上
                let is_in_direction = match dir {
                    Direction::Left => g.x + g.w <= cur_geo.x,
                    Direction::Right => g.x >= cur_geo.x + cur_geo.w,
                    Direction::Up => g.y + g.h <= cur_geo.y,
                    Direction::Down => g.y >= cur_geo.y + cur_geo.h,
                };

                if !is_in_direction {
                    return None;
                }

                // 计算投影重叠度
                let overlap = match dir {
                    Direction::Left | Direction::Right => {
                        let over = cur_geo.y.max(g.y) < (cur_geo.y + cur_geo.h).min(g.y + g.h);
                        if over {
                            1000
                        } else {
                            0
                        }
                    }
                    Direction::Up | Direction::Down => {
                        let over = cur_geo.x.max(g.x) < (cur_geo.x + cur_geo.w).min(g.x + g.w);
                        if over {
                            1000
                        } else {
                            0
                        }
                    }
                };

                // 计算边缘距离
                let dist = match dir {
                    Direction::Left => cur_geo.x - (g.x + g.w),
                    Direction::Right => g.x - (cur_geo.x + cur_geo.w),
                    Direction::Up => cur_geo.y - (g.y + g.h),
                    Direction::Down => g.y - (cur_geo.y + cur_geo.h),
                };

                let score = dist - overlap;
                Some((w.id.clone(), score))
            })
            .min_by_key(|&(_, score)| score)
            .map(|(id, _)| id)
    }
}
