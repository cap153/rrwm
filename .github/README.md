```bash
rrwm
├── Cargo.toml
├── protocols/
└── src/
    ├── main.rs          # 入口：解析命令行、启动连接、初始化循环
    ├── protocol.rs      # 协议生成：放那段 scanner 生成的宏代码
    ├── config.rs        # 配置定义：边框颜色、间隙大小、快捷键列表
    ├── wm/
    │   ├── mod.rs       # 业务中心：AppState 定义和核心 Dispatch 实现
    │   ├── layout.rs    # 平铺引擎：LayoutNode 定义及递归计算算法
    │   └── actions.rs   # 动作执行：处理“切换焦点”、“关闭窗口”的具体代码
    └── binds.rs         # 绑定中心：将快捷键与 actions 关联起来
```
