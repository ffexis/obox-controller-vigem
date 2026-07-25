"""
OBOX Bluetooth Controller -> ViGEmBus Xbox360 Middleware (Python)
================================================================
Reads HID input from OBOX Bluetooth controller (VID=0x0A5C PID=0x4502),
forwards as Xbox 360 virtual gamepad via ViGEmBus.

HID interfaces:
  Col01 Gamepad  (Report ID 0x07, 16B) — buttons/sticks/triggers/dpad
  Col03 Consumer (Report ID 0x0A,  7B) — Back/Menu(Start)/Home(Guide)
  Col04 Keyboard (Windows-exclusive)   — PrintScreen via pynput hook

Dependencies:
  pip install hidapi vgamepad pynput

Usage:
  python obox_middleware.py                   Run in CLI mode (default)
  python obox_middleware.py --cli             Run in CLI mode (explicit)
  python obox_middleware.py --debug-keys     Debug: real-time key input
  python obox_middleware.py --debug-output   Debug: vibration/LED test
  python obox_middleware.py --hidhide-status Show HidHide configuration
  python obox_middleware.py --hidhide-disable Disable HidHide for OBOX
  python obox_middleware.py -h, --help       Show help
  Press Ctrl+C to exit.
"""

import sys
import os
import time
import struct
import ctypes
import threading
import argparse
import subprocess
from datetime import datetime
from pathlib import Path
from typing import Optional, Tuple, List, Set, Dict

# ---- HID ----
try:
    import hid
except ImportError:
    print("[ERROR] Missing hidapi: pip install hidapi")
    sys.exit(1)

# ---- vgamepad ----
try:
    import vgamepad as vg
except ImportError:
    print("[ERROR] Missing vgamepad: pip install --no-build-isolation vgamepad")
    sys.exit(1)

# ---- pynput (keyboard hook) ----
try:
    from pynput import keyboard as pynput_kb
except ImportError:
    print("[ERROR] Missing pynput: pip install pynput")
    sys.exit(1)


# ============================================================
# Constants (matching Rust implementation)
# ============================================================
VENDOR_ID  = 0x0A5C
PRODUCT_ID = 0x4502

STICK_CENTER    = 32768
STICK_DEADZONE  = 2000
TRIGGER_THRESHOLD = 100

REPORT_ID_GAMEPAD  = 0x07
REPORT_ID_CONSUMER = 0x0A
REPORT_ID_OUTPUT   = 0xB3

USAGE_BACK = 0x224
USAGE_MENU = 0x040
USAGE_HOME = 0x223

XBUTTON_A     = 0x0001
XBUTTON_B     = 0x0002
XBUTTON_X     = 0x0004
XBUTTON_Y     = 0x0008
XBUTTON_LB    = 0x0010
XBUTTON_RB    = 0x0020
XBUTTON_L3    = 0x0040
XBUTTON_R3    = 0x0080
XBUTTON_BACK  = 0x0100
XBUTTON_START = 0x0200
XBUTTON_GUIDE = 0x0400
XBUTTON_UP    = 0x1000
XBUTTON_DOWN  = 0x2000
XBUTTON_LEFT  = 0x4000
XBUTTON_RIGHT = 0x8000

# vgamepad button mapping for consumer usage -> Xbox360
CONSUMER_MAP = {
    USAGE_BACK: vg.XUSB_BUTTON.XUSB_GAMEPAD_BACK,
    USAGE_MENU: vg.XUSB_BUTTON.XUSB_GAMEPAD_START,
    USAGE_HOME: vg.XUSB_BUTTON.XUSB_GAMEPAD_GUIDE,
}

# vgamepad button mapping for gamepad buttons -> Xbox360
BUTTON_MAP = {
    1:  vg.XUSB_BUTTON.XUSB_GAMEPAD_A,
    2:  vg.XUSB_BUTTON.XUSB_GAMEPAD_B,
    4:  vg.XUSB_BUTTON.XUSB_GAMEPAD_X,
    5:  vg.XUSB_BUTTON.XUSB_GAMEPAD_Y,
    7:  vg.XUSB_BUTTON.XUSB_GAMEPAD_LEFT_SHOULDER,
    8:  vg.XUSB_BUTTON.XUSB_GAMEPAD_RIGHT_SHOULDER,
    14: vg.XUSB_BUTTON.XUSB_GAMEPAD_LEFT_THUMB,
    15: vg.XUSB_BUTTON.XUSB_GAMEPAD_RIGHT_THUMB,
}

# Hat switch -> Xbox360 D-pad buttons
HAT_MAP = {
    0: (vg.XUSB_BUTTON.XUSB_GAMEPAD_DPAD_UP,),
    1: (vg.XUSB_BUTTON.XUSB_GAMEPAD_DPAD_UP,    vg.XUSB_BUTTON.XUSB_GAMEPAD_DPAD_RIGHT),
    2: (vg.XUSB_BUTTON.XUSB_GAMEPAD_DPAD_RIGHT,),
    3: (vg.XUSB_BUTTON.XUSB_GAMEPAD_DPAD_DOWN,  vg.XUSB_BUTTON.XUSB_GAMEPAD_DPAD_RIGHT),
    4: (vg.XUSB_BUTTON.XUSB_GAMEPAD_DPAD_DOWN,),
    5: (vg.XUSB_BUTTON.XUSB_GAMEPAD_DPAD_DOWN,  vg.XUSB_BUTTON.XUSB_GAMEPAD_DPAD_LEFT),
    6: (vg.XUSB_BUTTON.XUSB_GAMEPAD_DPAD_LEFT,),
    7: (vg.XUSB_BUTTON.XUSB_GAMEPAD_DPAD_UP,    vg.XUSB_BUTTON.XUSB_GAMEPAD_DPAD_LEFT),
}

# Xbox360 button bit constants for debug output
XBOX_BUTTON_NAMES = [
    (XBUTTON_A,     "A"),
    (XBUTTON_B,     "B"),
    (XBUTTON_X,     "X"),
    (XBUTTON_Y,     "Y"),
    (XBUTTON_LB,    "LB"),
    (XBUTTON_RB,    "RB"),
    (XBUTTON_L3,    "L3"),
    (XBUTTON_R3,    "R3"),
    (XBUTTON_BACK,  "Back"),
    (XBUTTON_START, "Start"),
    (XBUTTON_GUIDE, "Guide"),
    (XBUTTON_UP,    "Up"),
    (XBUTTON_DOWN,  "Down"),
    (XBUTTON_LEFT,  "Left"),
    (XBUTTON_RIGHT, "Right"),
]

# Consumer usage names for debug
CONSUMER_USAGE_NAMES = {
    USAGE_BACK: "BACK",
    USAGE_MENU: "MENU/START",
    USAGE_HOME: "HOME/GUIDE",
}


