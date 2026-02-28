use crate::protocol::river_wm::river_window_v1::RiverWindowV1;
use crate::wm::WindowData;
use wayland_backend::client::ObjectId;

// --- 调整轴向枚举 ---
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ResizeAxis {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SplitType {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, Copy)]
pub struct Geometry {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

// --- 递归查找的反馈状态 ---
#[derive(Debug, PartialEq)]
pub enum ResizeResult {
    NotFound,
    FoundNeedResize, // 找到了目标，但当前容器方向不对，请求祖先容器处理
    Handled,         // 已经成功调整
}

pub enum LayoutNode {
    Window(WindowData),
    Container {
        split_type: SplitType,
        ratio: f32,
        left_child: Box<LayoutNode>,
        right_child: Box<LayoutNode>,
    },
}

impl LayoutNode {
    pub fn insert_at(
        &mut self,
        target_id: &ObjectId,
        new_win: WindowData,
        split: SplitType,
    ) -> bool {
        match self {
            LayoutNode::Window(w_data) => {
                if &w_data.id == target_id {
                    let old_win = w_data.clone();
                    *self = LayoutNode::Container {
                        split_type: split,
                        ratio: 0.5,
                        left_child: Box::new(LayoutNode::Window(old_win)),
                        right_child: Box::new(LayoutNode::Window(new_win)),
                    };
                    return true;
                }
                false
            }
            LayoutNode::Container {
                left_child,
                right_child,
                ..
            } => {
                left_child.insert_at(target_id, new_win.clone(), split)
                    || right_child.insert_at(target_id, new_win, split)
            }
        }
    }

    pub fn remove_at(node: LayoutNode, target_id: &ObjectId) -> Option<LayoutNode> {
        match node {
            LayoutNode::Window(w_data) => {
                if &w_data.id == target_id {
                    None
                } else {
                    Some(LayoutNode::Window(w_data))
                }
            }
            LayoutNode::Container {
                split_type,
                ratio,
                left_child,
                right_child,
            } => {
                let new_left = Self::remove_at(*left_child, target_id);
                let new_right = Self::remove_at(*right_child, target_id);
                match (new_left, new_right) {
                    (Some(l), Some(r)) => Some(LayoutNode::Container {
                        split_type,
                        ratio,
                        left_child: Box::new(l),
                        right_child: Box::new(r),
                    }),
                    (None, Some(r)) => Some(r),
                    (Some(l), None) => Some(l),
                    (None, None) => None,
                }
            }
        }
    }

    pub fn swap_windows(node: &mut Self, id1: &ObjectId, id2: &ObjectId) {
        // 1. 先把两个窗口的数据找出来
        fn find_data(n: &LayoutNode, target: &ObjectId) -> Option<WindowData> {
            match n {
                LayoutNode::Window(w) if &w.id == target => Some(w.clone()),
                LayoutNode::Container {
                    left_child,
                    right_child,
                    ..
                } => find_data(left_child, target).or_else(|| find_data(right_child, target)),
                _ => None,
            }
        }

        let d1 = find_data(node, id1);
        let d2 = find_data(node, id2);

        // 2. 如果都找到了，用单次递归进行“同时替换”
        if let (Some(data1), Some(data2)) = (d1, d2) {
            fn perform_swap(
                n: &mut LayoutNode,
                id1: &ObjectId,
                d1: &WindowData,
                id2: &ObjectId,
                d2: &WindowData,
            ) {
                match n {
                    LayoutNode::Window(w) => {
                        if &w.id == id1 {
                            *w = d2.clone(); // 发现 A 的位子，塞入 B 的数据
                        } else if &w.id == id2 {
                            *w = d1.clone(); // 发现 B 的位子，塞入 A 的数据
                        }
                    }
                    LayoutNode::Container {
                        left_child,
                        right_child,
                        ..
                    } => {
                        perform_swap(left_child, id1, d1, id2, d2);
                        perform_swap(right_child, id1, d1, id2, d2);
                    }
                }
            }
            perform_swap(node, id1, &data1, id2, &data2);
        }
    }

