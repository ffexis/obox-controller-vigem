# OBOX Controller Driver

OBOX 蓝牙手柄 (VID=0x0A5C PID=0x4502) -> ViGEmBus Xbox360 中间件。

解决该手柄在 Windows 下不完全兼容的问题。

## 项目结构

```
.
├── src/              # Rust 实现 (主项目)
├── docs/             # 文档
│   └── HID_PROTOCOL.md  # HID 协议详细文档
├── python/           # Python 参考实现 (已 gitignore,不入库)
└── Cargo.toml
```

## Rust 实现 (MVP)

### 功能

- Col01 Gamepad 接口:按钮 / 摇杆(Y 轴反转) / 扳机 / D-pad(斜向支持)
- 转发到 ViGEmBus Xbox360 虚拟手柄

### 待实现

- Col03 Consumer 接口 (Back/Start/Guide)
- 蓝牙断线自动重连
- 配置文件 (死区、灵敏度、按键映射)
- GUI / 托盘图标

### 构建

```
cargo build --release
```

### 运行

```
cargo run --release
```

需要已安装 ViGEmBus 驱动。

## 协议文档

详见 [docs/HID_PROTOCOL.md](docs/HID_PROTOCOL.md)。
