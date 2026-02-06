[中文README](README_CN.md)

# Dependencies

* [river](https://codeberg.org/river/river) 0.4.0
* `wev` (Optional: used to query XKB names for specific keys)
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

# Configuration Example



```toml
# ~/.config/river/rrwm.toml

[input.keyboard]
layout = "us"
variant = "colemak" # Defaults to qwerty
options = "caps:swapescape" # Supports multiple options, separated by commas
model = "pc105" # Defaults to pc105
numlock = "true" # Defaults to false

[waybar] # Configure tag icons here; defaults to Arabic numerals
tag_icons = ["", "", "󰃽", "", "", "󰙯", "", "󰎄", "", "", "", "󰊴", "", "", "󰆍", "", "", "", "", "󰘦", "", "󰗨"]
focused_style = "<span color='#bd93f9'>" # Style for focused tags
occupied_style = "<span color='#6C7086'>" # Style for unfocused/occupied tags
empty_style = "<span color='#313244'>" # Style for empty tags

# Use `wev` to find XKB names for specific keys
[keybindings.alt]
# Close focused window
q = { action = "close_focused" }
# Toggle full screen
f = { action = "toggle_fullscreen" }
# Switch focus between different tags
1 = { action = "focus", args = ["1"] }
2 = { action = "focus", args = ["2"] }
# ...
9 = { action = "focus", args = ["9"] }
0 = { action = "focus", args = ["31"] }
# Focus movement (Up/Down/Left/Right), automatically crossing tag boundaries
n = { action = "focus", args = ["left"] }
i = { action = "focus", args = ["right"] }
u = { action = "focus", args = ["up"] }
e = { action = "focus", args = ["down"] }
# Launch applications 
# (spawn has lower overhead, but cannot access user environment variables)
j = { action="shell", cmd='/usr/bin/grim -g "$(/usr/bin/slurp < /dev/null)" - | /usr/bin/satty --filename -' }
a = { action="spawn", args=["/usr/bin/fuzzel", "toggle"] }

# Support for different modifiers
[keybindings.super]
Return = { action = "spawn", args = ["ghostty"] }

# Example of multiple modifiers: Alt + Shift
[keybindings.alt_shift]
# Reload configuration 
# (Hot-plugging monitors might cause waybar overlap; restarting waybar helps)
c = [
    { action = "reload_configuration" },
    { action = "spawn", args = ["pkill", "waybar"] },
    { action = "shell", cmd = "sleep 0.1;waybar &" },
]
# Move window to another tag
1 = { action = "move", args = ["1"] }
# ...
0 = { action = "move", args = ["10"] }
# Move window (Up/Down/Left/Right), automatically crossing tag boundaries
n = { action = "move", args = ["left"] }
i = { action = "move", args = ["right"] }
u = { action = "move", args = ["up"] }
e = { action = "move", args = ["down"] }

# Use quotes if you want to use the plus sign for multiple modifiers
[keybindings."super+shift"]
space = { action = "spawn", args = ["neovide"] }

# Keys without modifiers
[keybindings]
F1 = { action = "shell", cmd = "pactl set-sink-volume @DEFAULT_SINK@ -5%" }
[keybindings.none]
F2 = { action = "shell", cmd = "pactl set-sink-volume @DEFAULT_SINK@ +5%" }

# Switch focus between multiple outputs (monitors)
[keybindings.alt_ctrl]
u = { action = "focus", args = ["up_output"] }
e = { action = "focus", args = ["down_output"] }
n = { action = "focus", args = ["left_output"] }
i = { action = "focus", args = ["right_output"] }

# Move window between multiple outputs
[keybindings.alt_ctrl_shift]
u = { action = "move", args = ["up_output"] }
e = { action = "move", args = ["down_output"] }
n = { action = "move", args = ["left_output"] }
i = { action = "move", args = ["right_output"] }

[output.HDMI-A-1]
focus_at_startup = "true" # Focus this monitor on startup. Ensure only one monitor has this set.
mode = "3840x2160@60.000" # Format: "<width>x<height>" or "<width>x<height>@<refresh rate>"
scale = "2" # Integer or fractional scaling (e.g., "1.25")
transform = "normal" # Valid values: normal, 90, 180, 270, flipped, flipped-90, flipped-180, flipped-270
position={ x="1080", y="0" } # Position in global coordinate space. Default is {x="0", y="0"}.
# Note: Width/Height must be logical (physical pixels / scale). 
# A 4K screen (3840 wide) with 2x scale occupies 1920 logical units.

[output.DP-1]
mode = "1920x1080@60.000"
scale = "1"
transform = "90"
position={ x="0", y="0" }

[window]
smart_borders = "true" # Hide borders/gaps when only one window is present
gaps = "2" # Gap between windows

[window.active] # Borders for focused window
border = { width = "2", color = "#bd93f9" }
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
        └── binds.rs     # Input mapping: Parses config and registers XKB binding objects
```
## License

[MIT](../LICENSE)