# ============================================================
# Windows Single Instance Check
# ============================================================
def check_single_instance() -> bool:
    """Ensure only one instance is running via Windows mutex."""
    if sys.platform != "win32":
        return True
    try:
        kernel32 = ctypes.windll.kernel32
        mutex_name = "OBOXControllerDriverMutex"
        mutex = kernel32.CreateMutexW(None, False, mutex_name)
        if kernel32.GetLastError() == 183:  # ERROR_ALREADY_EXISTS
            kernel32.CloseHandle(mutex)
            return False
        return True
    except Exception:
        return True


# ============================================================
# HidHide Integration (ported from Rust hidhide.rs)
# ============================================================
DEFAULT_CLI_PATH = r"C:\Program Files\Nefarius Software Solutions\HidHide\x64\HidHideCLI.exe"

OBOX_PATTERNS = ["0a5c", "pid&4502", "col01"]


def _find_hidhide_cli() -> Optional[Path]:
    """Locate HidHideCLI.exe."""
    p = Path(DEFAULT_CLI_PATH)
    if p.exists():
        return p
    return None


def _run_hidhide_cli(cli: Path, args: List[str]) -> str:
    """Execute HidHideCLI and return stdout."""
    cmd = [str(cli)] + args
    result = subprocess.run(cmd, capture_output=True, text=True, timeout=10)
    if result.returncode != 0:
        raise RuntimeError(
            f"HidHideCLI {args} failed (exit {result.returncode}): {result.stderr.strip()}"
        )
    return result.stdout


def _parse_quoted_values(text: str, prefix: str) -> Set[str]:
    """Parse '--cmd "value"' lines from HidHideCLI output."""
    values = set()
    for line in text.splitlines():
        line = line.strip()
        if not line.startswith(prefix):
            continue
        rest = line[len(prefix):].strip()
        if len(rest) >= 2 and rest.startswith('"') and rest.endswith('"'):
            values.add(rest[1:-1].lower())
    return values


def _extract_instance_paths(json_text: str, patterns: List[str]) -> List[str]:
    """Extract deviceInstancePath values from --dev-gaming JSON output."""
    key = '"deviceInstancePath"'
    results = []
    i = 0
    while i + len(key) <= len(json_text):
        idx = json_text.find(key, i)
        if idx == -1:
            break
        # Find opening quote after the key
        j = idx + len(key)
        while j < len(json_text) and json_text[j] != '"':
            j += 1
        if j >= len(json_text):
            break
        start = j + 1
        end = json_text.find('"', start)
        if end == -1:
            break
        val_raw = json_text[start:end]
        val = val_raw.replace("\\\\", "\\")
        val_lower = val.lower()
        if all(p in val_lower for p in patterns):
            results.append(val)
        i = end + 1
    return results


def _parse_dev_list(text: str) -> Set[str]:
    """Parse --dev-list output for hidden device paths."""
    return _parse_quoted_values(text, "--dev-hide")


def _parse_app_list(text: str) -> Set[str]:
    """Parse --app-list output for registered app paths."""
    return _parse_quoted_values(text, "--app-reg")


def hidhide_ensure_enabled(app_path: str) -> bool:
    """
    Auto-check and enable HidHide configuration:
    - Register current exe (idempotent)
    - Hide OBOX device interfaces (idempotent)
    - Enable cloak if OFF
    Returns True on success, False on warning (continues without HidHide).

    NOTE: For Python, app_path should be sys.executable (python.exe),
    NOT the .py script path. HidHide works at the process level —
    it matches the executable that opens the HID device.
    """
    print("[HidHide] Checking HidHide configuration...")

    cli = _find_hidhide_cli()
    if cli is None:
        print(f"[HidHide] WARN: HidHideCLI.exe not found at {DEFAULT_CLI_PATH}")
        print("[HidHide] Continuing without HidHide...")
        return False
    print(f"[HidHide] CLI: {cli}")
    print(f"[HidHide] Application path: {app_path}")

    try:
        # Register app
        app_list = _run_hidhide_cli(cli, ["--app-list"])
        registered = _parse_app_list(app_list)
        app_lower = app_path.lower()
        if app_lower not in registered:
            _run_hidhide_cli(cli, ["--app-reg", app_path])
            print("[HidHide] Application registered")
        else:
            print("[HidHide] Application already registered")

        # Hide OBOX devices
        gaming_json = _run_hidhide_cli(cli, ["--dev-gaming"])
        dev_list = _run_hidhide_cli(cli, ["--dev-list"])
        hidden_devs = _parse_dev_list(dev_list)
        instances = _extract_instance_paths(gaming_json, OBOX_PATTERNS)

        if not instances:
            print("[HidHide] No OBOX device interfaces found (controller disconnected?)")
        else:
            hidden_count = 0
            already_count = 0
            for inst in instances:
                if inst.lower() in hidden_devs:
                    already_count += 1
                    continue
                try:
                    _run_hidhide_cli(cli, ["--dev-hide", inst])
                    hidden_count += 1
                    print(f"[HidHide] Hidden: {inst}")
                except Exception as e:
                    print(f"[HidHide] WARN: Failed to hide {inst}: {e}")
            print(f"[HidHide] {hidden_count} interface(s) hidden, {already_count} already hidden")

        # Ensure cloak ON
        state = _run_hidhide_cli(cli, ["--cloak-state"]).strip()
        if "--cloak-off" in state:
            print("[HidHide] Cloak is OFF, enabling...")
            _run_hidhide_cli(cli, ["--cloak-on"])
            print("[HidHide] Cloak ON (global)")
        elif "--cloak-on" in state:
            print("[HidHide] Cloak already ON")
        else:
            print(f"[HidHide] Cloak state unknown: {state}")

        print("[HidHide] Configuration OK.")
        return True
    except Exception as e:
        print(f"[HidHide] WARN: {e}")
        print("[HidHide] Continuing without HidHide...")
        return False


def hidhide_print_status() -> None:
    """Print current HidHide status."""
    cli = _find_hidhide_cli()
    if cli is None:
        print(f"HidHideCLI.exe not found at {DEFAULT_CLI_PATH}")
        sys.exit(1)

    # Use Python interpreter path — HidHide matches the executable, not the script
    app_path = sys.executable

    try:
        combined = _run_hidhide_cli(cli, ["--cloak-state", "--app-list", "--dev-list", "--dev-gaming"])
    except Exception as e:
        print(f"Error: {e}")
        sys.exit(1)

    cloak_state = ""
    app_lines = []
    dev_lines = []
    json_lines = []
    in_json = False
    for line in combined.splitlines():
        trimmed = line.strip()
        if trimmed.startswith("{") or in_json:
            in_json = True
            json_lines.append(line)
            if "]" in trimmed:
                in_json = False
            continue
        if trimmed.startswith("--cloak-"):
            cloak_state = trimmed
        elif trimmed.startswith("--app-reg"):
            app_lines.append(trimmed)
        elif trimmed.startswith("--dev-hide"):
            dev_lines.append(trimmed)

    registered_apps = _parse_quoted_values("\n".join(app_lines), "--app-reg")
    hidden_devs = _parse_quoted_values("\n".join(dev_lines), "--dev-hide")

    print("=== HidHide Status ===")
    print(f"CLI: {cli}")
    print(f"Cloak: {cloak_state}")
    if "--cloak-off" in cloak_state:
        print("  (INACTIVE -- hidden devices are NOT actually hidden)")
    elif "--cloak-on" in cloak_state:
        print("  (ACTIVE -- hidden devices are hidden from other apps)")

    app_lower = app_path.lower()
    app_registered = app_lower in registered_apps
    print(f"This app registered: {'YES' if app_registered else 'NO'}")
    print(f"  (path: {app_path})")
    print(f"Total apps registered: {len(registered_apps)}")

    gaming_json = "\n".join(json_lines)
    instances = _extract_instance_paths(gaming_json, OBOX_PATTERNS)

    if not instances:
        print("OBOX Col01 (gamepad) interface: NOT FOUND (controller disconnected?)")
    else:
        for inst in instances:
            is_hidden = inst.lower() in hidden_devs
            status = "HIDDEN" if is_hidden else "VISIBLE"
            print(f"OBOX Col01 (gamepad): {status}")
            print(f"  {inst}")
            if is_hidden:
                print("  (Col02/Col03 auto-hidden by HidHide)")


