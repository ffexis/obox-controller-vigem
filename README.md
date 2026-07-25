# OBOX Controller ViGEm Driver

**The world's first complete reverse engineering of the Snail OBox gamepad protocol.**

## Background

The **Snail OBox (蜗牛OBox)** was a Chinese Android-based game console that became a complete commercial failure — the console itself was barely ever sold. However, large quantities of its Bluetooth gamepads flooded the second-hand market, making them cheaply available to enthusiasts.

The gamepad can connect to Windows via Bluetooth (VID `0x0A5C`, PID `0x4502`), but Windows cannot use it properly:

- **LT / RT triggers** do not function correctly
- **SELECT / START** keys lack correct mapping
- **LED control** and **rumble/vibration** are completely inaccessible

This project is the **first and only complete reverse engineering** of the OBox gamepad's HID protocol, documenting every single feature — including LED control (RGB, HOME indicator, consumer area) and dual-motor rumble. Through this driver, you can unlock the gamepad's **full functionality** on Windows.

## Features

- **Col01 Gamepad interface** — buttons, analog sticks (Y-axis inverted to match XInput), analog triggers (LT/RT), D-pad with diagonal support
- **Col03 Consumer interface** — Back / Start / Guide key mapping (0x224→BACK, 0x040→START, 0x0223→GUIDE) via set-difference to handle slot order variations
- **[ViGEmBus](https://github.com/nefarius/ViGEmBus) Xbox 360 virtual gamepad forwarding** — presents a standard Xbox 360 controller to Windows
- **Rumble callback** — ViGEmBus vibration notifications forwarded back to the physical gamepad via HID Output Report 0xB3 (dual-motor, pulse mode + timed mode)
- **LED control** — RGB LED, HOME button LED, and consumer area LED (HID Output Report 0xB3)
- **[HidHide](https://github.com/nefarius/HidHide) integration** — auto-registers the app and hides the physical gamepad so only the virtual Xbox 360 is visible to games; idempotent config, conservative cloak-state handling
- **Bluetooth disconnect / auto-reconnect** — detects HID read errors, unplugs the virtual Xbox 360, waits, and reconnects when the controller reappears; also waits at startup if the controller isn't paired yet
- **System tray mode** — runs in background with tray icon, shows connection status/MAC address, LED control menu, and Windows notifications for connection events
- **Debug modes** — `--debug-keys` (real-time Col01/Col03 input dump) and `--debug-output` (interactive vibration/LED test menu)
- **CLI subcommands** — `--hidhide-status` / `--hidhide-disable` for inspecting and undoing HidHide configuration

## Project Structure

```
.
├── src/                        # Rust implementation (main project)
│   ├── main.rs                 # Entry point, session loop, debug modes
│   ├── hidhide.rs              # HidHide CLI integration
│   ├── tray.rs                 # System tray, LED control, notifications
│   └── boxicons-joystick-filled.ico  # Tray icon
├── docs/                       # Protocol documentation
│   ├── HID_PROTOCOL.md         # English
│   └── HID_PROTOCOL_cn.md      # 中文
├── Cargo.toml
├── build.rs                    # Windows icon embedding
└── LICENSE
```

## Build

```bash
cargo build --release
```

## Run

```bash
cargo run --release
```

> **Prerequisite:** The [ViGEmBus](https://github.com/nefarius/ViGEmBus) driver must be installed.
> [HidHide](https://github.com/nefarius/HidHide) is optional but recommended (auto-configured on startup if present).

## CLI

```
obox-controller-driver                 Run in tray mode (auto when double-clicked)
obox-controller-driver --cli           Run in CLI mode
obox-controller-driver --hidhide-status   Show current HidHide configuration
obox-controller-driver --hidhide-disable  Unhide OBOX from HidHide (keeps global cloak state)
obox-controller-driver --debug-keys       Real-time Col01/Col03 input dump
obox-controller-driver --debug-output     Interactive vibration/LED test menu
obox-controller-driver -h, --help         Show help
```

### Tray Mode

When launched by double-clicking the executable (no console), the driver runs in tray mode:

- **Windows notifications** — shows "Waiting for connection", "Connected successfully!", and "Disconnected"
- **Tray menu** — displays connection status, controller MAC address, and LED control options
- **LED control** — RGB status LED (Red/Green/Blue ON/OFF), Consumer area LED (ON/OFF), HOME button LED (ON/OFF)
- **Single instance** — prevents multiple instances from running simultaneously

### CLI Mode

When launched from a terminal (with console), the driver runs in CLI mode with full output logging. Use `--cli` to force CLI mode.

## Protocol Documentation

The full HID protocol specification is available in [docs/HID_PROTOCOL.md](docs/HID_PROTOCOL.md) (English) and [docs/HID_PROTOCOL_cn.md](docs/HID_PROTOCOL_cn.md) (中文).

## Acknowledgments

This project relies on the following excellent open-source components:

- **[ViGEmBus](https://github.com/nefarius/ViGEmBus)** — Virtual Gamepad Emulation Bus driver by Nefarius Software Solutions e.U.
- **[HidHide](https://github.com/nefarius/HidHide)** — Device hiding solution for gaming input devices by Nefarius Software Solutions e.U.
- **[hidapi-rs](https://github.com/Osspial/hidapi-rs)** — Rust bindings for the hidapi library
- **[vigem-client-rs](https://github.com/timniederhausen/vigem-client-rs)** — Rust bindings for the ViGEm client SDK
- **[tray-icon](https://github.com/tauri-apps/tray-icon)** — Cross-platform system tray icon library
- **[muda](https://github.com/tauri-apps/muda)** — Cross-platform menu library
- **[winit](https://github.com/rust-windowing/winit)** — Cross-platform window creation and management library
- **[windows-rs](https://github.com/microsoft/windows-rs)** — Rust bindings for the Windows API by Microsoft
- **[Boxicons](https://boxicons.com/)** — Beautiful open-source icons (used for tray icon)

## Contributors

- **TRAE (by ByteDance)** — AI programming assistant

## License

[MIT](LICENSE)

---

# OBOX Controller ViGEm Driver

**全球首个对蜗牛 OBox 手柄协议的完整逆向工程。**

## 背景

**蜗牛 OBox（Snail OBox）** 是一台基于 Android 的国产游戏主机，最终在商业上彻底失败——主机本身几乎没有卖出去。然而，大量配套的蓝牙手柄流入二手市场，价格极其低廉，成为了爱好者们的宝藏。

该手柄可以通过蓝牙连接 Windows（VID `0x0A5C`，PID `0x4502`），但 Windows 无法正常使用它：

- **LT / RT 扳机** 无法正常工作
- **SELECT / START** 按键缺少正确映射
- **LED 控制** 和 **振动马达** 完全无法访问

本项目是 **全球首个也是唯一一个** 对 OBox 手柄 HID 协议的完整逆向工程，记录了手柄的全部功能——包括 LED 控制（RGB、HOME 指示灯、消费区）和双马达振动。通过本驱动，你可以在 Windows 上解锁手柄的 **全部功能**。

## 功能

- **Col01 Gamepad 接口** — 按钮、模拟摇杆（Y 轴反转以符合 XInput 规范）、模拟扳机（LT/RT）、支持斜向的 D-pad
- **Col03 Consumer 接口** — Back / Start / Guide 按键映射（0x224→BACK, 0x040→START, 0x0223→GUIDE），使用集合差集检测以应对槽位顺序变化
- **[ViGEmBus](https://github.com/nefarius/ViGEmBus) Xbox 360 虚拟手柄转发** — 向 Windows 呈现标准 Xbox 360 手柄
- **振动回调** — ViGEmBus 振动通知回传至物理手柄（HID Output Report 0xB3，双马达，脉冲模式 + 定时模式）
- **LED 控制** — RGB LED、HOME 键指示灯、消费区 LED（HID Output Report 0xB3）
- **[HidHide](https://github.com/nefarius/HidHide) 集成** — 自动注册本应用并隐藏物理手柄，使游戏只能看到虚拟 Xbox 360；配置幂等，保守处理全局 cloak 状态
- **蓝牙断开 / 自动重连** — 检测 HID 读取错误，断开虚拟 Xbox 360，等待手柄重新出现后自动重连；启动时若手柄未配对也会进入等待状态
- **系统托盘模式** — 后台运行，托盘图标显示连接状态/MAC地址，LED控制菜单，Windows通知提示连接事件
- **调试模式** — `--debug-keys`（实时打印 Col01/Col03 输入）和 `--debug-output`（交互式振动/LED 测试菜单）
- **CLI 子命令** — `--hidhide-status` / `--hidhide-disable` 用于查看和撤销 HidHide 配置

## 项目结构

```
.
├── src/                        # Rust 实现（主项目）
│   ├── main.rs                 # 入口、session 循环、调试模式
│   ├── hidhide.rs              # HidHide CLI 集成
│   ├── tray.rs                 # 系统托盘、LED控制、通知
│   └── boxicons-joystick-filled.ico  # 托盘图标
├── docs/                       # 协议文档
│   ├── HID_PROTOCOL.md         # English
│   └── HID_PROTOCOL_cn.md      # 中文
├── Cargo.toml
├── build.rs                    # Windows 图标嵌入
└── LICENSE
```

## 构建

```bash
cargo build --release
```

## 运行

```bash
cargo run --release
```

> **前提条件：** 需要已安装 [ViGEmBus](https://github.com/nefarius/ViGEmBus) 驱动。
> [HidHide](https://github.com/nefarius/HidHide) 为可选但推荐（启动时如检测到会自动配置）。

## CLI

```
obox-controller-driver                 托盘模式运行（双击自动进入）
obox-controller-driver --cli           CLI模式运行
obox-controller-driver --hidhide-status   查看 HidHide 当前配置
obox-controller-driver --hidhide-disable  从 HidHide 中取消隐藏 OBOX（保留全局 cloak 状态）
obox-controller-driver --debug-keys       实时打印 Col01/Col03 输入
obox-controller-driver --debug-output     交互式振动/LED 测试菜单
obox-controller-driver -h, --help         显示帮助
```

### 托盘模式

双击可执行文件启动（无控制台）时，驱动以托盘模式运行：

- **Windows 通知** — 显示"Waiting for connection"、"Connected successfully!"和"Disconnected"
- **托盘菜单** — 显示连接状态、手柄MAC地址、LED控制选项
- **LED 控制** — RGB状态灯（红/绿/蓝 ON/OFF）、消费区LED（ON/OFF）、HOME键LED（ON/OFF）
- **单例运行** — 防止多个实例同时运行

### CLI 模式

从终端启动（有控制台）时，驱动以 CLI 模式运行，输出完整日志。使用 `--cli` 参数强制进入 CLI 模式。

## 协议文档

完整的 HID 协议规范见 [docs/HID_PROTOCOL.md](docs/HID_PROTOCOL.md)（English）和 [docs/HID_PROTOCOL_cn.md](docs/HID_PROTOCOL_cn.md)（中文）。

## 致谢

本项目依赖以下优秀的开源组件：

- **[ViGEmBus](https://github.com/nefarius/ViGEmBus)** — 虚拟手柄模拟总线驱动，由 Nefarius Software Solutions e.U. 开发
- **[HidHide](https://github.com/nefarius/HidHide)** — 游戏输入设备隐藏方案，由 Nefarius Software Solutions e.U. 开发
- **[hidapi-rs](https://github.com/Osspial/hidapi-rs)** — hidapi 库的 Rust 绑定
- **[vigem-client-rs](https://github.com/timniederhausen/vigem-client-rs)** — ViGEm 客户端 SDK 的 Rust 绑定
- **[tray-icon](https://github.com/tauri-apps/tray-icon)** — 跨平台系统托盘图标库
- **[muda](https://github.com/tauri-apps/muda)** — 跨平台菜单库
- **[winit](https://github.com/rust-windowing/winit)** — 跨平台窗口创建和管理库
- **[windows-rs](https://github.com/microsoft/windows-rs)** — Microsoft 官方的 Windows API Rust 绑定
- **[Boxicons](https://boxicons.com/)** — 精美的开源图标库（用于托盘图标）

## 贡献者

- **TRAE (by ByteDance)** — AI 编程助手

## 许可证

[MIT](LICENSE)
