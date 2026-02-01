* wlr-randr: 可选，用于获取显示器的硬件信息

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
