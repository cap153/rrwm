A window manager based on [river](https://codeberg.org/river/river) >= 0.4.x, written in Rust

![preview](preview.png) 

https://github.com/user-attachments/assets/c711d157-920f-4586-ba03-90476dc7ac16

[中文README](README_CN.md)

[[Video] Feature Overview](https://www.youtube.com/watch?v=of0Jt_hqNO4&list=PLWSurdQkgoVUXJKW5QTtVaP7WXStyLLyy&index=1)

[[Video] v0.1.1 new features](https://www.youtube.com/watch?v=hUQIJ_3N5t8&list=PLWSurdQkgoVUXJKW5QTtVaP7WXStyLLyy&index=2)

# Dependencies

* [river](https://codeberg.org/river/river) 0.4.0
* `wev` (Optional: used to query XKB names for specific keys)
* `libinput-tools` (Optional: used `libinput events` to query standard Names for Mouse Buttons)
* `wlr-randr` (Optional: used to retrieve hardware information for monitors)

# Installation

## AUR

```bash
paru -S rrwm-bin
```

## Build from Source

```bash
git clone https://github.com/cap153/rrwm.git
cd rrwm
cargo build --release
sudo cp target/release/rrwm /usr/local/bin
sudo cp example/rrwm.desktop /usr/local/share/wayland-sessions/
```

# Usage

## Window Manager

```bash
river -c rrwm
```

## Client

```bash
rrwm [OPTIONS]

Options:
  --waybar    Run in Waybar client mode (receive JSON status stream)
  --appid     List all active windows and their AppIDs
  --help      Print this help message
```

# Configuration Example

My configuration is here: [https://github.com/cap153/config/tree/main/river/.config/river](https://github.com/cap153/config/tree/main/river/.config/river) 

```toml
# ~/.config/river/rrwm.toml

[input.keyboard] # Note: This configuration option requires logging out and back in, or restarting, to take effect.
layout = "us"
variant = "colemak" # Default is qwerty
options = "caps:swapescape" # Supports multiple options, separated by commas
model = "pc105" # Default is pc105
numlock = "true" # Default is off

[output.HDMI-A-1] # You can use wlr-randr to get hardware information of connected monitors
focus_at_startup = "true" # Focus this monitor by default on startup. Ensure only one monitor has this option set, otherwise initial focus may be random.
mode = "3840x2160@60.000" # Format: "<width>x<height>" or "<width>x<height>@<refresh rate>". If refresh rate is omitted, the highest available is chosen by default.
scale = "2" # Adjust scale using integers or fractions, e.g., "1.25".
transform = "normal" # Rotate display. Valid values: normal, 90, 180, 270, flipped, flipped-90, flipped-180, and flipped-270.
position={ x="1080", y="0" } # Position of the output in the coordinate space of all monitors. Monitors without an explicit position default to position={ x="0", y="0" }, creating a mirroring effect.
# Note: The width (or height) here must be the logical width (physical pixels / scale factor). For example, a 4K screen (3840 wide) with 2x scaling occupies only 1920 units in the layout space.

[output.DP-1]
mode = "1920x1080@60.000"
scale = "1"
transform = "90"
position={ x="0", y="0" }

[waybar]  # Icons and styles for waybar tags, defaults to Arabic numerals
tag_icons = ["", "", "", "", "󰃽", "󰊢", "", "󰙯", "", "󰐋", "󰕼", "󰎈", "󰎄", "", "", "", "󰊴", "", "", "󰆍", "", "", "", "", "", "󰘦", "", "󰗨", "", "", "", "", ""]
focused_style = "<span color='#bd93f9'>"
occupied_style = "<span color='#6C7086'>"
empty_style = "<span color='#313244'>"

[animations]
enable = "true" # Animation enabled by default
duration = "150" # Animation transition time

[window]
smart_borders = "true" # Borders/gaps disappear when only one window is present
gaps = "2" # Window gaps

[window.active] # Set border for the focused window; width should not exceed the gaps defined in [window]
border = { width = "2", color = "#bd93f9", resize_color = "#ff5555" }

[window.rule] # You can use 'rrwm --appid' List all active windows with their appid and title
match = [
	{ appid="chromium", icon="", width="27%" }, # Split tiled window width/height by ratio, units in % or px
	{ appid="scrcpy", icon="", width="34.31%", height="100%", floating="true" },
	{ appid="zen-browser", icon="" },
	{ appid="zen-browser", title="Peek*", icon="", width="27%" }, # Allow regular matching via title
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


# You can use 'wev' to query the XKB names for specific keys
[keybindings.alt]
minus = { action = "toggle_minimize_restore" }
equal = { action = "toggle_minimize_restore" }
r = { action = "toggle_resize_mode" }
# Toggle focus between floating and tiling windows
space = { action = "switch_focus_between_floating_and_tiling" }
# Close the focused window
q = { action = "close_focused" }
# Toggle fullscreen
f = { action = "toggle_fullscreen" }
# Switch focus between different tags
1 = { action = "focus", args = ["1"] }
# ...
9 = { action = "focus", args = ["9"] }
0 = { action = "focus", args = ["31"] }
# Switch focus Up/Down/Left/Right, automatically crossing tag boundaries
n = { action = "focus", args = ["left"] }
i = { action = "focus", args = ["right"] }
u = { action = "focus", args = ["up"] }
e = { action = "focus", args = ["down"] }
# Launch software
Return = { action = "spawn", args = ["kitty"] }
x = { action="spawn", args=["zen"] }
j = { action="shell", cmd='/usr/bin/grim -t ppm - | /usr/bin/satty --filename - --fullscreen' }
a = { action="spawn", args=["/usr/bin/fuzzel", "toggle"] }
k = [
	{ action="shell", cmd='showkeys' },
	{ action="shell", cmd='systemctl --user stop notify-reg' },
]

# Allow different modifiers
[keybindings.super]
Return = { action = "spawn", args = ["ghostty"] }

# Multi-modifier example: Alt + Shift
[keybindings.alt_shift]
# Toggle the currently focused window between floating/tiling states
space = { action = "toggle_window_floating" }
# Overload configuration
c = { action = "reload_configuration" }
# Exit the current river.
q = [
	{ action="spawn", args=["pkill", "fcitx5"] },
	{ action="shell", cmd="fuser -k ${XDG_RUNTIME_DIR}/${WAYLAND_DISPLAY}" }
]

# Move window across tags
1 = { action = "move", args = ["1"] }
# ...
0 = { action = "move", args = ["10"] }
# Move window Up/Down/Left/Right, automatically crossing tag boundaries
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

# Use quotes if you want to use a plus sign for multiple modifiers
[keybindings."super+shift"]
space = { action = "spawn", args = ["wezterm"] }

# Keys without modifiers
[keybindings]
F1 = { action = "shell", cmd = "pactl set-sink-volume @DEFAULT_SINK@ -5%" }
[keybindings.none] # This has the same effect as [keybindings]
F2 = { action = "shell", cmd = "pactl set-sink-volume @DEFAULT_SINK@ +5%" }
# Volume control, 'wpctl' is included with wireplumber
XF86AudioRaiseVolume = { action = "shell", cmd = "wpctl set-volume @DEFAULT_AUDIO_SINK@ 0.1+" }
XF86AudioLowerVolume = { action = "shell", cmd = "wpctl set-volume @DEFAULT_AUDIO_SINK@ 0.1-" }
XF86AudioMute        = { action = "shell", cmd = "wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle" }
XF86AudioMicMute     = { action = "shell", cmd = "wpctl set-mute @DEFAULT_AUDIO_SOURCE@ toggle" }
# Brightness control
XF86MonBrightnessUp   = { action = "spawn", args = ["brightnessctl", "--class=backlight", "set", "+10%"] }
XF86MonBrightnessDown = { action = "spawn", args = ["brightnessctl", "--class=backlight", "set", "10%-"] }

# Switch focus between multiple monitors
[keybindings.alt_ctrl]
u = { action = "focus", args = ["up_output"] }
e = { action = "focus", args = ["down_output"] }
n = { action = "focus", args = ["left_output"] }
i = { action = "focus", args = ["right_output"] }

# Move windows between multiple monitors
[keybindings.alt_ctrl_shift]
u = { action = "move", args = ["up_output"] }
e = { action = "move", args = ["down_output"] }
n = { action = "move", args = ["left_output"] }
i = { action = "move", args = ["right_output"] }

[resize] # Acitons such as "shrink_width" can also be used in ordinary shortcut keys. The resize mode elegantly isolates two sets of shortcut keys.
# Adjust window size, unit 10px
n = { action = "shrink_width", unit = "10" }
e = { action = "grow_height", unit = "10" }
u = { action = "shrink_height", unit = "10" }
i = { action = "grow_width", unit = "10" }
# Exit resize mode
Return = { action = "exit_resize_mode" }
Escape = { action = "exit_resize_mode" }

[resize.alt]
r = { action = "exit_resize_mode" } # The effect here is the same as "toggle_resize_mode".

[resize.shift]
# Combined shortcut shift+n/e/i/u for minor window size adjustments
n = { action = "shrink_width", unit = "5"}
e = { action = "grow_height", unit = "5"}
u = { action = "shrink_height", unit = "5"}
i = { action = "grow_width", unit = "5"}

[resize.alt_shift] # When unit is carried, the function becomes moving by pixels.
# Combined shortcut alt+shift+n/e/i/u to move window coordinates, unit 5px
n = { action = "move", args = ["left"], unit = "5" }
i = { action = "move", args = ["right"], unit = "5" }
u = { action = "move", args = ["up"], unit = "5" }
e = { action = "move", args = ["down"], unit = "5" }

[pointer.alt] # You can use 'libinput events' to query standard names for mouse buttons
BTN_LEFT = { action = "move_interactive" } # Move by holding Alt + Left Click anywhere on the window
BTN_RIGHT = { action = "resize_interactive" } # Resize by holding Alt + Right Click anywhere on the window
```

# Waybar Integration Example

```json
{
    "modules-left": ["custom/rrwm_tags"],
    "custom/rrwm_tags": {
            "format": "{}",
            "return-type": "json",
            "exec": "rrwm --waybar",
            "escape": false
    },
    // Other waybar configurations...
}

```

# Project Architecture

```bash
rrwm
├── protocols/           # XML protocol files
└── src
    ├── main.rs          # Entry point: IPC listener, Config loader, State initialization
    ├── protocol.rs      # Protocol sandbox: Isolated macro-generated code for different protocols
    ├── config.rs        # Configuration definitions: TOML structures (Serde)
    └── wm
        ├── mod.rs       # Core logic: AppState definition, Dispatch implementations
        ├── layout.rs    # Layout engine: BSP tree, Cosmic tiling algorithm, recursive insertion/deletion
        ├── actions.rs   # Action system: IPC broadcasting, focus finding, cross-tag movement logic
        ├── animation.rs # Animation Calculation Engine
        └── binds.rs     # Input mapping: Parses config and registers XKB binding objects
```
## License

[MIT](../LICENSE)
