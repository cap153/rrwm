use crate::wm::layout::Geometry;

/// 1. 核心缓动函数 (Cubic Ease-Out)
/// 公式：1 - (1 - progress)^3
/// 效果：起步极快，结尾平滑减速，符合物理惯性。
pub fn ease_out_cubic(progress: f32) -> f32 {
    let p = progress.clamp(0.0, 1.0); // 确保在 0~1 之间
    1.0 - (1.0 - p).powi(3)
}

/// 2. 单个数值的插值计算
/// 根据起始值、结束值和动画进度，计算当前帧应该在的位置
pub fn interpolate(start: i32, end: i32, progress: f32) -> i32 {
    let eased = ease_out_cubic(progress);
    // 加上 0.5 用于四舍五入，防止像素抖动
    start + ((end - start) as f32 * eased).round() as i32
}

/// 3. 完整几何矩形 (Geometry) 的插值计算
pub fn interpolate_geo(start: Geometry, end: Geometry, progress: f32) -> Geometry {
    Geometry {
        x: interpolate(start.x, end.x, progress),
        y: interpolate(start.y, end.y, progress),
        w: interpolate(start.w, end.w, progress),
        h: interpolate(start.h, end.h, progress),
    }
}

/// 4. 核心黑魔法：越界裁剪计算 (Clip Box)
/// 目的：在跨屏动画时，防止窗口的另一半溢出并盖住相邻的显示器。
/// 返回值：(clip_x, clip_y, clip_width, clip_height)
pub fn calculate_clip_box(
    window: Geometry,
    output: Geometry,
    border_width: i32,
) -> (i32, i32, i32, i32) {
    let window_left = window.x;
    let window_right = window.x + window.w;
    let window_top = window.y;
    let window_bottom = window.y + window.h;

    let output_left = output.x;
    let output_right = output.x + output.w;
    let output_top = output.y;
    let output_bottom = output.y + output.h;

    let mut clip_width = window.w;
    let mut clip_height = window.h;
    let mut clip_x = 0;
    let mut clip_y = 0;

    // --- 水平方向裁剪 ---
    if output_left < window_right && output_left > window_left {
        // 窗口左边溢出了显示器左边界，裁剪掉溢出的部分
        clip_x = output_left - window_left;
        clip_width = (window_right - output_left).min(output.w);
    } else if output_right > window_left && output_right < window_right {
        // 窗口右边溢出了显示器右边界
        clip_width = output_right - window_left;
    }

    // --- 垂直方向裁剪 ---
    if output_top < window_bottom && output_top > window_top {
        // 窗口顶部溢出了显示器上边界
        clip_y = output_top - window_top;
        clip_height = window_bottom - output_top;
    } else if output_bottom > window_top && output_bottom < window_bottom {
        // 窗口底部溢出了显示器下边界
        clip_height = output_bottom - window_top;
    }

    // River 的协议要求：clip_box 是相对于窗口内容的，
    // 但是它会同时裁剪边框（Borders）。所以 x 和 y 必须减去 border_width 
    // 才能保证边框不被误伤削掉。
    (
        clip_x - border_width,
        clip_y - border_width,
        clip_width,
        clip_height,
    )
}
