// src/wm/actions.rs
use crate::protocol::wlr_output_management::zwlr_output_mode_v1::ZwlrOutputModeV1;
use crate::wm::layout::{Direction, Geometry, LayoutNode, SplitType};
use crate::wm::AppState;
use serde::Serialize;
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

#[derive(Debug, Clone)]
pub enum Action {
    CloseFocused,
    Focus(Direction),
    FocusTag(u32),           // 切换到某个标签掩码
    MoveToTag(u32),          // 将窗口移动到某个标签掩码
    Move(Direction),         // 统一处理方向性移动
    FocusOutput(Direction),  // 处理 left_output / right_output
    MoveToOutput(Direction), // 处理 left_output / right_output
    Spawn(Vec<String>),      // 纯净启动：[程序名, 参数1, 参数2]
    Shell(String),           // Shell 启动：一整串命令字符串
    ReloadConfiguration,     // 重载配置
}

impl Action {
    /// 核心逻辑：把 TOML 里的字符串配置变成代码里的枚举
    pub fn from_config(name: &str, args: &Option<Vec<String>>, cmd: &Option<String>) -> Self {
        match name.to_lowercase().as_str() {
            // --- 内部指令：关闭窗口 ---
            "close_window" | "close_focused" => Action::CloseFocused,
            // --- 内部指令：重载配置 ---
            "reload_configuration" => Action::ReloadConfiguration,
            // --- 内部指令：焦点切换 ---
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
                    "left" => Action::Move(Direction::Left),
                    "right" => Action::Move(Direction::Right),
                    "up" => Action::Move(Direction::Up),
                    "down" => Action::Move(Direction::Down),
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
                println!("警告：未知的动作名称 {}", name);
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

            // 【核心修正】获取目标显示器当前正在查看的标签
            let next_monitor_tags = next_data.tags;

            println!(
                "-> [跨屏跳转] {} (Tag掩码:{:b}) -> {} (Tag掩码:{:b})",
                current_out, self.focused_tags, next_id, next_monitor_tags
            );

            // 1. 确定“着陆边缘”
            let landing_dir = match dir {
                Direction::Up => Direction::Down,
                Direction::Down => Direction::Up,
                Direction::Left => Direction::Right,
                Direction::Right => Direction::Left,
            };

            // 2. 【核心修正】使用目标显示器自己的标签构造 Key
            let tree_key = (next_id.clone(), next_monitor_tags);

            let edge_win = if let Some(root) = self.layout_roots.get(&tree_key) {
                Some(Self::find_edge_in_tree(root, landing_dir))
            } else {
                None
            };

            // 3. 执行焦点和鼠标瞬移
            if let Some(win_id) = edge_win {
                println!("-> [焦点] 锁定目标屏幕边缘窗口: {:?}", win_id);
                self.focused_window = Some(win_id.clone());
                self.tag_focus_history.insert(tree_key, win_id.clone());

                if let Some(geom) = self.last_geometry.get(&win_id) {
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

            // 4. 更新全局状态：切换当前活跃显示器，并同步影子标签
            self.focused_output = Some(next_id);
            self.focused_tags = next_monitor_tags; // 👈 必须同步这个，否则 Waybar 会显示错误的 Tag

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

            println!(
                "-> [跨屏搬运] 窗口由 {} 搬至 {} (位置: {:?})",
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
    pub fn apply_output_configs(&mut self, qh: &QueueHandle<Self>, serial: u32) {
        let mgr = match &self.output_manager {
            Some(m) => m,
            None => return,
        };

        let config_obj = mgr.create_configuration(serial, qh, ());

        // 存储最终计算结果的临时结构
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

        println!("-> 正在计算多显示器独立排布 (基于名称索引)...");

        // 【关键】每一轮配置开始前，清空旧的 ID 映射，因为 apply 后 ID 会变
        self.output_id_to_name.clear();

        // --- 第一轮：计算几何数据与名字映射 ---
        for head in &self.heads {
            let name = head.name.clone();
            let cfg = self.config.output.as_ref().and_then(|m| m.get(&name));

            // 建立桥梁：让 mod.rs 能通过 ID 找到这个名字
            self.output_id_to_name.insert(head.obj.id(), name.clone());

            // 1. 处理启动焦点配置
            if let Some(c) = cfg {
                if c.focus_at_startup.as_deref() == Some("true") {
                    if !startup_focus_found {
                        target_output_name = Some(name.clone());
                        startup_focus_found = true;
                        println!("   [配置] 发现启动焦点显示器: {}", name);
                    } else {
                        println!(
                            "   警告: 多个显示器配置了 focus-at-startup，将出现焦点随机事件！"
                        );
                    }
                }
            }

            // 2. 初始化显示器的标签 (使用名字查找，解决 E0277)
            if let Some(out_data) = self.outputs.get_mut(&name) {
                out_data.tags = 1;
                out_data.base_tag = 1;
            }

            // 3. 计算几何
            let scale = cfg
                .and_then(|c| c.scale.as_ref())
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(1.0);
            let (log_w, target_mode) = self.get_output_geometry(head, cfg, scale);
            let transform = Self::parse_transform(cfg);

            // 4. 计算逻辑高度 (用于后续计算鼠标中心点)
            // 我们需要拿到物理宽高再除以缩放
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

            // 5. 确定坐标
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

        // --- 第二轮：向 River 提交物理配置并更新内存 ---
        for res in &calculated {
            if let Some(head_info) = self.heads.iter().find(|h| h.obj.id() == res.id) {
                let head_config = config_obj.enable_head(&head_info.obj, qh, ());
                head_config.set_position(res.x, res.y);
                head_config.set_scale(res.scale);
                head_config.set_transform(res.transform);
                if let Some(m) = &res.mode {
                    head_config.set_mode(m);
                }

                // 【修正】使用名字查找更新，确保 usable_area 固化了算好的坐标和对调后的宽高
                if let Some(out_data) = self.outputs.get_mut(&res.name) {
                    out_data.usable_area = Geometry {
                        x: res.x,
                        y: res.y,
                        w: res.w,
                        h: res.h,
                    };
                }
            }
        }

        config_obj.apply();

        // --- 第三轮：准备物理焦点生效 (Pointer Warp) ---
        let final_target_name =
            target_output_name.or_else(|| calculated.first().map(|c| c.name.clone()));

        if let Some(t_name) = final_target_name {
            if let Some(target_res) = calculated.iter().find(|c| c.name == t_name) {
                let center_x = target_res.x + (target_res.w / 2);
                let center_y = target_res.y + (target_res.h / 2);

                println!(
                    "-> [排队] 准备在安全时刻瞬移鼠标至 {} 中心: {},{}",
                    t_name, center_x, center_y
                );
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

        // 【关键修正】根据旋转角度对调物理宽高
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

    pub fn perform_action(&mut self, action: Action) {
        match action {
            Action::ReloadConfiguration => {
                println!("-> 正在手动重载配置...");
                self.config = crate::config::Config::load();
                self.needs_reload = true;
                self.current_keymap = None;
                println!("-> 配置已重载，新的布局将在下次键盘接入或手动触发时生效");
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
                        println!("-> [动作] 切换显示器 {:?} 的标签至: {:b}", out_id, mask);
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
                println!("-> [Spawn] 启动进程: {:?}", cmd_list);

                std::process::Command::new(&cmd_list[0])
                    .args(&cmd_list[1..])
                    .spawn()
                    .map_err(|e| eprintln!("-> Spawn 失败: {}", e))
                    .ok();
            }

            // Shell 启动逻辑：支持环境变量和管道
            Action::Shell(cmd_str) => {
                if cmd_str.is_empty() {
                    return;
                }
                println!("-> [Shell] 执行命令: {}", cmd_str);

                std::process::Command::new("sh")
                    .arg("-c")
                    .arg(cmd_str)
                    .spawn()
                    .map_err(|e| eprintln!("-> Shell 执行失败: {}", e))
                    .ok();
            }
            Action::Focus(dir) => {
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
                    match dir {
                        // 将 dir 传进去，让 cycle_tag 知道是从哪边“撞墙”的
                        Direction::Right => self.cycle_tag(1, dir),
                        Direction::Left => self.cycle_tag(-1, dir),
                        _ => {}
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

    /// 辅助：统一生成给 Waybar 的状态数据
    fn get_waybar_response_json(&self) -> String {
        let occupied = self.get_occupied_tags();
        let user_icons = self
            .config
            .appearance
            .as_ref()
            .and_then(|a| a.tag_icons.as_ref());
        let mut tag_strings = Vec::new();

        // 计算显示范围
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

        for i in 0..=visual_bound {
            let mask = 1 << i;
            let icon = user_icons
                .and_then(|icons| icons.get(i as usize))
                .cloned()
                .unwrap_or_else(|| (i + 1).to_string());

            let styled_icon = if (self.focused_tags & mask) != 0 {
                format!("<span color='#bd93f9'>{}</span>", icon)
            } else if (occupied & mask) != 0 {
                format!("<span color='#6C7086'>{}</span>", icon)
            } else {
                format!("<span color='#313244'>{}</span>", icon)
            };
            tag_strings.push(styled_icon);
        }

        let response = WaybarResponse {
            text: tag_strings.join("  "),
            tooltip: format!("Focus: {}", self.get_active_window_title()),
            class: "rrwm-status".to_string(),
        };

        serde_json::to_string(&response).unwrap_or_default()
    }

    /// 核心：处理 IPC 连接
    pub fn handle_ipc_connections(&mut self) {
        if let Some(ref listener) = self.ipc_listener {
            while let Ok((mut stream, _)) = listener.accept() {
                // println!("-> IPC: 发现新听众 (Bar/Script)");
                // 【修正】新听众进来，立刻发送当前“精装修”后的状态
                let mut json = self.get_waybar_response_json();
                json.push('\n');
                let _ = std::io::Write::write_all(&mut stream, json.as_bytes());

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
                println!(
                    "-> [跟随] 显示器 {} 视角切换至新标签掩码: {:b}",
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
        println!(
            "-> [跨标搬运] 窗口由 Tag {} 移至 Tag {}",
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
            println!("-> 发现邻居 {:?}，执行位置交换", neighbor_id);
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
                    println!("-> 左边界已达，跨标签移动至上一个 Tag");
                    self.move_window_relative(win_id, -1, MoveHint::Rightmost);
                }
                Direction::Right => {
                    println!("-> 右边界已达，跨标签移动至下一个 Tag");
                    self.move_window_relative(win_id, 1, MoveHint::Leftmost);
                }
                _ => {
                    println!("-> 上下边界已达，暂不处理跨标签");
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
    fn find_edge_in_tree(node: &LayoutNode, dir: Direction) -> ObjectId {
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
        let bound_idx = (max_occupied_idx + 1).min(8);

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
            println!(
                "-> [流转] 显示器 {} : Tag {} -> {}",
                out_id,
                current_idx + 1,
                next_idx + 1
            );

            if let Some(out_data) = self.outputs.get_mut(&out_id) {
                out_data.tags = next_mask;
                self.focused_tags = next_mask;
            }

            // --- 基于树的边缘焦点重定向 ---
            let tree_key = (out_id.clone(), next_mask);
            let edge_win = if let Some(root) = self.layout_roots.get(&tree_key) {
                // 如果向右切(Direction::Right)，进入新页面要找【左】边缘
                // 如果向左切(Direction::Left)，进入新页面要找【右】边缘
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
                println!("-> [焦点] 进入新标签，锁定物理边缘窗口: {:?}", win_id);
                self.focused_window = Some(win_id.clone());
                self.tag_focus_history.insert(tree_key, win_id);
            } else {
                self.focused_window = None;
            }
        }
    }

    /// 邻居查找，严格方向判定
    /// 邻居查找：增加显示器隔离判定
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
