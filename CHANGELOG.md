# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.1.1] - 2026-08-22

### Fixed

- **Tray menu lag when disconnected**: driver thread now releases the `connection_status` lock before the 3s reconnection sleep, eliminating lock contention with the UI thread; UI thread additionally uses `try_lock()` as a secondary defense so the message pump can never block
- **Tray icon tooltip**: added a base tooltip on creation (fixes blank tooltip box) and dynamic hover text reflecting connection state ("Connected (MAC)" / "Waiting for controller...")

## [1.1.0] - 2026-08-16

### Added

- **Link Watchdog** — detects a controller powered off without a graceful Bluetooth disconnect in ~1s instead of minutes. Windows keeps a "zombie" HID link alive for minutes on an idle connection; periodically probing the link with a no-op LED report (all zones `0x00`, no visual side effects, dedicated HID handle) forces the baseband to surface a write error as soon as the link is dead, which triggers the normal disconnect path (virtual gamepad unplugged + tray notification)
- **Bounded thread joins (Rust)** — session teardown joins rumble-heartbeat / consumer / watchdog threads with a 2s timeout, so final writes blocked on a dead link can no longer stall the reconnect loop

### Fixed

- **Rust tray mode crashed on double-click launch** (regression in 1.0.4): after `FreeConsole()`, `println!` on the invalid stdout handle panicked and aborted the whole process once a controller connected. Session-thread logging now goes through a panic-free `log_e!` macro that ignores write errors
- **Python: virtual Xbox360 was never unplugged** — `VX360Gamepad` was created once outside the session loop with no teardown; now created per-session and deterministically unplugged (via destructor after dropping the last reference) on every session end/error
- **Python: rumble stop command could block teardown** for seconds on a dead link; now sent from a daemon thread with a 1s bound

## [1.0.4] - 2026-07-28

### Added

- **Radial 2D Deadzone** — replaced per-axis linear deadzone with vector-based radial deadzone algorithm, eliminating non-linear diagonal behavior
- **ADC Jitter Filter** — new `JITTER_TOLERANCE` (64) suppresses sub-threshold stick noise, always active regardless of deadzone setting
- **`--no-deadzone` CLI Flag** — disables deadzone filtering while keeping ADC jitter filter active
- **Tray Menu Deadzone Toggle** — clickable "Deadzone: ON/OFF" item in tray menu for runtime switching (tray mode only)

### Fixed

- CLI mode now waits for physical controller to be confirmed connected before registering virtual Xbox360 gamepad with ViGEmBus (previously registered an idle virtual device when no OBOX was paired)

### Changed

- `ControllerState` now holds `Arc<AtomicBool>` for deadzone flag, enabling runtime toggle in tray mode
- Updated CLI/tray version display to v1.0.4
- Updated help text with new `--no-deadzone` option and deadzone/jitter filter features

## [1.0.3] - 2026-07-26

### Fixed

- Fixed tray status showing "Connected" at startup when controller is not actually connected
- Connection status is now only set to Connected after virtual gamepad is fully established
- LED Control menu is now greyed out (disabled) when controller is not connected

### Changed

- Renamed tray status "Reconnecting..." to "Connecting..."
- Simplified notification text: "Connected successfully!" → "Connected"
- Updated CLI version display to v1.0.3

## [1.0.2] - 2026-07-25

### Fixed

- Fixed critical button mapping error: entire Col01 report parsing was off by one byte (buttons read from wrong offset, hat/sticks/triggers all shifted)
- Button bitmap now correctly read as 16-bit LE from bytes[1-2], hat from byte[3], sticks from bytes[4-11], triggers from bytes[12-15]
- Added missing L3/R3 (stick click) button support

### Changed

- Replaced single-shot rumble forwarding with persistent heartbeat thread model (producer-consumer)
- ViGEmBus notification callback now only updates shared state; heartbeat thread sends pulse commands at ~30ms intervals while active
- Properly bridges XInput state-machine rumble to OBOX pulse-mode hardware requirement
- Long/sustained vibrations now work correctly (previously only a single pulse was sent)

## [1.0.1] - 2026-07-25

### Fixed

- Fixed LED control mapping: HOME LED and Consumer Area LED were swapped in both tray mode and CLI debug mode
- HOME LED now correctly uses byte offset 8-9
- Consumer Area LED now correctly uses byte offset 10-11

## [1.0.0] - 2026-07-25

### Added

- **System Tray Mode** — runs in background with tray icon, shows connection status/MAC address, and LED control menu
- **Windows Notifications** — shows "Waiting for connection", "Connected successfully!", and "Disconnected"
- **LED Control Menu** — RGB status LED (Red/Green/Blue ON/OFF), Consumer Area LED (ON/OFF), HOME button LED (ON/OFF)
- **CLI/Tray Auto-detection** — uses `GetConsoleProcessList` to automatically select mode based on launch method
- **Single Instance Protection** — prevents multiple instances from running simultaneously using named mutex
- **LED Control** — HID Output Report 0xB3 with command 0x01 for LED brightness control

### Changed

- Updated README with tray mode documentation and acknowledgments for open-source components

### Dependencies

- Added `tray-icon`, `winit`, `muda`, `winres`, `windows` crates for system tray and Windows API integration
