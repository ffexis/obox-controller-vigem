# OBOX Bluetooth Gamepad HID Protocol Documentation

> VID = `0x0A5C`  PID = `0x4502` (Broadcom default IDs — see Hardware note below)
> Connection: Bluetooth HID over L2CAP
> Purpose: Reference for implementing ViGEmBus middleware in any language
>
> **Hardware (confirmed by teardown):** The gamepad's main controller is the
> **Broadcom BCM20733** Bluetooth SoC (die marking `BCM20733A3KFB2G`), a
> single-chip solution that integrates the Bluetooth radio, baseband and MCU.
> The only other notable IC on the board is a power-management chip on the
> reverse side. The product is therefore a *single-SoC Bluetooth HID* design.
> Note: VID `0x0A5C` / PID `0x4502` are Broadcom's **default** identifiers and
> do **not** reveal the exact chip — the earlier "BCM20702" guess derived from
> VID/PID alone was incorrect.

---

## 1. HID Interface Enumeration

The device exposes **4 HID collections** under a single Bluetooth HID connection:

| Collection | Usage Page | Usage | Description |
|------------|-----------|-------|-------------|
| Col01 | Generic Desktop (0x01) | Gamepad (0x05) | Main gamepad input + LED/rumble output |
| Col02 | Generic Desktop (0x01) | Mouse (0x02) | **Unused** — no reports observed |
| Col03 | Consumer (0x0C) | Consumer Control (0x01) | System keys (Back, Menu, Home) |
| Col04 | Keyboard (0x01) | Keyboard (0x06) | PrintScreen key only |

Each collection has its own Report ID and must be opened independently via HIDAPI.

---

## 2. Col01 Gamepad Interface (Report ID `0x07`)

### Input Report — 16 bytes

```
Byte Offset  Size    Field
───────────  ──────  ─────────────────────────────────
0            1       Report ID (0x07)
1-2          2       Buttons (16-bit bitmap, little-endian)
3            1       Hat Switch (lower 4 bits)
4-5          2       LX — Left Stick X (uint16, little-endian)
6-7          2       LY — Left Stick Y (uint16, little-endian)
8-9          2       RX — Right Stick X (uint16, little-endian)
10-11        2       RY — Right Stick Y (uint16, little-endian)
12-13        2       L2 — Left Trigger (uint16, little-endian)
14-15        2       R2 — Right Trigger (uint16, little-endian)
```

### Button Mapping (16-bit bitmap)

| Bit | Xbox 360 Button | Notes |
|-----|----------------|-------|
| B0 (bit 0) | A | |
| B1 (bit 1) | B | |
| B2 (bit 2) | — | Unused |
| B3 (bit 3) | X | |
| B4 (bit 4) | Y | |
| B5 (bit 5) | — | Unused |
| B6 (bit 6) | LB (Left Bumper) | |
| B7 (bit 7) | RB (Right Bumper) | |
| B8 (bit 8) | — | Unused |
| B9 (bit 9) | — | Unused |
| B10 (bit 10) | — | Unused |
| B11 (bit 11) | — | Unused |
| B12 (bit 12) | — | Unused |
| B13 (bit 13) | L3 (Left Stick Click) | |
| B14 (bit 14) | R3 (Right Stick Click) | |
| B15 (bit 15) | — | Unused |

### Hat Switch (D-Pad)

The lower 4 bits of byte[3] encode the hat switch using standard HID clock positions:

| Value | Direction |
|-------|-----------|
| 0 | Up |
| 1 | Up-Right |
| 2 | Right |
| 3 | Down-Right |
| 4 | Down |
| 5 | Down-Left |
| 6 | Left |
| 7 | Up-Left |
| 8 | Centered (released) |

### Analog Sticks

- Range: `0x0000` – `0xFFFF` (0 – 65535)
- Center: `32768` (0x8000)
- **Y-axis inversion required for XInput**: The device reports Y with up = high value. XInput expects up = low value. Apply: `y_out = 65535 - y_in`

