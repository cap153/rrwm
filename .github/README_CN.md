![preview](preview.png) 

# 环境依赖

* [river](https://codeberg.org/river/river) 0.4.0
* wev (可选，查询特定的按键对应的XKB名称)
* libinput-tools (可选：使用 `libinput events` 查询鼠标按键的标准名称。)
* wlr-randr (可选，用于获取显示器的硬件信息)

# 安装

## aur

```bash
paru -S rrwm-bin
```

## 编译安装

```bash
git clone https://github.com/cap153/rrwm.git
cd rrwm
cargo build --release
sudo cp target/release/rrwm /usr/local/bin
sudo cp example/rrwm.desktop /usr/local/share/wayland-sessions/
```

# 用法

## 窗口管理器

```bash
river -c rrwm
```

## 客户端

```bash
rrwm [选项]

选项:
  --waybar    以 Waybar 客户端模式运行（接收 JSON 状态流）
  --appid     列出所有活动窗口及其 AppID
  --help      打印此帮助消息
```

# 示例配置

我的配置在这里：[https://github.com/cap153/config/tree/main/river/.config/river](https://github.com/cap153/config/tree/main/river/.config/river) 

```toml
# ~/.config/river/rrwm.toml

[input.keyboard] # 注意：此配置选项需要注销并重新登录或重新启动才能生效。
layout = "us"
variant = "colemak" # 默认qwerty
options = "caps:swapescape" # 支持多个选项，用英文逗号隔开
model = "pc105" # 默认pc105
numlock = "true" # 默认关闭小键盘

[output.HDMI-A-1] # 可以使用 wlr-randr 来获取已连接的显示器的硬件信息
focus_at_startup = "true" # 默认聚焦在这个显示器，确保只有一个显示器配置了这个选项，否则启动时的焦点可能是随机的
mode = "3840x2160@60.000" # 格式为"<width>x<height>" 或者 "<width>x<height>@<refresh rate>"，如果省略了刷新率，默认选择最高的刷新率。
scale = "2" # 整数或分数调整缩放，例如"1.25"。
transform = "normal" # 旋转显示，有效值为:normal, 90, 180, 270, flipped, flipped-90, flipped-180 and flipped-270.
position={ x="1080", y="0" } # 输出在所有显示器坐标空间中的位置。未明确配置位置的显示器将默认position={ x="0", y="0" }实现镜像效果
# 注意：这里的宽度(或高度)必须是逻辑宽度（物理像素 / 缩放比例）。比如一个 4K 屏幕（3840 宽）如果设置了 2 倍缩放，它在排版空间里只占 1920 个单位。

[output.DP-1]
mode = "1920x1080@60.000"
scale = "1"
transform = "90"
position={ x="0", y="0" }

[waybar]  # waybar标签的图标和样式，默认阿拉伯数字
tag_icons = ["", "", "", "", "󰃽", "󰊢", "", "󰙯", "", "󰐋", "󰕼", "󰎈", "󰎄", "", "", "", "󰊴", "", "", "󰆍", "", "", "", "", "", "󰘦", "", "󰗨", "", "", "", "", ""]
focused_style = "<span color='#bd93f9'>"
occupied_style = "<span color='#6C7086'>"
empty_style = "<span color='#313244'>"

[animations]
enable = "true" # 默认开启动画
duration = "150" # 动画的过渡时间

[window]
smart_borders = "true" # 只有一个窗口时边框/间隙消失
gaps = "2" # 窗口间隙

[window.active] # 聚焦窗口设置边框，width不要大于 [window] 中的gaps
border = { width = "2", color = "#bd93f9", resize_color = "#ff5555" }

[window.rule] # rrwm --appid 列出所有活动窗口及其 appid 和 title
match = [
	{ appid="chromium", icon="", width="27%" }, # 平铺窗口宽/高按比例分割，单位百分比或px
	{ appid="scrcpy", icon="", width="34.31%", height="100%", floating="true" },
	{ appid="zen-browser", icon="" },
	{ appid="zen-browser", title="Peek*", icon="", width="27%" }, # 允许通过 title 正则匹配
	{ appid="kitty", icon="󰆍" },
	{ appid="neovide", icon="" },
	{ appid="wechat", icon="" },
	{ appid="lanchat", icon="" },
	{ appid="mpv", icon="󰐋", fullscreen="true", floating="true" },
	{ appid="com.obsproject.Studio", icon="󰃽" },
	{ appid="com.mitchellh.ghostty", icon="" },
	{ appid="org.wezfurlong.wezterm", icon="" },
	{ appid="com.gabm.satty", icon="", fullscreen="true" },
	{ appid="kiro", icon="" }
]


# 可以使用wev来查询特定的按键对应的XKB名称
[keybindings.alt]
minus = { action = "toggle_minimize_restore" }
equal = { action = "toggle_minimize_restore" }
r = { action = "toggle_resize_mode" }
# 在悬浮和平铺窗口之间切换焦点
space = { action = "switch_focus_between_floating_and_tiling" }
# 关闭聚焦窗口
q = { action = "close_focused" }
# 切换全屏
f = { action = "toggle_fullscreen" }
# 不同标签之间焦点切换
1 = { action = "focus", args = ["1"] }
# ...
9 = { action = "focus", args = ["9"] }
0 = { action = "focus", args = ["31"] }
# 上下左右焦点切换，边界自动跨标签
n = { action = "focus", args = ["left"] }
i = { action = "focus", args = ["right"] }
u = { action = "focus", args = ["up"] }
e = { action = "focus", args = ["down"] }
# 启动软件
Return = { action = "spawn", args = ["kitty"] }
x = { action="spawn", args=["zen"] }
j = { action="shell", cmd='/usr/bin/grim -t ppm - | /usr/bin/satty --filename - --fullscreen' }
a = { action="spawn", args=["/usr/bin/fuzzel", "toggle"] }
k = [
	{ action="shell", cmd='showkeys' },
	{ action="shell", cmd='systemctl --user stop notify-reg' },
]

# 允许不同的修饰符
[keybindings.super]
Return = { action = "spawn", args = ["ghostty"] }

# 多重修饰符示例：Alt + Shift
[keybindings.alt_shift]
# 当前聚焦的窗口切换悬浮/平铺状态
space = { action = "toggle_window_floating" }
# 重载配置
c = { action = "reload_configuration" }
# 退出当前rrwm的river
q = [
	{ action="spawn", args=["pkill", "fcitx5"] },
	{ action="shell", cmd="fuser -k ${XDG_RUNTIME_DIR}/${WAYLAND_DISPLAY}" }
]

# 跨标签移动窗口
1 = { action = "move", args = ["1"] }
# ...
0 = { action = "move", args = ["10"] }
# 上下左右移动窗口，边界自动跨标签
n = { action = "move", args = ["left"] }
i = { action = "move", args = ["right"] }
u = { action = "move", args = ["up"] }
e = { action = "move", args = ["down"] }
j = { action="shell", cmd='/usr/bin/grim -g "$(/usr/bin/slurp < /dev/null)" - | /usr/bin/satty --filename -' }
Return = { action = "spawn", args = ["neovide", "--frame", "none"] }
k = [
	{ action = "spawn", args = ["pkill", "showkeys"] },
	{ action="shell", cmd='systemctl --user start notify-reg' },
]

# 如果想使用加号连接多重修饰符，需要加引号
[keybindings."super+shift"]
space = { action = "spawn", args = ["wezterm"] }

# 没有修饰符的按键
[keybindings]
F1 = { action = "shell", cmd = "pactl set-sink-volume @DEFAULT_SINK@ -5%" }
[keybindings.none] # 效果和 [keybindings] 是一样的
F2 = { action = "shell", cmd = "pactl set-sink-volume @DEFAULT_SINK@ +5%" }
# 音量控制，这里的wpctl是wireplumber附带的
XF86AudioRaiseVolume = { action = "shell", cmd = "wpctl set-volume @DEFAULT_AUDIO_SINK@ 0.1+" }
XF86AudioLowerVolume = { action = "shell", cmd = "wpctl set-volume @DEFAULT_AUDIO_SINK@ 0.1-" }
XF86AudioMute        = { action = "shell", cmd = "wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle" }
XF86AudioMicMute     = { action = "shell", cmd = "wpctl set-mute @DEFAULT_AUDIO_SOURCE@ toggle" }
# 亮度控制
XF86MonBrightnessUp   = { action = "spawn", args = ["brightnessctl", "--class=backlight", "set", "+10%"] }
XF86MonBrightnessDown = { action = "spawn", args = ["brightnessctl", "--class=backlight", "set", "10%-"] }

# 多显示器之间切换焦点
[keybindings.alt_ctrl]
u = { action = "focus", args = ["up_output"] }
e = { action = "focus", args = ["down_output"] }
n = { action = "focus", args = ["left_output"] }
i = { action = "focus", args = ["right_output"] }

# 多显示器之间移动窗口
[keybindings.alt_ctrl_shift]
u = { action = "move", args = ["up_output"] }
e = { action = "move", args = ["down_output"] }
n = { action = "move", args = ["left_output"] }
i = { action = "move", args = ["right_output"] }

[resize] # "shrink_width"等aciton也可以在普通快捷键中使用，resize模式优雅的隔离了两组快捷键
# 调整窗口大小，单位10px
n = { action = "shrink_width", unit = "10" }
e = { action = "grow_height", unit = "10" }
u = { action = "shrink_height", unit = "10" }
i = { action = "grow_width", unit = "10" }
# 退出resize模式
Return = { action = "exit_resize_mode" }
Escape = { action = "exit_resize_mode" }

[resize.alt]
r = { action = "exit_resize_mode" } # 这里的效果和toggle_resize_mode是一样的

[resize.shift]
# 组合快捷键 shift+n/e/i/u 小幅度调整窗口大小
n = { action = "shrink_width", unit = "5"}
e = { action = "grow_height", unit = "5"}
u = { action = "shrink_height", unit = "5"}
i = { action = "grow_width", unit = "5"}

[resize.alt_shift] # 携带了unit时功能就变成了按像素移动
# 组合快捷键 alt+shift+n/e/i/u 移动窗口坐标，单位5px
n = { action = "move", args = ["left"], unit = "5" }
i = { action = "move", args = ["right"], unit = "5" }
u = { action = "move", args = ["up"], unit = "5" }
e = { action = "move", args = ["down"], unit = "5" }

[pointer.alt] # 可以使用 libinput events 查询鼠标按键的标准名称
BTN_LEFT = { action = "move_interactive" } # 在窗口任意位置 alt+鼠标左键 按住拖拽移动
BTN_RIGHT = { action = "resize_interactive" } # 在窗口任意位置 alt+鼠标右键 按住拖拽调整窗口大小
```

# Waybar 示例配置

```json
{
	"modules-left": ["custom/rrwm_tags"],
	"custom/rrwm_tags": {
			"format": "{}",
			"return-type": "json",
			"exec": "rrwm --waybar",
			"escape": false
	},
    // 其他的waybar配置。。。
}
```

# 项目结构

```bash
rrwm
├── protocols/           # 存放 XML 协议文件
└── src
    ├── main.rs          # 入口：IPC 监听、Config 加载、状态初始化
    ├── protocol.rs      # 协议沙盒：隔离不同协议的宏生成代码
    ├── config.rs        # 配置定义：TOML 结构体 (Serde)
    └── wm
        ├── mod.rs       # 业务中枢：AppState 定义、所有 Dispatch 实现
        ├── layout.rs    # 布局引擎：BSP 树、Cosmic 切割算法、递归插入/删除
        ├── actions.rs   # 动作系统：IPC 广播、焦点查找、跨标签移动逻辑
        ├── animation.rs # 动画计算引擎
        └── binds.rs     # 输入映射：解析配置并注册 XKB 绑定对象
```

## 许可证

[MIT](../LICENSE)


