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
- **Debug modes** — `--debug-keys` (real-time Col01/Col03 input dump) and `--debug-output` (interactive vibration/LED test menu)
- **CLI subcommands** — `--hidhide-status` / `--hidhide-disable` for inspecting and undoing HidHide configuration

## Project Structure

```
.
├── src/                        # Rust implementation (main project)
│   ├── main.rs                 # Entry point, session loop, debug modes
│   └── hidhide.rs              # HidHide CLI integration
├── docs/                       # Protocol documentation
│   ├── HID_PROTOCOL.md         # English
│   └── HID_PROTOCOL_cn.md      # 中文
├── Cargo.toml
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
obox-controller-driver                 Run driver (auto-enable HidHide, forward input to ViGEmBus)
obox-controller-driver --hidhide-status   Show current HidHide configuration
obox-controller-driver --hidhide-disable  Unhide OBOX from HidHide (keeps global cloak state)
obox-controller-driver --debug-keys       Real-time Col01/Col03 input dump
obox-controller-driver --debug-output     Interactive vibration/LED test menu
obox-controller-driver -h, --help         Show help
```

## TODO / Roadmap

- [ ] System tray GUI (background running, status indicator, quick settings)

## Protocol Documentation

The full HID protocol specification is available in [docs/HID_PROTOCOL.md](docs/HID_PROTOCOL.md) (English) and [docs/HID_PROTOCOL_cn.md](docs/HID_PROTOCOL_cn.md) (中文).

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
- **调试模式** — `--debug-keys`（实时打印 Col01/Col03 输入）和 `--debug-output`（交互式振动/LED 测试菜单）
- **CLI 子命令** — `--hidhide-status` / `--hidhide-disable` 用于查看和撤销 HidHide 配置

## 项目结构

```
.
├── src/                        # Rust 实现（主项目）
│   ├── main.rs                 # 入口、session 循环、调试模式
│   └── hidhide.rs              # HidHide CLI 集成
├── docs/                       # 协议文档
│   ├── HID_PROTOCOL.md         # English
│   └── HID_PROTOCOL_cn.md      # 中文
├── Cargo.toml
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
obox-controller-driver                 运行驱动（自动配置 HidHide，转发输入到 ViGEmBus）
obox-controller-driver --hidhide-status   查看 HidHide 当前配置
obox-controller-driver --hidhide-disable  从 HidHide 中取消隐藏 OBOX（保留全局 cloak 状态）
obox-controller-driver --debug-keys       实时打印 Col01/Col03 输入
obox-controller-driver --debug-output     交互式振动/LED 测试菜单
obox-controller-driver -h, --help         显示帮助
```

## 待实现 / 路线图

- [ ] 系统托盘 GUI（后台运行、状态指示、快捷设置）

## 协议文档

完整的 HID 协议规范见 [docs/HID_PROTOCOL.md](docs/HID_PROTOCOL.md)（English）和 [docs/HID_PROTOCOL_cn.md](docs/HID_PROTOCOL_cn.md)（中文）。

## 贡献者

- **TRAE (by ByteDance)** — AI 编程助手

## 许可证

[MIT](LICENSE)