### Triggers (L2 / R2)

- Range: `0x0000` – `0xFFFF` (0 – 65535)
- For Xbox 360 emulation, compress to 8-bit: `trigger_out = value >> 8` (yields 0–255)
- Apply threshold before output: if `value < TRIGGER_THRESHOLD`, output `0`

---

## 3. Col03 Consumer Interface (Report ID `0x0A`)

### Input Report — 7 bytes

```
Byte Offset  Size    Field
───────────  ──────  ─────────────────────────────────
0            1       Report ID (0x0A)
1-2          2       Usage Slot 1 (uint16, little-endian)
3-4          2       Usage Slot 2 (uint16, little-endian)
5-6          2       Usage Slot 3 (uint16, little-endian)
```

### Consumer Usage Codes

| Usage Code | Meaning | Xbox 360 Mapping |
|-----------|---------|-----------------|
| `0x0224` | AC Back | Back |
| `0x0040` | Menu | Start |
| `0x0223` | AC Home | Guide (Home) |

### State Change Detection

The consumer report uses a **set-based** model:

- When a key is pressed, its usage code appears in one of the 3 slots.
- When released, the slot is set to `0x0000`.
- Multiple keys can be held simultaneously (up to 3 slots).
- **Do not** treat the report as a snapshot of all keys — detect transitions by comparing against the previous report's slot contents.

```python
# Pseudocode for state detection
prev_usages = {0x0000, 0x0000, 0x0000}

def on_consumer_report(data):
    curr_usages = {data[1:3], data[3:5], data[5:7]}
    pressed  = curr_usages - prev_usages   # newly appeared
    released = prev_usages - curr_usages   # newly disappeared
    prev_usages = curr_usages
```

### Windows Home Key Issue

On Windows, `0x0223` (AC Home) triggers the default browser launch via the system's consumer control handler.

**How to suppress:**

- **With HidHide** — the physical device is hidden from the OS, so the consumer key is never processed. No extra action needed.
- **Without HidHide** — use AutoHotKey with a single-line script:
  ```ahk
  Browser_Home::return
  ```

---

## 4. Col04 Keyboard Interface

### Exclusive Access Problem

The Windows keyboard class driver (`kbdclass.sys`) **exclusively claims** the keyboard HID collection. Standard HIDAPI `hid_read()` calls will fail or return no data because the OS owns the read handle.

### Solution

Use a **global keyboard hook** (`SetWindowsHookEx` with `WH_KEYBOARD_LL`) to intercept the PrintScreen key:

```c
HHOOK hHook = SetWindowsHookEx(WH_KEYBOARD_LL, KeyboardProc, NULL, 0);

LRESULT CALLBACK KeyboardProc(int nCode, WPARAM wParam, LPARAM lParam) {
    KBDLLHOOKSTRUCT *pKey = (KBDLLHOOKSTRUCT *)lParam;
    if (pKey->vkCode == VK_SNAPSHOT) {  // PrintScreen = 0x2C
        // Handle key press/release
        return 1;  // Suppress default handling
    }
    return CallNextHookEx(hHook, nCode, wParam, lParam);
}
```

### Key Mapping

| Key | VK Code | Xbox 360 Mapping |
|-----|---------|-----------------|
| PrintScreen | `0x2C` (VK_SNAPSHOT) | (Application-defined) |

---

## 5. Forwarding to ViGEmBus (Xbox 360)

### Architecture

```
┌─────────────────┐
│  Col01 Thread   │──┐
│  (Gamepad 0x07) │  │
└─────────────────┘  │
                     ▼
┌─────────────────┐  ┌──────────────────┐     ┌─────────────────┐
│  Col03 Thread   │──│ ControllerState  │────▶│  Output Thread  │──▶ ViGEmBus
│  (Consumer 0x0A)│  │  (mutex-locked)  │     │  (dirty-driven) │    (Xbox 360)
└─────────────────┘  └──────────────────┘     └─────────────────┘
                     ▲
┌─────────────────┐  │
│  Keyboard Hook  │──┘
│  (PrintScreen)  │
└─────────────────┘
```