def hidhide_disable() -> None:
    """Disable HidHide for OBOX controller only."""
    print("[HidHide] Disabling HidHide for OBOX controller...")

    cli = _find_hidhide_cli()
    if cli is None:
        print(f"[HidHide] HidHideCLI.exe not found at {DEFAULT_CLI_PATH}")
        sys.exit(1)

    try:
        gaming_json = _run_hidhide_cli(cli, ["--dev-gaming"])
        dev_list = _run_hidhide_cli(cli, ["--dev-list"])
    except Exception as e:
        print(f"[HidHide] Error: {e}")
        sys.exit(1)

    hidden_devs = _parse_dev_list(dev_list)
    instances = _extract_instance_paths(gaming_json, OBOX_PATTERNS)

    if not instances:
        print("[HidHide] No OBOX device interfaces found. Nothing to unhide.")
        return

    unhidden = 0
    already = 0
    for inst in instances:
        if inst.lower() not in hidden_devs:
            already += 1
            continue
        try:
            _run_hidhide_cli(cli, ["--dev-unhide", inst])
            unhidden += 1
            print(f"[HidHide] Unhidden: {inst}")
        except Exception as e:
            print(f"[HidHide] WARN: Failed to unhide {inst}: {e}")

    print(f"[HidHide] {unhidden} interface(s) unhidden, {already} already visible")
    print("[HidHide] NOTE: Global cloak state left unchanged (may affect other apps).")
    print(f'[HidHide] To fully disable HidHide, run: "{cli}" --cloak-off')


# ============================================================
# Controller State (thread-safe)
# ============================================================
class ControllerState:
    """Thread-shared controller state."""

    def __init__(self):
        self._lock = threading.Lock()
        self.gamepad_buttons: int = 0
        self.consumer_buttons: int = 0
        self.lt: int = 0
        self.rt: int = 0
        self.thumb_lx: int = 0
        self.thumb_ly: int = 0
        self.thumb_rx: int = 0
        self.thumb_ry: int = 0
        self.dirty: bool = True

    def update(self, gamepad_buttons: int = None, consumer_buttons: int = None,
               lt: int = None, rt: int = None,
               thumb_lx: int = None, thumb_ly: int = None,
               thumb_rx: int = None, thumb_ry: int = None):
        with self._lock:
            changed = False
            if gamepad_buttons is not None and self.gamepad_buttons != gamepad_buttons:
                self.gamepad_buttons = gamepad_buttons
                changed = True
            if consumer_buttons is not None and self.consumer_buttons != consumer_buttons:
                self.consumer_buttons = consumer_buttons
                changed = True
            if lt is not None and self.lt != lt:
                self.lt = lt
                changed = True
            if rt is not None and self.rt != rt:
                self.rt = rt
                changed = True
            if thumb_lx is not None and self.thumb_lx != thumb_lx:
                self.thumb_lx = thumb_lx
                changed = True
            if thumb_ly is not None and self.thumb_ly != thumb_ly:
                self.thumb_ly = thumb_ly
                changed = True
            if thumb_rx is not None and self.thumb_rx != thumb_rx:
                self.thumb_rx = thumb_rx
                changed = True
            if thumb_ry is not None and self.thumb_ry != thumb_ry:
                self.thumb_ry = thumb_ry
                changed = True
            if changed:
                self.dirty = True

    def snapshot_and_clear_dirty(self):
        with self._lock:
            if not self.dirty:
                return None
            self.dirty = False
            return {
                "buttons": self.gamepad_buttons | self.consumer_buttons,
                "lt": self.lt,
                "rt": self.rt,
                "thumb_lx": self.thumb_lx,
                "thumb_ly": self.thumb_ly,
                "thumb_rx": self.thumb_rx,
                "thumb_ry": self.thumb_ry,
            }


# ============================================================
# HID Device Finding
# ============================================================
def find_path(usage_page: int, usage: int) -> Optional[str]:
    """Find HID device path by usage_page and usage."""
    devices = hid.enumerate(VENDOR_ID, PRODUCT_ID)
    for d in devices:
        if d.get("usage_page", 0) == usage_page and d.get("usage", 0) == usage:
            return d["path"]
    return None


def get_mac_address() -> str:
    """Extract MAC address from HID device serial number."""
    devices = hid.enumerate(VENDOR_ID, PRODUCT_ID)
    for d in devices:
        serial = d.get("serial_number", "")
        if len(serial) >= 12:
            mac = ":".join(serial[i:i+2].upper() for i in range(0, 12, 2))
            return mac
    return ""


# ============================================================
# Deadzone Rescaling (matching Rust implementation)
# ============================================================
def apply_deadzone(val: int) -> int:
    """Apply deadzone with rescaling.

    Maps deadzone boundary to 0, full deflection to ±32767.
    Output always fits in int16 range.
    """
    delta = val - STICK_CENTER
    if abs(delta) < STICK_DEADZONE:
        return 0
    if delta > 0:
        return int((delta - STICK_DEADZONE) * 32767 / (32767 - STICK_DEADZONE))
    else:
        return int((delta + STICK_DEADZONE) * 32767 / (32768 - STICK_DEADZONE))


def apply_deadzone_y(val: int) -> int:
    """Apply deadzone with rescaling + Y-axis inversion for XInput.

    Device: UP=low(0), DOWN=high(65535).
    XInput: UP=positive, DOWN=negative.
    So we negate the Y axis.
    """
    return -apply_deadzone(val)


def apply_trigger_deadzone(val: int) -> int:
    """Apply trigger threshold and compress 16-bit to 8-bit (protocol: value >> 8)."""
    if val < TRIGGER_THRESHOLD:
        return 0
    return min(255, val >> 8)