    // --- 核心 BSP 树调整算法 ---
    pub fn apply_resize(
        &mut self,
        target_id: &ObjectId,
        area: Geometry,
        axis: ResizeAxis,
        delta: i32,
    ) -> ResizeResult {
        match self {
            LayoutNode::Window(w_data) => {
                // 如果是叶子节点，且是目标窗口，向上报告“我需要调整！”
                if &w_data.id == target_id {
                    ResizeResult::FoundNeedResize
                } else {
                    ResizeResult::NotFound
                }
            }
            LayoutNode::Container {
                split_type,
                ratio,
                left_child,
                right_child,
            } => {
                // 1. 预先计算子区域尺寸，以便后面能算出准确的像素比例
                let (left_area, right_area) = if *split_type == SplitType::Vertical {
                    let left_w = (area.w as f32 * *ratio) as i32;
                    (
                        Geometry { w: left_w, ..area },
                        Geometry {
                            x: area.x + left_w,
                            w: area.w - left_w,
                            ..area
                        },
                    )
                } else {
                    let top_h = (area.h as f32 * *ratio) as i32;
                    (
                        Geometry { h: top_h, ..area },
                        Geometry {
                            y: area.y + top_h,
                            h: area.h - top_h,
                            ..area
                        },
                    )
                };

                // 2. 递归寻找目标，先找左边，再找右边
                let mut res = left_child.apply_resize(target_id, left_area, axis, delta);
                let mut is_left = true;

                if res == ResizeResult::NotFound {
                    res = right_child.apply_resize(target_id, right_area, axis, delta);
                    is_left = false;
                }

                // 3. 如果孩子报告需要调整，说明目标就在这棵子树里
                if res == ResizeResult::FoundNeedResize {
                    // 判断当前容器的切割方向，是否与用户想要调整的轴向匹配？
                    // 水平调整宽度 -> 需要移动垂直分割线
                    // 垂直调整高度 -> 需要移动水平分割线
                    let matches = match (*split_type, axis) {
                        (SplitType::Vertical, ResizeAxis::Horizontal) => true,
                        (SplitType::Horizontal, ResizeAxis::Vertical) => true,
                        _ => false,
                    };

                    if matches {
                        // 匹配成功！当前容器就是“最近的有效分割线”，执行调整。
                        let total_px = if *split_type == SplitType::Vertical {
                            area.w
                        } else {
                            area.h
                        } as f32;

                        if total_px > 0.0 {
                            let delta_ratio = delta as f32 / total_px;

                            // 数学逻辑：
                            // 如果目标在左边(is_left)，增长(+delta)意味着左边变大，ratio 增加。
                            // 如果目标在右边(!is_left)，增长(+delta)意味着右边变大，左边必须缩小，ratio 减少。
                            let mut new_ratio = if is_left {
                                *ratio + delta_ratio
                            } else {
                                *ratio - delta_ratio
                            };

                            // 安全钳制：防止把窗口挤压到 0 导致崩溃或除零错误
                            new_ratio = new_ratio.clamp(0.05, 0.95);
                            *ratio = new_ratio;
                        }
                        return ResizeResult::Handled;
                    } else {
                        // 虽然目标在我这里，但我切错方向了，帮不上忙。
                        // 把“锅”继续甩给上一级祖先容器。
                        return ResizeResult::FoundNeedResize;
                    }
                }

                res
            }
        }
    }
}

pub fn calculate_layout(
    node: &LayoutNode,
    area: Geometry,
    results: &mut Vec<(RiverWindowV1, Geometry)>,
) {
    match node {
        LayoutNode::Window(w_data) => results.push((w_data.window.clone(), area)),
        LayoutNode::Container {
            split_type,
            ratio,
            left_child,
            right_child,
        } => {
            if *split_type == SplitType::Vertical {
                let left_w = (area.w as f32 * ratio) as i32;
                calculate_layout(left_child, Geometry { w: left_w, ..area }, results);
                calculate_layout(
                    right_child,
                    Geometry {
                        x: area.x + left_w,
                        w: area.w - left_w,
                        ..area
                    },
                    results,
                );
            } else {
                let top_h = (area.h as f32 * ratio) as i32;
                calculate_layout(left_child, Geometry { h: top_h, ..area }, results);
                calculate_layout(
                    right_child,
                    Geometry {
                        y: area.y + top_h,
                        h: area.h - top_h,
                        ..area
                    },
                    results,
                );
            }
        }
    }
}