- **3 input threads** read their respective HID interfaces concurrently.
- All inputs merge into a shared `ControllerState` struct (protected by a mutex).
- A single **output thread** polls `ControllerState` at `OUTPUT_INTERVAL` (5 ms).
- Output is **dirty-driven**: only send to ViGEmBus when state has changed since last submission.

### vgamepad API Reference (Python)

```python
import vgamepad as vg

gamepad = vg.VX360Gamepad()

# Buttons
gamepad.press_button(vg.XUSB_BUTTON.XUSB_GAMEPAD_A)
gamepad.release_button(vg.XUSB_BUTTON.XUSB_GAMEPAD_A)

# Triggers (0-255)
gamepad.left_trigger(value=128)
gamepad.right_trigger(value=255)

# Sticks (-32768 to 32767)
gamepad.left_joystick(x_value=0, y_value=0)
gamepad.right_joystick(x_value=0, y_value=0)

# D-Pad
gamepad.press_button(vg.XUSB_BUTTON.XUSB_GAMEPAD_DPAD_UP)

# Submit
gamepad.update()
```

### Xbox 360 Button Enums

| Enum | Xbox Button |
|------|-------------|
| `XUSB_GAMEPAD_DPAD_UP` | D-Pad Up |
| `XUSB_GAMEPAD_DPAD_DOWN` | D-Pad Down |
| `XUSB_GAMEPAD_DPAD_LEFT` | D-Pad Left |
| `XUSB_GAMEPAD_DPAD_RIGHT` | D-Pad Right |
| `XUSB_GAMEPAD_START` | Start |
| `XUSB_GAMEPAD_BACK` | Back |
| `XUSB_GAMEPAD_LEFT_THUMB` | L3 |
| `XUSB_GAMEPAD_RIGHT_THUMB` | R3 |
| `XUSB_GAMEPAD_LEFT_SHOULDER` | LB |
| `XUSB_GAMEPAD_RIGHT_SHOULDER` | RB |
| `XUSB_GAMEPAD_GUIDE` | Guide/Home |
| `XUSB_GAMEPAD_A` | A |
| `XUSB_GAMEPAD_B` | B |
| `XUSB_GAMEPAD_X` | X |
| `XUSB_GAMEPAD_Y` | Y |

---

## 6. Col01 Output Report (Report ID `0xB3`) — LED Control

### Output Report — 13 bytes (including Report ID)

```
Byte Offset  Size    Field
───────────  ──────  ─────────────────────────────────
0            1       Report ID (0xB3)
1            1       Command Type: 0x01 = LED Control
2            1       Red — Opcode
3            1       Red — Brightness (0x00–0xFF)
4            1       Green — Opcode
5            1       Green — Brightness (0x00–0xFF)
6            1       Blue — Opcode
7            1       Blue — Brightness (0x00–0xFF)
8            1       HOME LED — Opcode
9            1       HOME LED — Brightness (0x00–0xFF)
10           1       Consumer Area LED — Opcode
11           1       Consumer Area LED — Brightness (0x00–0xFF)
12           1       Reserved (0x00)
```

### LED Opcodes

| Opcode | Meaning |
|--------|---------|
| `0x00` | No-op (ignore this LED channel) |
| `0x01` | Set brightness to the value in the adjacent byte |
| `0x02` | Turn off (brightness byte ignored) |

### Example: Set Red to 50%, Green off, Blue to 100%

```
B3 01 01 80 02 00 01 FF 00 00 00 00 00
```

Breakdown:
- `B3` — Report ID
- `01` — LED command
- `01 80` — Red: set brightness to 0x80 (128/255 ≈ 50%)
- `02 00` — Green: off
- `01 FF` — Blue: set brightness to 0xFF (100%)
- `00 00` — HOME LED: no-op
- `00 00` — Consumer LED: no-op
- `00` — Reserved