# ============================================================
# Gamepad Reader Thread
# ============================================================
class GamepadReader(threading.Thread):
    """Read Col01 Gamepad interface (Report ID 0x07) with reconnect."""

    def __init__(self, path: str, state: ControllerState, debug: bool = False):
        super().__init__(daemon=True)
        self.path = path
        self.state = state
        self.debug = debug
        self.running = True

    def _process(self, h):
        """Read and process one frame. Returns False on disconnect."""
        buf = h.read(64, timeout_ms=5)
        if not buf:
            return True
        buf = bytes(buf)
        if len(buf) < 16 or buf[0] != REPORT_ID_GAMEPAD:
            return True

        # Buttons: 16-bit bitmap (bytes 1-2, little-endian)
        buttons = buf[1] | (buf[2] << 8)

        # Hat switch: lower 4 bits of byte 3
        hat = buf[3] & 0x0F

        # Map button bits to Xbox buttons (protocol bit num - 1 = actual bit offset)
        btns = 0
        if buttons & (1 << 0):  btns |= XBUTTON_A
        if buttons & (1 << 1):  btns |= XBUTTON_B
        if buttons & (1 << 3):  btns |= XBUTTON_X
        if buttons & (1 << 4):  btns |= XBUTTON_Y
        if buttons & (1 << 6):  btns |= XBUTTON_LB
        if buttons & (1 << 7):  btns |= XBUTTON_RB
        if buttons & (1 << 13): btns |= XBUTTON_L3
        if buttons & (1 << 14): btns |= XBUTTON_R3

        # Map hat to D-pad
        dpad = 0
        if hat == 0: dpad = XBUTTON_UP
        elif hat == 1: dpad = XBUTTON_UP | XBUTTON_RIGHT
        elif hat == 2: dpad = XBUTTON_RIGHT
        elif hat == 3: dpad = XBUTTON_RIGHT | XBUTTON_DOWN
        elif hat == 4: dpad = XBUTTON_DOWN
        elif hat == 5: dpad = XBUTTON_DOWN | XBUTTON_LEFT
        elif hat == 6: dpad = XBUTTON_LEFT
        elif hat == 7: dpad = XBUTTON_LEFT | XBUTTON_UP

        btns |= dpad

        # Sticks with deadzone rescaling (bytes 4-11)
        lx_raw = struct.unpack_from("<H", buf, 4)[0]
        ly_raw = struct.unpack_from("<H", buf, 6)[0]
        rx_raw = struct.unpack_from("<H", buf, 8)[0]
        ry_raw = struct.unpack_from("<H", buf, 10)[0]

        lx = apply_deadzone(lx_raw)
        ly = apply_deadzone_y(ly_raw)
        rx = apply_deadzone(rx_raw)
        ry = apply_deadzone_y(ry_raw)

        # Triggers with deadzone (bytes 12-15)
        lt_raw = struct.unpack_from("<H", buf, 12)[0]
        rt_raw = struct.unpack_from("<H", buf, 14)[0]
        lt = apply_trigger_deadzone(lt_raw)
        rt = apply_trigger_deadzone(rt_raw)

        self.state.update(
            gamepad_buttons=btns,
            lt=lt, rt=rt,
            thumb_lx=lx, thumb_ly=ly,
            thumb_rx=rx, thumb_ry=ry,
        )

        if self.debug:
            btn_names = [name for bit, name in XBOX_BUTTON_NAMES if btns & bit]
            dpad_names = ["UP", "UP+RIGHT", "RIGHT", "RIGHT+DOWN",
                           "DOWN", "DOWN+LEFT", "LEFT", "LEFT+UP"]
            dpad_name = dpad_names[hat] if hat < 8 else "none"
            print(f"[Col01] btns=0x{btns:04X} ({' '.join(btn_names) or 'none'}), "
                  f"hat={hat} ({dpad_name}), "
                  f"LX={lx} LY={ly} RX={rx} RY={ry} LT={lt} RT={rt}")

        return True

    def run(self):
        while self.running:
            try:
                if self.debug:
                    print("[Gamepad] Opening interface...")
                h = hid.device()
                h.open_path(self.path)
                h.set_nonblocking(True)
                if self.debug:
                    print("[Gamepad] Connected, reading...")
                while self.running:
                    if not self._process(h):
                        break
                h.close()
            except Exception as e:
                if not self.running:
                    break
                if self.debug:
                    print(f"[Gamepad] Disconnected ({e}), retrying in 3s...")
                time.sleep(3)
                g_path = find_path(0x0001, 0x0005)
                if g_path:
                    self.path = g_path


# ============================================================
# Consumer Reader Thread
# ============================================================
class ConsumerReader(threading.Thread):
    """Read Col03 Consumer interface (Report ID 0x0A) with reconnect."""

    def __init__(self, path: str, state: ControllerState, debug: bool = False):
        super().__init__(daemon=True)
        self.path = path
        self.state = state
        self.debug = debug
        self.running = True

    def _process(self, h):
        """Read and process one frame. Returns False on disconnect."""
        buf = h.read(64, timeout_ms=100)
        if not buf:
            return True
        buf = bytes(buf)
        if len(buf) < 7 or buf[0] != REPORT_ID_CONSUMER:
            return True

        # Parse usages (3 slots, filter 0x0000)
        curr_usages = set()
        for i in range(3):
            u = buf[1 + i * 2] | (buf[2 + i * 2] << 8)
            if u:
                curr_usages.add(u)

        prev_usages = getattr(self, "_prev_usages", set())

        # Process press/release via set difference
        for u in curr_usages - prev_usages:
            xbtn = CONSUMER_MAP.get(u)
            if xbtn:
                with self.state._lock:
                    # Map Xbox bit to vgamepad button
                    self.state.consumer_buttons |= _xbox_bit_from_usage(u)
                    self.state.dirty = True
                if self.debug:
                    print(f"[Col03] Usage 0x{u:04X} ({CONSUMER_USAGE_NAMES.get(u, 'unknown')}) pressed")

        for u in prev_usages - curr_usages:
            xbtn = CONSUMER_MAP.get(u)
            if xbtn:
                with self.state._lock:
                    self.state.consumer_buttons &= ~_xbox_bit_from_usage(u)
                    self.state.dirty = True
                if self.debug:
                    print(f"[Col03] Usage 0x{u:04X} ({CONSUMER_USAGE_NAMES.get(u, 'unknown')}) released")

        self._prev_usages = curr_usages
        return True

    def run(self):
        while self.running:
            try:
                if self.debug:
                    print("[Consumer] Opening interface...")
                h = hid.device()
                h.open_path(self.path)
                if self.debug:
                    print("[Consumer] Connected, reading...")
                while self.running:
                    if not self._process(h):
                        break
                    time.sleep(0.001)
                h.close()
            except Exception as e:
                if not self.running:
                    break
                if self.debug:
                    print(f"[Consumer] Disconnected ({e}), retrying in 3s...")
                time.sleep(3)
                c_path = find_path(0x000C, 0x0001)
                if c_path:
                    self.path = c_path


