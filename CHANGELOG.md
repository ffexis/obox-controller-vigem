# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