### RGB Behavior

- RGB LEDs operate in **persistent additive mode**: setting one channel does not affect others.
- LED state persists until explicitly changed or the device is power-cycled.
- To turn off all LEDs, send opcode `0x02` for each channel.

---

## 7. Col01 Output Report (Report ID `0xB3`) — Rumble Control

### Output Report — 13 bytes (including Report ID)

```
Byte Offset  Size    Field
───────────  ──────  ─────────────────────────────────
0            1       Report ID (0xB3)
1            1       Command Type: 0x02 = Rumble Control
2            1       Motor Enable (2-bit opcodes per motor)
3            1       Left Motor Intensity (0x00–0xFF)
4            1       Left Motor Duration (× 0.25s; 0 = pulse mode)
5            1       Right Motor Intensity (0x00–0xFF)
6            1       Right Motor Duration (× 0.25s; 0 = pulse mode)
7-12         6       Reserved (0x00)
```

### Motor Enable Byte (byte[2])

```
Bits [1:0] = Left Motor Opcode
Bits [3:2] = Right Motor Opcode
```

| 2-bit Opcode | Meaning |
|-------------|---------|
| `00` | No-op (do not change this motor's state) |
| `01` | Start vibration |
| `10` | Stop vibration |
| `11` | Reserved |

### Example: Start both motors at full intensity for 2 seconds

```
B3 02 05 FF 08 FF 08 00 00 00 00 00 00
```

Breakdown:
- `B3` — Report ID
- `02` — Rumble command
- `05` — Motor enable: `0b00000101` → left=`01` (start), right=`01` (start)
- `FF` — Left intensity: 255 (max)
- `08` — Left duration: 8 × 0.25s = 2.0s
- `FF` — Right intensity: 255 (max)
- `08` — Right duration: 8 × 0.25s = 2.0s
- `00 00 00 00 00 00` — Reserved

### Vibration Modes

| Mode | Condition | Behavior |
|------|-----------|----------|
| **Pulse mode** | Duration byte = `0x00` | Motor vibrates at specified intensity indefinitely until a stop command is received. Intensity is adjustable. |
| **Timed mode** | Duration byte > `0x00` | Motor vibrates at **maximum intensity** for `duration × 0.25s`, then auto-stops. Intensity byte is ignored. |

### XInput State-Machine vs OBOX Pulse Mode

XInput/ViGEmBus rumble is **state-machine based**: the host sends ONE packet when vibration starts, changes intensity, or stops. The virtual gamepad holds that state until explicitly changed. There is no need to resend the same rumble value repeatedly.

OBOX pulse mode (duration=0) requires **continuous high-frequency sending** (~30ms interval) to sustain vibration. If you stop sending, the motor stops.

These two models are fundamentally incompatible: a single state-change packet from XInput cannot sustain OBOX pulse-mode vibration.

**Solution: Producer-Consumer heartbeat adapter**

- **Producer**: The ViGEmBus notification callback updates a shared rumble state (`large_motor`, `small_motor`, `active` flag). This callback fires only when the host changes vibration state.
- **Consumer**: A persistent background heartbeat thread polls the shared state every tick:
  - If `active`: send pulse-mode command (duration=0) at ~30ms interval with current intensity.
  - If just became inactive: send one stop command (byte[2]=0x0A), then idle.
  - If idle: poll at ~10ms interval waiting for next activation.

This bridges the state-machine model to the pulse-mode hardware requirement.

### Example: Stop left motor only

```
B3 02 02 00 00 00 00 00 00 00 00 00 00
```

- `02` — Motor enable: `0b00000010` → left=`10` (stop), right=`00` (no-op)

---

## 8. Calibration Parameters

| Parameter | Value | Description |
|-----------|-------|-------------|
| `STICK_CENTER` | `32768` | Analog stick center value (0x8000) |
| `STICK_DEADZONE` | `2000` | Radial deadzone around center; values within this radius are clamped to 0 |
| `TRIGGER_THRESHOLD` | `100` | Trigger values below this are treated as released |
| `OUTPUT_INTERVAL` | `5 ms` | ViGEmBus output polling interval (200 Hz) |

### Stick Processing Pseudocode

```python
def process_stick(raw_x, raw_y):
    dx = raw_x - STICK_CENTER
    dy = raw_y - STICK_CENTER
    magnitude = math.sqrt(dx*dx + dy*dy)
    if magnitude < STICK_DEADZONE:
        return (0, 0)
    # Normalize and scale to int16 range (-32768..32767)
    scale = min(magnitude / 32768.0, 1.0)
    nx = int((dx / magnitude) * scale * 32767)
    ny = int((dy / magnitude) * scale * 32767)
    # Invert Y for XInput
    ny = -ny
    return (nx, ny)
```

---

## 9. Known Issues

| # | Issue | Details / Workaround |
|---|-------|---------------------|
| 1 | **Home key opens browser** | Windows maps Consumer `0x0223` (AC Home) to launch the default browser. Suppressed automatically with HidHide; without HidHide, use AutoHotKey (`Browser_Home::return`). |
| 2 | **Y-axis inversion** | Device reports Y-up as high value; XInput expects Y-up as low value. Must apply `y = 65535 - y` before forwarding. |
| 3 | **D-Pad diagonal handling** | Hat switch reports diagonals as single values (1,3,5,7). Must decompose into two directional buttons for XInput (e.g., value 1 → UP + RIGHT). |
| 4 | **Keyboard exclusive access** | Windows `kbdclass.sys` exclusively claims the keyboard collection. Cannot read via HIDAPI; must use `SetWindowsHookEx(WH_KEYBOARD_LL)`. |
| 5 | **Consumer slot order** | The 3 usage slots do not guarantee fixed ordering. A key may appear in any slot across reports. Use set-based diffing, not positional comparison. |
| 6 | **hidapi package conflict** | Python packages `hidapi` and `hid` both provide `hid` module. Ensure only one is installed: `pip uninstall hidapi` if using `hid`, or vice versa. |
| 7 | **Device path is bytes** | On Windows, `hid.enumerate()` returns `path` as `bytes`, not `str`. Pass it directly to `hid.device().open_path()` without decoding. |
| 8 | **ViGEmBus driver required** | The virtual Xbox 360 controller requires the [ViGEmBus](https://github.com/nefarius/ViGEmBus) kernel driver to be installed. Without it, `vgamepad` will fail to create the virtual device. |

---

## 10. Quick Verification Checklist

- [ ] Device enumerates with VID `0x0A5C`, PID `0x4502`
- [ ] 4 HID collections are visible (Gamepad, Mouse, Consumer, Keyboard)
- [ ] Col01 input report is 16 bytes with Report ID `0x07`
- [ ] Button presses register in the correct bit positions
- [ ] Analog sticks center at ~32768 when released
- [ ] Y-axis is inverted before XInput forwarding
- [ ] Triggers compress from 16-bit to 8-bit correctly
- [ ] Hat switch values 0–8 map to correct D-Pad states
- [ ] Consumer report (ID `0x0A`) is 7 bytes with 3 usage slots
- [ ] Back (`0x0224`), Menu (`0x0040`), Home (`0x0223`) detected via set-diff
- [ ] Home key does NOT open browser (HidHide or AutoHotKey)
- [ ] PrintScreen captured via global keyboard hook
- [ ] LED output report (ID `0xB3`, cmd `0x01`) changes LED colors
- [ ] Rumble output report (ID `0xB3`, cmd `0x02`) activates motors
- [ ] Pulse mode (duration=0) vibrates until stop command
- [ ] Timed mode auto-stops after `duration × 0.25s`
- [ ] ViGEmBus virtual Xbox 360 controller appears in Device Manager
- [ ] Full input→output pipeline latency < 20 ms