def _xbox_bit_from_usage(usage: int) -> int:
    """Map consumer usage to Xbox button bit constant."""
    if usage == USAGE_BACK:
        return XBUTTON_BACK
    elif usage == USAGE_MENU:
        return XBUTTON_START
    elif usage == USAGE_HOME:
        return XBUTTON_GUIDE
    return 0


# ============================================================
# Keyboard Hook (pynput)
# ============================================================
class KeyboardReader:
    """Hook keyboard events via pynput (for PrintScreen etc.)."""

    def __init__(self, state: ControllerState, debug: bool = False):
        self.state = state
        self.debug = debug
        self.listener = None

    def on_press(self, key):
        vk = None
        if isinstance(key, pynput_kb.Key):
            vk = key.value.vk if hasattr(key, "value") and key.value else None
        elif isinstance(key, pynput_kb.KeyCode):
            vk = key.vk
        # PrintScreen (vk=0x2C) — not mapped to Xbox360 for now

    def on_release(self, key):
        pass  # Same as on_press — no mapping active

    def start(self):
        if self.debug:
            print("[Keyboard] Starting keyboard hook...")
        self.listener = pynput_kb.Listener(
            on_press=self.on_press,
            on_release=self.on_release,
        )
        self.listener.start()
        if self.debug:
            print("[Keyboard] Started")

    def stop(self):
        if self.listener:
            self.listener.stop()
        if self.debug:
            print("[Keyboard] Stopped")


# ============================================================
# Rumble Handler — XInput state-machine → OBOX pulse-mode adapter
# ============================================================
#
# XInput rumble is state-based: host sends one packet when vibration
# starts/changes/stops.  OBOX pulse mode (duration=0) requires
# continuous high-frequency sending to sustain vibration.
#
# Architecture (producer–consumer):
#   Producer  – ViGEmBus notification callback updates shared state
#   Consumer  – persistent heartbeat thread reads state every tick:
#               active  → send pulse command at ~30 ms interval
#               idle    → send one stop command, then poll at 10 ms
#
RUMBLE_HEARTBEAT_INTERVAL = 0.03   # 30 ms between pulse commands
RUMBLE_IDLE_INTERVAL      = 0.01   # 10 ms idle polling


class RumbleHandler:
    """Adapt XInput state-machine rumble to OBOX pulse-mode rumble."""

    def __init__(self):
        self._lock = threading.Lock()
        self._output_device = None
        self._large = 0
        self._small = 0
        self._active = False
        self._stop_event = threading.Event()
        self._thread: Optional[threading.Thread] = None

    def set_output_device(self, device):
        with self._lock:
            self._output_device = device

    def start(self):
        if self._thread is None or not self._thread.is_alive():
            self._stop_event.clear()
            self._thread = threading.Thread(target=self._heartbeat_loop, daemon=True)
            self._thread.start()
            print("[Rumble] Heartbeat thread started")

    def handle(self, large_motor: int, small_motor: int):
        """Called from ViGEmBus notification callback — just update shared state."""
        with self._lock:
            self._large = large_motor
            self._small = small_motor
            self._active = (large_motor > 0 or small_motor > 0)
        if self._active:
            print(f"[Rumble] State update: large={large_motor} small={small_motor}")
        else:
            print("[Rumble] State update: STOP")

    def _heartbeat_loop(self):
        last_was_active = False
        while not self._stop_event.is_set():
            with self._lock:
                large = self._large
                small = self._small
                active = self._active
                dev = self._output_device

            if active and dev is not None:
                enable = 0
                if large > 0:
                    enable |= 0x01
                if small > 0:
                    enable |= 0x04
                cmd = bytearray(13)
                cmd[0] = REPORT_ID_OUTPUT
                cmd[1] = 0x02
                cmd[2] = enable
                cmd[3] = large
                cmd[4] = 0x00
                cmd[5] = small
                cmd[6] = 0x00
                try:
                    dev.write(bytes(cmd))
                except Exception:
                    pass
                last_was_active = True
                time.sleep(RUMBLE_HEARTBEAT_INTERVAL)
            else:
                if last_was_active and dev is not None:
                    cmd = bytearray(13)
                    cmd[0] = REPORT_ID_OUTPUT
                    cmd[1] = 0x02
                    cmd[2] = 0x0A
                    try:
                        dev.write(bytes(cmd))
                    except Exception:
                        pass
                    print("[Rumble] Sent stop command")
                    last_was_active = False
                time.sleep(RUMBLE_IDLE_INTERVAL)

    def stop(self):
        self._stop_event.set()
        with self._lock:
            self._active = False
            self._large = 0
            self._small = 0
            dev = self._output_device
        if dev is not None:
            cmd = bytearray(13)
            cmd[0] = REPORT_ID_OUTPUT
            cmd[1] = 0x02
            cmd[2] = 0x0A
            try:
                dev.write(bytes(cmd))
            except Exception:
                pass


# ============================================================
# LED Control (for debug-output mode)
# ============================================================
def send_led_command(device, led_type: str, on: bool) -> None:
    """Send LED command to physical gamepad."""
    cmd = bytearray(13)
    cmd[0] = REPORT_ID_OUTPUT
    cmd[1] = 0x01

    offsets = {
        "red":      (2, 0xFF if on else 0x00),
        "green":    (4, 0xFF if on else 0x00),
        "blue":     (6, 0xFF if on else 0x00),
        "consumer": (10, 0xFF if on else 0x00),
        "home":     (8, 0xFF if on else 0x00),
    }

    if led_type in offsets:
        offset, brightness = offsets[led_type]
        cmd[offset] = 0x01 if on else 0x02
        cmd[offset + 1] = brightness

    try:
        device.write(bytes(cmd))
    except Exception as e:
        print(f"  Write failed: {e}")


