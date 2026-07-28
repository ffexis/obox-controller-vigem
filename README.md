# OBOX Controller ViGEm Driver

**[中文说明](readme_cn.md)**

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
- **Radial 2D deadzone + ADC jitter filter** — replaces per-axis linear deadzone with vector-based radial deadzone (no non-linear diagonal behavior); always-active jitter filter suppresses stick noise
- **Rumble callback** — ViGEmBus vibration notifications forwarded back to the physical gamepad via HID Output Report 0xB3 (dual-motor, pulse mode + timed mode)
- **LED control** — RGB LED, HOME button LED, and consumer area LED (HID Output Report 0xB3)
- **[HidHide](https://github.com/nefarius/HidHide) integration** — auto-registers the app and hides the physical gamepad so only the virtual Xbox 360 is visible to games; idempotent config, conservative cloak-state handling
- **Bluetooth disconnect / auto-reconnect** — detects HID read errors, unplugs the virtual Xbox 360, waits, and reconnects when the controller reappears; also waits at startup if the controller isn't paired yet
- **System tray mode** — runs in background with tray icon, shows connection status/MAC address, LED control menu, deadzone toggle, and Windows notifications for connection events
- **Debug modes** — `--debug-keys` (real-time Col01/Col03 input dump) and `--debug-output` (interactive vibration/LED test menu)
- **CLI subcommands** — `--hidhide-status` / `--hidhide-disable` for inspecting and undoing HidHide configuration

## Limitations

> ⚠️ **Only a single controller is supported at this time.** If you connect multiple OBox gamepads simultaneously, the driver will only process the first detected device.

## Project Structure

```
.
├── src/                        # Rust implementation (main project)
│   ├── main.rs                 # Entry point, session loop, debug modes
│   ├── hidhide.rs              # HidHide CLI integration
│   ├── tray.rs                 # System tray, LED control, notifications
│   └── boxicons-joystick-filled.ico  # Tray icon
├── python/                     # Python implementation (testing/prototyping)
│   ├── obox_middleware.py      # CLI-only driver (no tray)
│   └── test/                   # Protocol exploration and test scripts
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
obox-controller-driver --no-deadzone   Disable joystick deadzone (ADC jitter filter still active)
obox-controller-driver --hidhide-status   Show current HidHide configuration
obox-controller-driver --hidhide-disable  Unhide OBOX from HidHide (keeps global cloak state)
obox-controller-driver --debug-keys       Real-time Col01/Col03 input dump
obox-controller-driver --debug-output     Interactive vibration/LED test menu
obox-controller-driver -h, --help         Show help
```

### Tray Mode

When launched by double-clicking the executable (no console), the driver runs in tray mode:

- **Windows notifications** — shows "Waiting for connection", "Connected successfully!", and "Disconnected"
- **Tray menu** — displays connection status, controller MAC address, LED control options, and deadzone toggle
- **LED control** — RGB status LED (Red/Green/Blue ON/OFF), Consumer area LED (ON/OFF), HOME button LED (ON/OFF)
- **Single instance** — prevents multiple instances from running simultaneously

### CLI Mode

When launched from a terminal (with console), the driver runs in CLI mode with full output logging. Use `--cli` to force CLI mode.

## Python Implementation

The `python/` directory contains a Python port of the driver, primarily used for **protocol testing and rapid prototyping**. It is CLI-only (no system tray, no notifications).

```bash
pip install hidapi vgamepad pynput
python python/obox_middleware.py              # Run driver (CLI mode)
python python/obox_middleware.py --no-deadzone # Disable joystick deadzone
python python/obox_middleware.py --debug-keys  # Debug key input
python python/obox_middleware.py --debug-output # Debug vibration/LED
```

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
- **[hidapi (Python)](https://github.com/trezor/cython-hidapi)** — Python bindings for hidapi (used in Python implementation)
- **[vgamepad](https://github.com/nefarius/vgamepad)** — Python wrapper for ViGEm client (used in Python implementation)

## Contributors

- **TRAE (by ByteDance)** — AI programming assistant

## License

[MIT](LICENSE)
