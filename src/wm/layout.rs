use crate::protocol::river_wm::river_window_v1::RiverWindowV1;
use crate::wm::WindowData;
use wayland_backend::client::ObjectId;

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
