// 1. 窗口管理协议
pub mod river_wm {
    pub extern crate bitflags;
    pub extern crate wayland_backend;
    pub extern crate wayland_client;
    pub use wayland_client::protocol::{wl_output, wl_seat, wl_surface};

    pub mod __interfaces {
        pub use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("./protocols/river-window-management-v1.xml");
    }
    use self::__interfaces::*;
    wayland_scanner::generate_client_code!("./protocols/river-window-management-v1.xml");
}

// 2. 输入管理协议 (处理硬件设备)
pub mod river_input {
    pub extern crate wayland_backend;
    pub extern crate wayland_client;
    pub use wayland_client::protocol::wl_output;

    pub mod __interfaces {
        pub use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("./protocols/river-input-management-v1.xml");
    }
    use self::__interfaces::*;
    wayland_scanner::generate_client_code!("./protocols/river-input-management-v1.xml");
}

// 3. 键盘配置协议 (Colemak 逻辑所在)
pub mod river_xkb_config {
    pub extern crate wayland_backend;
    pub extern crate wayland_client;
    // 引入它依赖的输入设备接口
    pub use super::river_input::river_input_device_v1;

    pub mod __interfaces {
        pub use super::super::river_input::__interfaces::*;
        wayland_scanner::generate_interfaces!("./protocols/river-xkb-config-v1.xml");
    }
    use self::__interfaces::*;
    wayland_scanner::generate_client_code!("./protocols/river-xkb-config-v1.xml");
}

// 4. 快捷键绑定协议
pub mod river_xkb {
    pub extern crate bitflags;
    pub extern crate wayland_backend;
    pub extern crate wayland_client;
    pub use super::river_wm::river_seat_v1;
    pub use wayland_client::protocol::wl_seat;

    pub mod __interfaces {
        pub use super::super::river_wm::__interfaces::*;
        wayland_scanner::generate_interfaces!("./protocols/river-xkb-bindings-v1.xml");
    }
    use self::__interfaces::*;
    wayland_scanner::generate_client_code!("./protocols/river-xkb-bindings-v1.xml");
}

// 层级表面协议 (Waybar, 壁纸等)
pub mod river_layer_shell {
    pub extern crate wayland_backend;
    pub extern crate wayland_client;
    
    // 它依赖窗口管理里的 output 和 seat
    pub use super::river_wm::{river_output_v1, river_seat_v1};

    pub mod __interfaces {
        pub use wayland_client::protocol::__interfaces::*;
        pub use super::super::river_wm::__interfaces::*;
        wayland_scanner::generate_interfaces!("./protocols/river-layer-shell-v1.xml");
    }
    use self::__interfaces::*;
    wayland_scanner::generate_client_code!("./protocols/river-layer-shell-v1.xml");
}

// 6. 显示器硬件管理协议 (wlr-output-management)
pub mod wlr_output_management {
    pub extern crate wayland_backend;
    pub extern crate wayland_client;
    // 引入它依赖的标准输出接口，因为配置显示器时会用到旋转方向 (wl_output.transform)
    pub use wayland_client::protocol::wl_output;

    pub mod __interfaces {
        pub use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("./protocols/wlr-output-management-unstable-v1.xml");
    }
    use self::__interfaces::*;
    wayland_scanner::generate_client_code!("./protocols/wlr-output-management-unstable-v1.xml");
}