# ============================================================
# Output Thread (state -> vgamepad)
# ============================================================
class OutputThread(threading.Thread):
    """Periodically write ControllerState to Xbox360 virtual gamepad."""

    def __init__(self, gamepad, state: ControllerState, debug: bool = False):
        super().__init__(daemon=True)
        self.gamepad = gamepad
        self.state = state
        self.debug = debug
        self.running = True

    def run(self):
        if self.debug:
            print("[Output] Output thread started")
        while self.running:
            snap = self.state.snapshot_and_clear_dirty()
            if snap is None:
                time.sleep(0.005)
                continue

            self.gamepad.reset()

            # Buttons (decode Xbox bit mask to vgamepad)
            btns = snap["buttons"]
            for bit, vbtn in [
                (XBUTTON_A,     vg.XUSB_BUTTON.XUSB_GAMEPAD_A),
                (XBUTTON_B,     vg.XUSB_BUTTON.XUSB_GAMEPAD_B),
                (XBUTTON_X,     vg.XUSB_BUTTON.XUSB_GAMEPAD_X),
                (XBUTTON_Y,     vg.XUSB_BUTTON.XUSB_GAMEPAD_Y),
                (XBUTTON_LB,    vg.XUSB_BUTTON.XUSB_GAMEPAD_LEFT_SHOULDER),
                (XBUTTON_RB,    vg.XUSB_BUTTON.XUSB_GAMEPAD_RIGHT_SHOULDER),
                (XBUTTON_L3,    vg.XUSB_BUTTON.XUSB_GAMEPAD_LEFT_THUMB),
                (XBUTTON_R3,    vg.XUSB_BUTTON.XUSB_GAMEPAD_RIGHT_THUMB),
                (XBUTTON_BACK,  vg.XUSB_BUTTON.XUSB_GAMEPAD_BACK),
                (XBUTTON_START, vg.XUSB_BUTTON.XUSB_GAMEPAD_START),
                (XBUTTON_GUIDE, vg.XUSB_BUTTON.XUSB_GAMEPAD_GUIDE),
                (XBUTTON_UP,    vg.XUSB_BUTTON.XUSB_GAMEPAD_DPAD_UP),
                (XBUTTON_DOWN,  vg.XUSB_BUTTON.XUSB_GAMEPAD_DPAD_DOWN),
                (XBUTTON_LEFT,  vg.XUSB_BUTTON.XUSB_GAMEPAD_DPAD_LEFT),
                (XBUTTON_RIGHT, vg.XUSB_BUTTON.XUSB_GAMEPAD_DPAD_RIGHT),
            ]:
                if btns & bit:
                    self.gamepad.press_button(vbtn)

            # Sticks (convert back to float: int16 -> float)
            lx_f = snap["thumb_lx"] / 32767.0
            ly_f = snap["thumb_ly"] / 32767.0
            rx_f = snap["thumb_rx"] / 32767.0
            ry_f = snap["thumb_ry"] / 32767.0

            self.gamepad.left_joystick_float(lx_f, ly_f)
            self.gamepad.right_joystick_float(rx_f, ry_f)

            # Triggers (byte)
            self.gamepad.left_trigger(snap["lt"])
            self.gamepad.right_trigger(snap["rt"])

            self.gamepad.update()

            if self.debug:
                btn_names = [name for bit, name in [
                    (XBUTTON_A, "A"), (XBUTTON_B, "B"), (XBUTTON_X, "X"), (XBUTTON_Y, "Y"),
                    (XBUTTON_LB, "LB"), (XBUTTON_RB, "RB"),
                    (XBUTTON_BACK, "Back"), (XBUTTON_START, "Start"), (XBUTTON_GUIDE, "Guide"),
                ] if btns & bit]
                ts = datetime.now().strftime("%H:%M:%S.%f")[:-3]
                print(f"  [Output] {ts}  btns:{btn_names}  "
                      f"L({lx_f:.2f},{ly_f:.2f}) R({rx_f:.2f},{ry_f:.2f}) "
                      f"LT:{snap['lt']} RT:{snap['rt']}")

            time.sleep(0.005)

        if self.debug:
            print("[Output] Output thread stopped")


# ============================================================
# Debug: Real-time Key Input
# ============================================================
def debug_keys() -> None:
    """Debug mode: print real-time key input from controller."""
    gamepad_path = find_path(0x0001, 0x0005)
    consumer_path = find_path(0x000C, 0x0001)

    if not gamepad_path:
        print("Gamepad interface (Col01) not found. Is controller connected?")
        sys.exit(1)
    if not consumer_path:
        print("Consumer interface (Col03) not found. Is controller connected?")
        sys.exit(1)

    gp = hid.device()
    gp.open_path(gamepad_path)
    gp.set_nonblocking(True)

    cs = hid.device()
    cs.open_path(consumer_path)
    cs.set_nonblocking(True)

    prev_gamepad_btns = 0
    prev_consumer_usages = set()

    print("Debug mode: Press any button on controller...")
    print("Press Ctrl+C to exit.\n")

    try:
        while True:
            # Read gamepad
            buf = gp.read(64, timeout_ms=5)
            if buf:
                buf = bytes(buf)
                if len(buf) >= 16 and buf[0] == REPORT_ID_GAMEPAD:
                    # Buttons: 16-bit bitmap (bytes 1-2, little-endian)
                    buttons = buf[1] | (buf[2] << 8)
                    # Hat: lower 4 bits of byte 3
                    hat = buf[3] & 0x0F

                    btns = 0
                    if buttons & (1 << 0):  btns |= XBUTTON_A
                    if buttons & (1 << 1):  btns |= XBUTTON_B
                    if buttons & (1 << 3):  btns |= XBUTTON_X
                    if buttons & (1 << 4):  btns |= XBUTTON_Y
                    if buttons & (1 << 6):  btns |= XBUTTON_LB
                    if buttons & (1 << 7):  btns |= XBUTTON_RB
                    if buttons & (1 << 13): btns |= XBUTTON_L3
                    if buttons & (1 << 14): btns |= XBUTTON_R3

                    dpad_names = ["UP", "UP+RIGHT", "RIGHT", "RIGHT+DOWN",
                                   "DOWN", "DOWN+LEFT", "LEFT", "LEFT+UP"]
                    dpad = dpad_names[hat] if hat < 8 else "?"

                    lx = struct.unpack_from("<H", buf, 4)[0]
                    ly = struct.unpack_from("<H", buf, 6)[0]
                    rx = struct.unpack_from("<H", buf, 8)[0]
                    ry = struct.unpack_from("<H", buf, 10)[0]
                    lt = struct.unpack_from("<H", buf, 12)[0]
                    rt = struct.unpack_from("<H", buf, 14)[0]

                    if btns != prev_gamepad_btns or hat != 8:
                        prev_gamepad_btns = btns
                        btn_names = [name for bit, name in XBOX_BUTTON_NAMES if btns & bit]
                        print(f"[Col01] btns=0x{btns:04X} ({' '.join(btn_names) or 'none'}), "
                              f"hat={hat} ({dpad}), "
                              f"LX={lx} LY={ly} RX={rx} RY={ry} LT={lt} RT={rt}")

            # Read consumer
            buf = cs.read(64, timeout_ms=5)
            if buf:
                buf = bytes(buf)
                if len(buf) >= 7 and buf[0] == REPORT_ID_CONSUMER:
                    curr_usages = set()
                    for i in range(3):
                        u = buf[1 + i * 2] | (buf[2 + i * 2] << 8)
                        if u:
                            curr_usages.add(u)

                    if curr_usages != prev_consumer_usages:
                        prev_consumer_usages = curr_usages
                        usage_names = []
                        for u in curr_usages:
                            name = CONSUMER_USAGE_NAMES.get(u)
                            if name:
                                usage_names.append(f"0x{u:04X}({name})")
                            else:
                                usage_names.append(f"0x{u:04X}(unknown)")
                        print(f"[Col03] usages: {', '.join(usage_names) or 'none'}")

    except KeyboardInterrupt:
        pass
    finally:
        gp.close()
        cs.close()


