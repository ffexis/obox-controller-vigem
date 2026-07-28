# OBOX Controller ViGEm Driver

**[English README](README.md)**

**全球首个对蜗牛 OBox 手柄协议的完整逆向工程。**

## 背景

**蜗牛 OBox（Snail OBox）** 是一台基于 Android 的国产游戏主机，最终在商业上彻底失败——主机本身几乎没有卖出去。然而，大量配套的蓝牙手柄流入二手市场，价格极其低廉，成为了爱好者们的宝藏。

该手柄可以通过蓝牙连接 Windows（VID `0x0A5C`，PID `0x4502`），但 Windows 无法正常使用它：

- **LT / RT 扳机** 无法正常工作
- **SELECT / START** 按键缺少正确映射
- **LED 控制** 和 **振动马达** 完全无法访问

本项目是 **全球首个也是唯一一个** 对 OBox 手柄 HID 协议的完整逆向工程，记录了手柄的全部功能——包括 LED 控制（RGB、HOME 指示灯、消费区）和双马达振动。通过本驱动，你可以在 Windows 上解锁手柄的 **全部功能**。

## 硬件方案

手柄主控为 **Broadcom BCM20733** 蓝牙 SoC（芯片丝印 `BCM20733A3KFB2G`），经实物拆机确认——属单芯片设计，集成蓝牙射频、基带与 MCU，主板背面另有一颗电源管理芯片。

> **关于 VID/PID 的说明：** 蓝牙标识 `0x0A5C` / `0x4502` 是 Broadcom 的*默认*厂商/产品 ID，**无法**据此确定具体芯片型号。此前仅凭 VID/PID 推测主控为 BCM20702 是错误的，实际主控为 BCM20733。

## 功能

- **Col01 Gamepad 接口** — 按钮、模拟摇杆（Y 轴反转以符合 XInput 规范）、模拟扳机（LT/RT）、支持斜向的 D-pad
- **Col03 Consumer 接口** — Back / Start / Guide 按键映射（0x224→BACK, 0x040→START, 0x0223→GUIDE），使用集合差集检测以应对槽位顺序变化
- **[ViGEmBus](https://github.com/nefarius/ViGEmBus) Xbox 360 虚拟手柄转发** — 向 Windows 呈现标准 Xbox 360 手柄
- **径向 2D 死区 + ADC 抖动过滤** — 替换原有按轴线性死区，对角线方向不再非线性；始终启用抖动过滤抑制摇杆噪声
- **振动回调** — ViGEmBus 振动通知回传至物理手柄（HID Output Report 0xB3，双马达，脉冲模式 + 定时模式）
- **LED 控制** — RGB LED、HOME 键指示灯、消费区 LED（HID Output Report 0xB3）
- **[HidHide](https://github.com/nefarius/HidHide) 集成** — 自动注册本应用并隐藏物理手柄，使游戏只能看到虚拟 Xbox 360；配置幂等，保守处理全局 cloak 状态
- **蓝牙断开 / 自动重连** — 检测 HID 读取错误，断开虚拟 Xbox 360，等待手柄重新出现后自动重连；启动时若手柄未配对也会进入等待状态
- **系统托盘模式** — 后台运行，托盘图标显示连接状态/MAC地址，LED控制菜单，Windows通知提示连接事件，托盘菜单支持运行时切换摇杆死区开关
- **调试模式** — `--debug-keys`（实时打印 Col01/Col03 输入）和 `--debug-output`（交互式振动/LED 测试菜单）
- **CLI 子命令** — `--hidhide-status` / `--hidhide-disable` 用于查看和撤销 HidHide 配置

## 限制说明

> ⚠️ **目前只支持单一手柄连接。** 如果你同时连接多个 OBox 手柄，驱动只会处理第一个检测到的设备。

## 项目结构

```
.
├── src/                        # Rust 实现（主项目）
│   ├── main.rs                 # 入口、session 循环、调试模式
│   ├── hidhide.rs              # HidHide CLI 集成
│   ├── tray.rs                 # 系统托盘、LED控制、通知
│   └── boxicons-joystick-filled.ico  # 托盘图标
├── python/                     # Python 实现（测试/原型验证）
│   ├── obox_middleware.py      # 纯 CLI 驱动（无托盘）
│   └── test/                   # 协议探索和测试脚本
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
obox-controller-driver --no-deadzone   禁用摇杆死区（ADC抖动过滤仍然生效）
obox-controller-driver --hidhide-status   查看 HidHide 当前配置
obox-controller-driver --hidhide-disable  从 HidHide 中取消隐藏 OBOX（保留全局 cloak 状态）
obox-controller-driver --debug-keys       实时打印 Col01/Col03 输入
obox-controller-driver --debug-output     交互式振动/LED 测试菜单
obox-controller-driver -h, --help         显示帮助
```

### 托盘模式

双击可执行文件启动（无控制台）时，驱动以托盘模式运行：

- **Windows 通知** — 显示"Waiting for connection"、"Connected successfully!"和"Disconnected"
- **托盘菜单** — 显示连接状态、手柄MAC地址、LED控制选项，支持死区开关
- **LED 控制** — RGB状态灯（红/绿/蓝 ON/OFF）、消费区LED（ON/OFF）、HOME键LED（ON/OFF）
- **单例运行** — 防止多个实例同时运行

### CLI 模式

从终端启动（有控制台）时，驱动以 CLI 模式运行，输出完整日志。使用 `--cli` 参数强制进入 CLI 模式。

## Python 实现

`python/` 目录包含驱动的 Python 移植版，主要用于**协议测试和快速原型验证**。纯 CLI 模式，无系统托盘和通知功能。

```bash
pip install hidapi vgamepad pynput
python python/obox_middleware.py              # 运行驱动（CLI 模式）
python python/obox_middleware.py --no-deadzone # 禁用摇杆死区
python python/obox_middleware.py --debug-keys  # 调试按键输入
python python/obox_middleware.py --debug-output # 调试振动/LED
```

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
- **[hidapi (Python)](https://github.com/trezor/cython-hidapi)** — hidapi 的 Python 绑定（Python 实现使用）
- **[vgamepad](https://github.com/nefarius/vgamepad)** — ViGEm 客户端的 Python 封装（Python 实现使用）

## 贡献者

- **TRAE (by ByteDance)** — AI 编程助手

## 许可证

[MIT](LICENSE)
