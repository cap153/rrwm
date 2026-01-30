// 1. 窗口管理协议模块
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

// 2. 快捷键绑定协议模块
pub mod river_xkb {
    pub extern crate bitflags;
    pub extern crate wayland_backend;
    pub extern crate wayland_client;
    
    // 关键：xkb 协议引用了 wm 协议里的 river_seat_v1，所以我们要把它“拉”进来
    pub use super::river_wm::river_seat_v1;
    pub use wayland_client::protocol::wl_seat;

    pub mod __interfaces {
        pub use wayland_client::protocol::__interfaces::*;
        // 接口定义也需要引用对方的接口
        pub use super::super::river_wm::__interfaces::*;
        wayland_scanner::generate_interfaces!("./protocols/river-xkb-bindings-v1.xml");
    }
    use self::__interfaces::*;
    wayland_scanner::generate_client_code!("./protocols/river-xkb-bindings-v1.xml");
}
