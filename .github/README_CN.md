![preview](preview.png) 

# 环境依赖

* [river](https://codeberg.org/river/river) 0.4.0
* wev (可选，查询特定的按键对应的XKB名称)
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

[input.keyboard]
layout = "us"
variant = "colemak" # 默认qwerty
options = "caps:swapescape" # 支持多个选项，用英文逗号隔开
model = "pc105" # 默认pc105
numlock = "true" # 默认关闭小键盘

[waybar] # 这里配置标签的图标，默认阿拉伯数字
tag_icons = ["", "", "󰃽", "", "", "󰙯", "", "󰎄", "", "", "", "󰊴", "", "", "󰆍", "", "", "", "", "󰘦", "", "󰗨"]
focused_style = "<span color='#bd93f9'>" # 聚焦的标签样式
occupied_style = "<span color='#6C7086'>" # 失去焦点的标签样式
empty_style = "<span color='#313244'>" # 空标签样式

# 可以使用wev来查询特定的按键对应的XKB名称
[keybindings.alt]
# 关闭聚焦窗口
q = { action = "close_focused" }
# 切换全屏
f = { action = "toggle_fullscreen" }
# 不同标签之间焦点切换
1 = { action = "focus", args = ["1"] }
2 = { action = "focus", args = ["2"] }
# ...
9 = { action = "focus", args = ["9"] }
0 = { action = "focus", args = ["31"] }
# 上下左右焦点切换，边界自动跨标签
n = { action = "focus", args = ["left"] }
i = { action = "focus", args = ["right"] }
u = { action = "focus", args = ["up"] }
e = { action = "focus", args = ["down"] }
# 启动软件(spawn性能开销小一点，但是无法使用用户环境变量)
j = { action="shell", cmd='/usr/bin/grim -g "$(/usr/bin/slurp < /dev/null)" - | /usr/bin/satty --filename -' }
a = { action="spawn", args=["/usr/bin/fuzzel", "toggle"] }

# 允许不同的修饰符
[keybindings.super]
Return = { action = "spawn", args = ["ghostty"] }

# 多重修饰符示例：Alt + Shift
[keybindings.alt_shift]
# 重载配置(插拔显示器可能会导致waybar重叠，可以尝试重启waybar)
c = [
	{ action = "reload_configuration" },
	{ action = "spawn", args = ["pkill", "waybar"] },
	{ action = "shell", cmd = "sleep 0.1;waybar &" },
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

# 如果想使用加号连接多重修饰符，需要加引号
[keybindings."super+shift"]
space = { action = "spawn", args = ["neovide"] }

# 没有修饰符的按键
[keybindings]
F1 = { action = "shell", cmd = "pactl set-sink-volume @DEFAULT_SINK@ -5%" }
[keybindings.none]
F2 = { action = "shell", cmd = "pactl set-sink-volume @DEFAULT_SINK@ +5%" }

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

[output.HDMI-A-1]
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

[window]
smart_borders = "true" # 只有一个窗口时边框/间隙消失
gaps = "2" # 窗口间隙

[window.active] # 聚焦窗口设置边框
border = { width = "2", color = "#bd93f9" }
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
        └── binds.rs     # 输入映射：解析配置并注册 XKB 绑定对象
```

## 许可证

[MIT](../LICENSE)


