use anyhow::{Context, Result};
use std::time::Duration;
use vigem_client::{Client, TargetId, XGamepad, XButtons, Xbox360Wired};

const VENDOR_ID: u16 = 0x0A5C;
const PRODUCT_ID: u16 = 0x4502;

const STICK_CENTER: i32 = 32768;
const STICK_DEADZONE: i32 = 2000;
const TRIGGER_THRESHOLD: u16 = 100;

const REPORT_ID_GAMEPAD: u8 = 0x07;

fn main() -> Result<()> {
    println!("OBOX Bluetooth Controller -> ViGEmBus Xbox360 (Rust MVP)");
    println!("==========================================================");

    let client = Client::connect().context("Failed to connect to ViGEmBus driver")?;
    println!("[ViGEm] Connected");

    let target_id = TargetId::XBOX360_WIRED;
    let mut gamepad = Xbox360Wired::new(client, target_id);
    gamepad.plugin().context("Failed to plug in virtual Xbox360 controller")?;
    gamepad.wait_ready().context("Virtual controller not ready")?;
    println!("[ViGEm] Virtual Xbox360 controller plugged in");

    let hid = hidapi::HidApi::new().context("Failed to initialize hidapi")?;

    let gamepad_path = find_gamepad_path(&hid)
        .context("Gamepad interface not found. Is the controller connected?")?;
    println!("[HID] Gamepad interface found");

    let device = hid.open_path(&gamepad_path)
        .context("Failed to open gamepad HID device")?;
    device.set_blocking_mode(false).ok();
    println!("[HID] Gamepad device opened");

    let mut buf = [0u8; 64];

    println!("\nReady. Press Ctrl+C to exit.\n");

    loop {
        let n = device.read_timeout(&mut buf, 100)
            .unwrap_or(0);
        if n == 0 {
            continue;
        }
        if n < 16 || buf[0] != REPORT_ID_GAMEPAD {
            continue;
        }

        let buttons = u16::from_le_bytes([buf[1], buf[2]]);
        let hat = buf[3] & 0x0F;
        let lx = u16::from_le_bytes([buf[4], buf[5]]);
        let ly = u16::from_le_bytes([buf[6], buf[7]]);
        let rx = u16::from_le_bytes([buf[8], buf[9]]);
        let ry = u16::from_le_bytes([buf[10], buf[11]]);
        let l2 = u16::from_le_bytes([buf[12], buf[13]]);
        let r2 = u16::from_le_bytes([buf[14], buf[15]]);

        let mut xb = XButtons::from_bits_truncate(0);

        for bit in 0..16 {
            if (buttons >> bit) & 1 != 0 {
                if let Some(btn) = button_map(bit + 1) {
                    xb |= btn;
                }
            }
        }

        if hat < 8 {
            for dpad_btn in hat_map(hat) {
                xb |= dpad_btn;
            }
        }

        let lx_f = stick_to_float(lx as i32, false);
        let ly_f = stick_to_float(ly as i32, true);
        let rx_f = stick_to_float(rx as i32, false);
        let ry_f = stick_to_float(ry as i32, true);

        let lt = trigger_to_byte(l2);
        let rt = trigger_to_byte(r2);

        let report = XGamepad {
            buttons: xb,
            left_trigger: lt,
            right_trigger: rt,
            thumb_lx: float_to_short(lx_f),
            thumb_ly: float_to_short(ly_f),
            thumb_rx: float_to_short(rx_f),
            thumb_ry: float_to_short(ry_f),
        };

        gamepad.update(&report).ok();

        std::thread::sleep(Duration::from_millis(1));
    }
}

fn find_gamepad_path(hid: &hidapi::HidApi) -> Option<hidapi::HidDeviceInfo> {
    for dev in hid.device_list() {
        if dev.vendor_id() == VENDOR_ID
            && dev.product_id() == PRODUCT_ID
            && dev.usage_page() == 0x0001
            && dev.usage() == 0x0005
        {
            return Some(dev.clone());
        }
    }
    None
}

fn button_map(bit_num: u8) -> Option<XButtons> {
    match bit_num {
        1  => Some(XButtons::A),
        2  => Some(XButtons::B),
        4  => Some(XButtons::X),
        5  => Some(XButtons::Y),
        7  => Some(XButtons::LEFT_SHOULDER),
        8  => Some(XButtons::RIGHT_SHOULDER),
        14 => Some(XButtons::LEFT_THUMB),
        15 => Some(XButtons::RIGHT_THUMB),
        _  => None,
    }
}

fn hat_map(hat: u8) -> &'static [XButtons] {
    match hat {
        0 => &[XButtons::DPAD_UP],
        1 => &[XButtons::DPAD_UP, XButtons::DPAD_RIGHT],
        2 => &[XButtons::DPAD_RIGHT],
        3 => &[XButtons::DPAD_DOWN, XButtons::DPAD_RIGHT],
        4 => &[XButtons::DPAD_DOWN],
        5 => &[XButtons::DPAD_DOWN, XButtons::DPAD_LEFT],
        6 => &[XButtons::DPAD_LEFT],
        7 => &[XButtons::DPAD_UP, XButtons::DPAD_LEFT],
        _ => &[],
    }
}

fn stick_to_float(val: i32, invert: bool) -> f32 {
    let delta = val - STICK_CENTER;
    if delta.abs() < STICK_DEADZONE {
        return 0.0;
    }
    let res = delta as f32 / STICK_CENTER as f32;
    if invert { -res } else { res }
}

fn trigger_to_byte(val: u16) -> u8 {
    if val < TRIGGER_THRESHOLD {
        return 0;
    }
    (val / 257).min(255) as u8
}

fn float_to_short(v: f32) -> i16 {
    (v.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
}