# ============================================================
# Debug: Vibration/LED Test
# ============================================================
def debug_output() -> None:
    """Debug mode: interactive vibration and LED tester."""
    gamepad_path = find_path(0x0001, 0x0005)
    if not gamepad_path:
        print("Gamepad interface (Col01) not found. Is controller connected?")
        sys.exit(1)

    dev = hid.device()
    dev.open_path(gamepad_path)

    print("Debug mode: Output command tester")
    print("Press Ctrl+C to exit.\n")

    try:
        while True:
            print("--- Vibration (Rumble, cmd=0x02) ---")
            print("  1  Left motor (large) pulse, full intensity")
            print("  2  Right motor (small) pulse, full intensity")
            print("  3  Both motors pulse, full intensity")
            print("  4  Stop all motors")
            print("  5  Left motor timed 1s")
            print("  6  Right motor timed 1s")
            print("  7  Both motors timed 1s")
            print("--- LED (cmd=0x01) ---")
            print("  r  Red LED on")
            print("  g  Green LED on")
            print("  b  Blue LED on")
            print("  w  White (R+G+B)")
            print("  h  HOME LED on")
            print("  c  Consumer area LED on")
            print("  o  All LED off")
            choice = input("Select command: ").strip()
            if not choice:
                continue

            cmd_char = choice[0]
            cmd = None

            if cmd_char == '1':
                cmd = bytes([REPORT_ID_OUTPUT, 0x02, 0x01, 0xFF, 0x00, 0, 0, 0, 0, 0, 0, 0, 0])
            elif cmd_char == '2':
                cmd = bytes([REPORT_ID_OUTPUT, 0x02, 0x04, 0, 0, 0xFF, 0x00, 0, 0, 0, 0, 0, 0])
            elif cmd_char == '3':
                cmd = bytes([REPORT_ID_OUTPUT, 0x02, 0x05, 0xFF, 0x00, 0xFF, 0x00, 0, 0, 0, 0, 0, 0])
            elif cmd_char == '4':
                cmd = bytes([REPORT_ID_OUTPUT, 0x02, 0x0A, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
            elif cmd_char == '5':
                cmd = bytes([REPORT_ID_OUTPUT, 0x02, 0x01, 0xFF, 0x04, 0, 0, 0, 0, 0, 0, 0, 0])
            elif cmd_char == '6':
                cmd = bytes([REPORT_ID_OUTPUT, 0x02, 0x04, 0, 0, 0xFF, 0x04, 0, 0, 0, 0, 0, 0])
            elif cmd_char == '7':
                cmd = bytes([REPORT_ID_OUTPUT, 0x02, 0x05, 0xFF, 0x04, 0xFF, 0x04, 0, 0, 0, 0, 0, 0])
            elif cmd_char == 'r':
                cmd = bytes([REPORT_ID_OUTPUT, 0x01, 0x01, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0, 0])
            elif cmd_char == 'g':
                cmd = bytes([REPORT_ID_OUTPUT, 0x01, 0, 0, 0x01, 0xFF, 0, 0, 0, 0, 0, 0, 0])
            elif cmd_char == 'b':
                cmd = bytes([REPORT_ID_OUTPUT, 0x01, 0, 0, 0, 0, 0x01, 0xFF, 0, 0, 0, 0, 0])
            elif cmd_char == 'w':
                cmd = bytes([REPORT_ID_OUTPUT, 0x01, 0x01, 0xFF, 0x01, 0xFF, 0x01, 0xFF, 0, 0, 0, 0, 0])
            elif cmd_char == 'h':
                cmd = bytes([REPORT_ID_OUTPUT, 0x01, 0, 0, 0, 0, 0, 0, 0x01, 0xFF, 0, 0, 0])
            elif cmd_char == 'c':
                cmd = bytes([REPORT_ID_OUTPUT, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0x01, 0xFF, 0])
            elif cmd_char == 'o':
                cmd = bytes([REPORT_ID_OUTPUT, 0x01, 0x02, 0x00, 0x02, 0x00, 0x02, 0x00, 0x02, 0x00, 0x02, 0x00, 0])
            else:
                print("Unknown command")
                continue

            print(f"Sending: {' '.join(f'{b:02X}' for b in cmd)}")
            dev.write(cmd)
            print()

    except KeyboardInterrupt:
        pass
    finally:
        dev.close()


# ============================================================
# CLI Help
# ============================================================
def print_usage():
    print(
        "OBOX Bluetooth Controller -> ViGEmBus Xbox360 (Python v1.0)\n"
        "\n"
        "Usage:\n"
        "  obox_middleware.py                     Run in CLI mode (default)\n"
        "  obox_middleware.py --cli               Run in CLI mode (explicit)\n"
        "  obox_middleware.py --hidhide-status    Show HidHide configuration\n"
        "  obox_middleware.py --hidhide-disable   Disable HidHide for OBOX\n"
        "  obox_middleware.py --debug-keys        Debug: real-time key input\n"
        "  obox_middleware.py --debug-output      Debug: vibration/LED test\n"
        "  obox_middleware.py -h, --help          Show this help\n"
        "\n"
        "Features:\n"
        "  - Col01 Gamepad + Col03 Consumer input forwarding\n"
        "  - ViGEmBus Xbox360 virtual controller\n"
        "  - Rumble callback (ViGEmBus -> physical gamepad)\n"
        "  - LED control (RGB, HOME, Consumer area)\n"
        "  - HidHide integration (auto-config)\n"
        "  - Bluetooth disconnect/reconnect handling"
    )


# ============================================================
# Main Session Loop (matching Rust's run_session)
# ============================================================
def run_session(gamepad) -> str:
    """
    Run one controller session. Returns disconnect reason string.
    This is called in a loop for auto-reconnect.
    """
    mac = get_mac_address()
    if mac:
        print(f"[Main] Controller MAC: {mac}")

    gamepad_path = find_path(0x0001, 0x0005)
    if not gamepad_path:
        raise RuntimeError("Controller not connected (Col01 gamepad interface not found)")

    consumer_path = find_path(0x000C, 0x0001)
    if not consumer_path:
        raise RuntimeError("Controller not connected (Col03 consumer interface not found)")

    state = ControllerState()
    rumble = RumbleHandler()

    output_dev = hid.device()
    output_dev.open_path(gamepad_path)
    rumble.set_output_device(output_dev)
    rumble.start()

    gp_dev = hid.device()
    gp_dev.open_path(gamepad_path)

    cs_dev = hid.device()
    cs_dev.open_path(consumer_path)

    kb = KeyboardReader(state)

    consumer_thread = ConsumerReader(consumer_path, state, debug=False)
    consumer_thread.start()

    output = OutputThread(gamepad, state)
    output.start()

    gamepad.register_notification(_make_rumble_callback(rumble))
    print("[Rumble] Notification callback registered")

    print(f"[Main] Session started. Waiting for input...")

    disconnect_reason = ""
    buf = bytearray(64)
    try:
        while True:
            # Read gamepad
            try:
                data = gp_dev.read(64, timeout_ms=5)
                if data:
                    data = bytes(data)
                    if len(data) >= 16 and data[0] == REPORT_ID_GAMEPAD:
                        # Buttons: 16-bit bitmap (bytes 1-2, little-endian)
                        buttons = data[1] | (data[2] << 8)
                        # Hat: lower 4 bits of byte 3
                        hat = data[3] & 0x0F

                        btns = 0
                        if buttons & (1 << 0):  btns |= XBUTTON_A
                        if buttons & (1 << 1):  btns |= XBUTTON_B
                        if buttons & (1 << 3):  btns |= XBUTTON_X
                        if buttons & (1 << 4):  btns |= XBUTTON_Y
                        if buttons & (1 << 6):  btns |= XBUTTON_LB
                        if buttons & (1 << 7):  btns |= XBUTTON_RB
                        if buttons & (1 << 13): btns |= XBUTTON_L3
                        if buttons & (1 << 14): btns |= XBUTTON_R3

                        dpad = 0
                        if hat == 0: dpad = XBUTTON_UP
                        elif hat == 1: dpad = XBUTTON_UP | XBUTTON_RIGHT
                        elif hat == 2: dpad = XBUTTON_RIGHT
                        elif hat == 3: dpad = XBUTTON_RIGHT | XBUTTON_DOWN
                        elif hat == 4: dpad = XBUTTON_DOWN
                        elif hat == 5: dpad = XBUTTON_DOWN | XBUTTON_LEFT
                        elif hat == 6: dpad = XBUTTON_LEFT
                        elif hat == 7: dpad = XBUTTON_LEFT | XBUTTON_UP

                        btns |= dpad

                        lx_raw = struct.unpack_from("<H", data, 4)[0]
                        ly_raw = struct.unpack_from("<H", data, 6)[0]
                        rx_raw = struct.unpack_from("<H", data, 8)[0]
                        ry_raw = struct.unpack_from("<H", data, 10)[0]
                        lt_raw = struct.unpack_from("<H", data, 12)[0]
                        rt_raw = struct.unpack_from("<H", data, 14)[0]

                        lx = apply_deadzone(lx_raw)
                        ly = apply_deadzone_y(ly_raw)
                        rx = apply_deadzone(rx_raw)
                        ry = apply_deadzone_y(ry_raw)
                        lt = apply_trigger_deadzone(lt_raw)
                        rt = apply_trigger_deadzone(rt_raw)

                        state.update(
                            gamepad_buttons=btns,
                            lt=lt, rt=rt,
                            thumb_lx=lx, thumb_ly=ly,
                            thumb_rx=rx, thumb_ry=ry,
                        )
            except Exception as e:
                disconnect_reason = f"Col01 read error: {e}"
                break

            # Yield GIL so ViGEmBus notification callback thread can execute.
            # hidapi's Cython wrapper does NOT release the GIL during hid_read,
            # so an explicit sleep is required here.
            time.sleep(0.001)

            if not consumer_thread.is_alive():
                disconnect_reason = "Col03 (consumer) thread died"
                break

    except KeyboardInterrupt:
        disconnect_reason = "User interrupt"
    finally:
        print("[Main] Cleaning up...")

        output.running = False
        consumer_thread.running = False

        if disconnect_reason == "User interrupt":
            rumble._stop_event.set()
            os._exit(0)

        rumble.stop()

        def _close_devices():
            for dev in (gp_dev, cs_dev, output_dev):
                try:
                    dev.close()
                except Exception:
                    pass

        closer = threading.Thread(target=_close_devices, daemon=True)
        closer.start()
        closer.join(timeout=1.0)

    return disconnect_reason


def _make_rumble_callback(rumble_handler):
    """Create a rumble notification callback closure.

    vgamepad callback signature: func(client, target, large_motor, small_motor, led_number, user_data)
    """
    def callback(client, target, large_motor, small_motor, led_number, user_data):
        print(f"[Rumble] Callback: large={large_motor} small={small_motor}")
        rumble_handler.handle(large_motor, small_motor)
    return callback


# ============================================================
# Entry Point
# ============================================================
def main():
    parser = argparse.ArgumentParser(
        description="OBOX Bluetooth Controller -> ViGEmBus Xbox360",
        add_help=False,
    )
    parser.add_argument("--cli", action="store_true",
                        help="Run in CLI mode (default)")
    parser.add_argument("-h", "--help", action="store_true",
                        help="Show help")
    parser.add_argument("--hidhide-status", action="store_true",
                        help="Show HidHide configuration")
    parser.add_argument("--hidhide-disable", action="store_true",
                        help="Disable HidHide for OBOX")
    parser.add_argument("--debug-keys", action="store_true",
                        help="Debug: real-time key input")
    parser.add_argument("--debug-output", action="store_true",
                        help="Debug: vibration/LED test")
    parser.add_argument("--debug", action="store_true",
                        help="Print debug information (legacy)")
    args = parser.parse_args()

    # Dispatch to sub-commands
    if args.help:
        print_usage()
        return

    if args.hidhide_status:
        hidhide_print_status()
        return

    if args.hidhide_disable:
        hidhide_disable()
        return

    if args.debug_keys:
        debug_keys()
        return

    if args.debug_output:
        debug_output()
        return

    # Single instance check
    if not check_single_instance():
        print("Another instance is already running.")
        sys.exit(1)

    # HidHide registers the Python interpreter (python.exe), not the .py script,
    # because HidHide works at the process/executable level.
    app_path = sys.executable

    # Run CLI mode
    print("OBOX Bluetooth Controller -> ViGEmBus Xbox360 (Python v1.0)")
    print("=" * 55)

    # HidHide setup
    hidhide_ensure_enabled(app_path)
    print()

    # Connect to ViGEmBus
    try:
        gamepad = vg.VX360Gamepad()
        print("[ViGEm] Connected to ViGEmBus driver")
    except Exception as e:
        print(f"[ViGEm] ERROR: Failed to connect to ViGEmBus driver: {e}")
        print("  Please install ViGEmBus driver: https://github.com/ViGEm/ViGEmBus")
        sys.exit(1)
    print()

    # Session loop with reconnect (matching Rust's run_cli_mode)
    first_attempt = True
    last_reason = ""
    while True:
        try:
            last_reason = run_session(gamepad)
            if last_reason == "User interrupt":
                print("\n[Main] Session ended by user, exiting.")
                break
            if "not connected" in last_reason.lower():
                if first_attempt:
                    print("[Main] Controller not connected. Waiting for pairing...")
                else:
                    print("\r[Main] Waiting for controller... (retry in 3s)   ", end="", flush=True)
            else:
                print(f"\n[Main] Session ended: {last_reason}")
                print("[Main] Waiting 3s before reconnect...")
        except Exception as e:
            last_reason = str(e)
            print(f"\n[Main] Session error: {e}")
            print("[Main] Waiting 3s before reconnect...")

        first_attempt = False
        time.sleep(3)

        if "not connected" not in last_reason.lower():
            print("[Main] Attempting reconnect...")

    os._exit(0)


if __name__ == "__main__":
    main()
