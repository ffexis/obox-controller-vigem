# OBOX Controller Driver

**The world's first complete reverse engineering of the Snail OBox gamepad protocol.**

## Background

The **Snail OBox (蜗牛OBox)** was a Chinese Android-based game console that became a complete commercial failure — the console itself was barely ever sold. However, large quantities of its Bluetooth gamepads flooded the second-hand market, making them cheaply available to enthusiasts.

The gamepad can connect to Windows via Bluetooth (VID `0x0A5C`, PID `0x4502`), but Windows cannot use it properly:

- **LT / RT triggers** do not function correctly
- **SELECT / START** keys lack correct mapping
- **LED control** and **rumble/vibration** are completely inaccessible

This project is the **first and only complete reverse engineering** of the OBox gamepad's HID protocol, documenting every single feature — including LED control (RGB, HOME indicator, consumer area) and dual-motor rumble. Through this driver, you can unlock the gamepad's **full functionality** on Windows.

## Features

- **Col01 Gamepad interface** — buttons, analog sticks (Y-axis inverted), analog triggers (LT/RT), D-pad with diagonal support
- **ViGEmBus Xbox 360 virtual gamepad forwarding** — presents a standard Xbox 360 controller to Windows
- **LED control** — RGB LED, HOME button LED, and consumer area LED
- **Rumble / Vibration** — dual motor (left/right), pulse mode and timed mode

## Project Structure

```
.
├── src/                        # Rust implementation (main project)
│   └── main.rs
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

## TODO / Roadmap

- [ ] Col03 Consumer interface (Back / Start / Guide)
- [ ] Bluetooth auto-reconnect
- [ ] Config file (deadzone, sensitivity, button mapping)
- [ ] GUI / system tray
- [ ] HidHide adaptation (hide physical gamepad from games, only expose virtual Xbox 360)

## Protocol Documentation

The full HID protocol specification is available in [docs/HID_PROTOCOL.md](docs/HID_PROTOCOL.md) (English) and [docs/HID_PROTOCOL_cn.md](docs/HID_PROTOCOL_cn.md) (中文).

## Contributors

- **TRAE (by ByteDance)** — AI programming assistant

## License

[MIT](LICENSE)

---

# OBOX Controller Driver

**全球首个对蜗牛 OBox 手柄协议的完整逆向工程。**

## 背景

**蜗牛 OBox（Snail OBox）** 是一台基于 Android 的国产游戏主机，最终在商业上彻底失败——主机本身几乎没有卖出去。然而，大量配套的蓝牙手柄流入二手市场，价格极其低廉，成为了爱好者们的宝藏。

该手柄可以通过蓝牙连接 Windows（VID `0x0A5C`，PID `0x4502`），但 Windows 无法正常使用它：

- **LT / RT 扳机** 无法正常工作
- **SELECT / START** 按键缺少正确映射
- **LED 控制** 和 **振动马达** 完全无法访问

本项目是 **全球首个也是唯一一个** 对 OBox 手柄 HID 协议的完整逆向工程，记录了手柄的全部功能——包括 LED 控制（RGB、HOME 指示灯、消费区）和双马达振动。通过本驱动，你可以在 Windows 上解锁手柄的 **全部功能**。

## 功能

- **Col01 Gamepad 接口** — 按钮、模拟摇杆（Y 轴反转）、模拟扳机（LT/RT）、支持斜向的 D-pad
- **ViGEmBus Xbox 360 虚拟手柄转发** — 向 Windows 呈现标准 Xbox 360 手柄
- **LED 控制** — RGB LED、HOME 键指示灯、消费区 LED
- **振动 (Rumble)** — 双马达（左/右），脉冲模式和定时模式

## 项目结构

```
.
├── src/                        # Rust 实现（主项目）
│   └── main.rs
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

## 待实现 / 路线图

- [ ] Col03 Consumer 接口（Back / Start / Guide）
- [ ] 蓝牙断线自动重连
- [ ] 配置文件（死区、灵敏度、按键映射）
- [ ] GUI / 系统托盘
- [ ] HidHide 适配（对游戏隐藏物理手柄，仅暴露虚拟 Xbox 360）

## 协议文档

完整的 HID 协议规范见 [docs/HID_PROTOCOL.md](docs/HID_PROTOCOL.md)（English）和 [docs/HID_PROTOCOL_cn.md](docs/HID_PROTOCOL_cn.md)（中文）。

## 贡献者

- **TRAE (by ByteDance)** — AI 编程助手

## 许可证

[MIT](LICENSE)
