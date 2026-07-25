//! OBOX 蓝牙手柄 → ViGEmBus Xbox360 中间件 (Rust 实现)
//!
//! 架构：
//! - 主线程：读 Col01 Gamepad input + dirty 驱动 ViGEmBus 输出
//! - Col03 子线程：读 Consumer 接口（Back/Menu/Home）
//! - 振动回调线程：ViGEmBus notification → HID Output Report 0xB3
//!
//! 三路输入合并到共享 ControllerState，仅当 dirty 时调用 gamepad.update()。

use anyhow::{Context, Result};
use std::collections::HashSet;
use std::ffi::CString;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use vigem_client::{Client, TargetId, XButtons, XGamepad, Xbox360Wired, XNotification};

mod hidhide;

const VENDOR_ID: u16 = 0x0A5C;
const PRODUCT_ID: u16 = 0x4502;

const STICK_CENTER: i32 = 32768;
const STICK_DEADZONE: i32 = 2000;
const TRIGGER_THRESHOLD: u16 = 100;

const REPORT_ID_GAMEPAD: u8 = 0x07;
const REPORT_ID_CONSUMER: u8 = 0x0A;
const REPORT_ID_OUTPUT: u8 = 0xB3;

// Consumer Usage Codes (Col03, Report ID 0x0A)
const USAGE_BACK: u16 = 0x224;
const USAGE_MENU: u16 = 0x040;
const USAGE_HOME: u16 = 0x223;

/// 共享控制器状态（多线程读写，由 Mutex 保护）。
struct ControllerState {
    /// 来自 Col01 的按键位图（A/B/X/Y/LB/RB/L3/R3 + D-pad）
    gamepad_buttons: u16,
    /// 来自 Col03 的按键位图（BACK/START/GUIDE）
    consumer_buttons: u16,
    lt: u8,
    rt: u8,
    thumb_lx: i16,
    thumb_ly: i16,
    thumb_rx: i16,
    thumb_ry: i16,
    dirty: bool,
}

impl Default for ControllerState {
    fn default() -> Self {
        Self {
            gamepad_buttons: 0,
            consumer_buttons: 0,
            lt: 0,
            rt: 0,
            thumb_lx: 0,
            thumb_ly: 0,
            thumb_rx: 0,
            thumb_ry: 0,
            dirty: true, // 启动时发送一次零状态
        }
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("");

    match cmd {
        "--help" | "-h" => {
            print_usage();
            return Ok(());
        }
        "--hidhide-status" => return hidhide::print_status(),
        "--hidhide-disable" => return hidhide::disable(),
        "--debug-keys" => return debug_keys(),
        "--debug-output" => return debug_output(),
        _ if !cmd.is_empty() && !cmd.starts_with('-') => {
            eprintln!("Unknown argument: {}", cmd);
            print_usage();
            std::process::exit(1);
        }
        _ => {}
    }

    println!("OBOX Bluetooth Controller -> ViGEmBus Xbox360 (Rust)");
    println!("==========================================================");

    // 1. HidHide 自动配置（幂等，含 cloak-on）
    //    失败不阻断启动，仅警告（用户可能未装 HidHide）
    if let Err(e) = hidhide::ensure_enabled() {
        eprintln!("[HidHide] WARN: {}", e);
        eprintln!("[HidHide] Continuing without HidHide...");
    }
    println!();

    // 2. ViGEmBus 连接
    let client = Client::connect().context("Failed to connect to ViGEmBus driver")?;
    let target_id = TargetId::XBOX360_WIRED;
    let mut gamepad = Xbox360Wired::new(client, target_id);
    gamepad
        .plugin()
        .context("Failed to plug in virtual Xbox360 controller")?;
    gamepad
        .wait_ready()
        .context("Virtual controller not ready")?;
    println!("[ViGEm] Virtual Xbox360 controller plugged in");

    // 2. HID 初始化 + 路径查找
    let hid = hidapi::HidApi::new().context("Failed to initialize hidapi")?;

    let gamepad_path = find_path(&hid, 0x0001, 0x0005)
        .context("Gamepad interface (Col01) not found. Is the controller connected?")?;
    let consumer_path = find_path(&hid, 0x000C, 0x0001)
        .context("Consumer interface (Col03) not found")?;
    println!("[HID] Col01 Gamepad + Col03 Consumer interfaces found");

    let gamepad_path_cstr = CString::new(gamepad_path.as_bytes())
        .context("Invalid gamepad device path")?;
    let consumer_path_cstr = CString::new(consumer_path.as_bytes())
        .context("Invalid consumer device path")?;

    // 3. 打开 3 个独立的 device handle（避免读/写锁竞争）
    // Col01 读 handle
    let gamepad_device = hid
        .open_path(&gamepad_path_cstr)
        .context("Failed to open Col01 gamepad device")?;
    gamepad_device.set_blocking_mode(false).ok();

    // Output 写 handle（振动），同一 path 再次 open
    let output_device = hid
        .open_path(&gamepad_path_cstr)
        .context("Failed to open output device handle")?;
    let output_device = Arc::new(Mutex::new(output_device));

    // Col03 读 handle
    let consumer_device = hid
        .open_path(&consumer_path_cstr)
        .context("Failed to open Col03 consumer device")?;
    consumer_device.set_blocking_mode(false).ok();
    println!("[HID] All device handles opened (Col01 read, Col03 read, output write)");

    // 4. 共享状态
    let state = Arc::new(Mutex::new(ControllerState::default()));

    // 5. 振动回调（ViGEmBus notification → HID Output Report）
    // XNotification.large_motor = 左马达（大），small_motor = 右马达（小）
    let rumble_last = Arc::new(Mutex::new((0u8, 0u8))); // 上次发送的 (large, small)
    let output_for_rumble = output_device.clone();
    let rumble_last_cb = rumble_last.clone();
    let _rumble_thread = gamepad
        .request_notification()
        .context("Failed to request ViGEmBus notification")?
        .spawn_thread(move |_, notif: XNotification| {
            handle_rumble(
                &output_for_rumble,
                &rumble_last_cb,
                notif.large_motor,
                notif.small_motor,
            );
        });
    println!("[ViGEm] Rumble callback registered (large=left motor, small=right motor)");

    // 6. Col03 Consumer 线程
    let state_for_consumer = state.clone();
    std::thread::Builder::new()
        .name("col03-consumer".into())
        .spawn(move || {
            consumer_thread(consumer_device, state_for_consumer);
        })
        .context("Failed to spawn Col03 consumer thread")?;
    println!("[HID] Col03 Consumer thread started");

    println!("\nReady. Press Ctrl+C to exit.\n");

    // 7. 主线程：Col01 读循环 + dirty 驱动输出
    let mut buf = [0u8; 64];
    loop {
        let n = gamepad_device.read_timeout(&mut buf, 5).unwrap_or(0);
        if n >= 16 && buf[0] == REPORT_ID_GAMEPAD {
            update_gamepad_state(&state, &buf);
        }

        // dirty 驱动输出：仅当状态变化时调用 gamepad.update()
        let report_opt = {
            let mut s = state.lock().unwrap();
            if s.dirty {
                s.dirty = false;
                Some(XGamepad {
                    buttons: XButtons {
                        raw: s.gamepad_buttons | s.consumer_buttons,
                    },
                    left_trigger: s.lt,
                    right_trigger: s.rt,
                    thumb_lx: s.thumb_lx,
                    thumb_ly: s.thumb_ly,
                    thumb_rx: s.thumb_rx,
                    thumb_ry: s.thumb_ry,
                })
            } else {
                None
            }
        };
        if let Some(r) = report_opt {
            if let Err(e) = gamepad.update(&r) {
                eprintln!("[ViGEm] update error: {:?}", e);
            }
        }
    }
}

/// Col03 Consumer 线程主循环：读取 Consumer 接口，用集合差集检测按下/释放。
///
/// 协议见 HID_PROTOCOL.md §3：3 个 usage slot，每次上报当前按下的 Usage 集合。
/// 用集合差集避免槽位顺序变化导致的误触发。
fn consumer_thread(device: hidapi::HidDevice, state: Arc<Mutex<ControllerState>>) {
    let mut prev_usages: HashSet<u16> = HashSet::new();
    let mut buf = [0u8; 64];
    loop {
        let n = device.read_timeout(&mut buf, 100).unwrap_or(0);
        if n < 7 || buf[0] != REPORT_ID_CONSUMER {
            continue;
        }
        let curr_usages: HashSet<u16> = [
            u16::from_le_bytes([buf[1], buf[2]]),
            u16::from_le_bytes([buf[3], buf[4]]),
            u16::from_le_bytes([buf[5], buf[6]]),
        ]
        .into_iter()
        .filter(|&u| u != 0)
        .collect();

        let pressed = curr_usages.difference(&prev_usages);
        let released = prev_usages.difference(&curr_usages);

        {
            let mut s = state.lock().unwrap();
            for u in pressed {
                if let Some(btn) = consumer_usage_to_xbox(*u) {
                    s.consumer_buttons |= btn;
                    s.dirty = true;
                }
            }
            for u in released {
                if let Some(btn) = consumer_usage_to_xbox(*u) {
                    s.consumer_buttons &= !btn;
                    s.dirty = true;
                }
            }
        }

        prev_usages = curr_usages;
    }
}

/// ViGEmBus 振动反馈 → HID Output Report (Report ID 0xB3, cmd 0x02)。
///
/// 协议见 HID_PROTOCOL.md §7：
/// - byte[1] = 0x02 (Rumble command)
/// - byte[2] = motor enable (bits[1:0]=left, bits[3:2]=right; 01=start, 10=stop)
/// - byte[3] = left intensity, byte[4] = left duration (0=continuous)
/// - byte[5] = right intensity, byte[6] = right duration (0=continuous)
fn handle_rumble(
    output: &Arc<Mutex<hidapi::HidDevice>>,
    last: &Arc<Mutex<(u8, u8)>>,
    large: u8,
    small: u8,
) {
    // 去重：值未变则不发送（避免游戏每帧触发 HID 写入）
    {
        let mut last_val = last.lock().unwrap();
        if last_val.0 == large && last_val.1 == small {
            return;
        }
        *last_val = (large, small);
    }

    let mut cmd = [0u8; 13];
    cmd[0] = REPORT_ID_OUTPUT; // 0xB3
    cmd[1] = 0x02; // Rumble command

    if large == 0 && small == 0 {
        // 双马达停止
        cmd[2] = 0x0A; // bits=10 for both motors (stop)
    } else {
        // 启动相应马达（持续振动，duration=0）
        let mut enable: u8 = 0;
        if large > 0 {
            enable |= 0x01; // 左马达 start (bits[1:0]=01)
        }
        if small > 0 {
            enable |= 0x04; // 右马达 start (bits[3:2]=01)
        }
        cmd[2] = enable;
        cmd[3] = large; // 左马达强度
        cmd[4] = 0; // 左马达持续时间 (0 = 持续振动直到 stop)
        cmd[5] = small; // 右马达强度
        cmd[6] = 0; // 右马达持续时间
    }

    if let Ok(dev) = output.lock() {
        if let Err(e) = dev.write(&cmd) {
            eprintln!("[HID] rumble write error: {}", e);
        }
    }
}

/// 从 Col01 input report (Report ID 0x07, 16 bytes) 更新共享状态。
fn update_gamepad_state(state: &Arc<Mutex<ControllerState>>, buf: &[u8]) {
    let buttons = u16::from_le_bytes([buf[1], buf[2]]);
    let hat = buf[3] & 0x0F;
    let lx = u16::from_le_bytes([buf[4], buf[5]]);
    let ly = u16::from_le_bytes([buf[6], buf[7]]);
    let rx = u16::from_le_bytes([buf[8], buf[9]]);
    let ry = u16::from_le_bytes([buf[10], buf[11]]);
    let l2 = u16::from_le_bytes([buf[12], buf[13]]);
    let r2 = u16::from_le_bytes([buf[14], buf[15]]);

    let mut xb_raw: u16 = 0;
    for bit in 0..16 {
        if (buttons >> bit) & 1 != 0 {
            if let Some(btn) = button_bit(bit + 1) {
                xb_raw |= btn;
            }
        }
    }
    if hat < 8 {
        for dpad in hat_buttons(hat) {
            xb_raw |= dpad;
        }
    }

    let lx_f = stick_to_float(lx as i32, false);
    let ly_f = stick_to_float(ly as i32, true);
    let rx_f = stick_to_float(rx as i32, false);
    let ry_f = stick_to_float(ry as i32, true);
    let lt = trigger_to_byte(l2);
    let rt = trigger_to_byte(r2);

    let mut s = state.lock().unwrap();
    s.gamepad_buttons = xb_raw;
    s.lt = lt;
    s.rt = rt;
    s.thumb_lx = float_to_short(lx_f);
    s.thumb_ly = float_to_short(ly_f);
    s.thumb_rx = float_to_short(rx_f);
    s.thumb_ry = float_to_short(ry_f);
    s.dirty = true;
}

/// 按 usage_page + usage 查找 HID 设备路径。
fn find_path(hid: &hidapi::HidApi, usage_page: u16, usage: u16) -> Option<String> {
    for dev in hid.device_list() {
        if dev.vendor_id() == VENDOR_ID
            && dev.product_id() == PRODUCT_ID
            && dev.usage_page() == usage_page
            && dev.usage() == usage
        {
            return Some(dev.path().to_string_lossy().into_owned());
        }
    }
    None
}

/// Col01 button bit → Xbox360 button. 物理键编号 = bit 位 + 1。
fn button_bit(bit_num: u8) -> Option<u16> {
    match bit_num {
        1 => Some(XButtons::A),
        2 => Some(XButtons::B),
        4 => Some(XButtons::X),
        5 => Some(XButtons::Y),
        7 => Some(XButtons::LB),
        8 => Some(XButtons::RB),
        14 => Some(XButtons::LTHUMB),
        15 => Some(XButtons::RTHUMB),
        _ => None,
    }
}

/// Col03 Consumer Usage → Xbox360 button.
fn consumer_usage_to_xbox(usage: u16) -> Option<u16> {
    match usage {
        USAGE_BACK => Some(XButtons::BACK),
        USAGE_MENU => Some(XButtons::START),
        USAGE_HOME => Some(XButtons::GUIDE),
        _ => None,
    }
}

/// Hat switch → D-pad button(s). 支持斜向（同时按两个方向）。
fn hat_buttons(hat: u8) -> &'static [u16] {
    match hat {
        0 => &[XButtons::UP],
        1 => &[XButtons::UP, XButtons::RIGHT],
        2 => &[XButtons::RIGHT],
        3 => &[XButtons::DOWN, XButtons::RIGHT],
        4 => &[XButtons::DOWN],
        5 => &[XButtons::DOWN, XButtons::LEFT],
        6 => &[XButtons::LEFT],
        7 => &[XButtons::UP, XButtons::LEFT],
        _ => &[],
    }
}

/// 摇杆原始值 → float (-1.0 ~ 1.0)，带死区。invert=true 时反转 Y 轴。
fn stick_to_float(val: i32, invert: bool) -> f32 {
    let delta = val - STICK_CENTER;
    if delta.abs() < STICK_DEADZONE {
        return 0.0;
    }
    let res = delta as f32 / STICK_CENTER as f32;
    if invert {
        -res
    } else {
        res
    }
}

/// 扳机原始值 (0~65535) → Xbox360 扳机字节 (0~255)，带阈值。
fn trigger_to_byte(val: u16) -> u8 {
    if val < TRIGGER_THRESHOLD {
        return 0;
    }
    (val / 257).min(255) as u8
}

/// float (-1.0 ~ 1.0) → i16 (-32768 ~ 32767)。
fn float_to_short(v: f32) -> i16 {
    (v.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
}

// ============================================================================
// Debug 模式：按键调试 + 振动/LED 测试
// ============================================================================

fn print_usage() {
    eprintln!("OBOX Bluetooth Controller -> ViGEmBus Xbox360 (Rust)");
    eprintln!();
    eprintln!("Usage: obox-controller-driver [OPTION]");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  (no option)        Run driver (auto-enable HidHide, then forward input to ViGEmBus)");
    eprintln!("  --hidhide-status   Show current HidHide configuration (cloak / app / devices)");
    eprintln!("  --hidhide-disable  Unhide OBOX device from HidHide (keeps global cloak state)");
    eprintln!("  --debug-keys       Print real-time Col01/Col03 input (no ViGEmBus)");
    eprintln!("  --debug-output     Interactive vibration/LED test menu (no ViGEmBus)");
    eprintln!("  -h, --help         Show this help");
}

/// 打开 Col01 + Col03 设备，返回 (gamepad_device, consumer_device)。
fn open_debug_devices() -> Result<(hidapi::HidDevice, hidapi::HidDevice)> {
    let hid = hidapi::HidApi::new().context("Failed to initialize hidapi")?;
    let gp_path = find_path(&hid, 0x0001, 0x0005).context("Col01 gamepad interface not found")?;
    let cs_path = find_path(&hid, 0x000C, 0x0001).context("Col03 consumer interface not found")?;
    let gp_cstr = CString::new(gp_path.as_bytes()).context("Invalid gamepad path")?;
    let cs_cstr = CString::new(cs_path.as_bytes()).context("Invalid consumer path")?;
    let gp = hid.open_path(&gp_cstr).context("Failed to open Col01")?;
    let cs = hid.open_path(&cs_cstr).context("Failed to open Col03")?;
    gp.set_blocking_mode(false).ok();
    cs.set_blocking_mode(false).ok();
    Ok((gp, cs))
}

/// debug-keys 模式：实时打印 Col01/Col03 输入。
fn debug_keys() -> Result<()> {
    println!("=== Key Debug Mode ===");
    println!("Prints Col01 (gamepad) and Col03 (consumer) input in real time.");
    println!("Press Ctrl+C to exit.\n");

    let (gp, cs) = open_debug_devices()?;
    println!("[HID] Col01 + Col03 opened\n");

    let mut gp_buf = [0u8; 64];
    let mut cs_buf = [0u8; 64];
    // 用 sentinel 初值强制首帧打印
    let mut prev_btns: u16 = 0xFFFF;
    let mut prev_hat: u8 = 0xFF;
    let mut prev_lt: u16 = 0xFFFF;
    let mut prev_rt: u16 = 0xFFFF;
    let mut prev_lx: u16 = 0xFFFF;
    let mut prev_ly: u16 = 0xFFFF;
    let mut prev_rx: u16 = 0xFFFF;
    let mut prev_ry: u16 = 0xFFFF;
    let mut prev_cs: HashSet<u16> = HashSet::new();

    loop {
        // Col01 gamepad
        let n = gp.read_timeout(&mut gp_buf, 10).unwrap_or(0);
        if n >= 16 && gp_buf[0] == REPORT_ID_GAMEPAD {
            let btns = u16::from_le_bytes([gp_buf[1], gp_buf[2]]);
            let hat = gp_buf[3] & 0x0F;
            let lx = u16::from_le_bytes([gp_buf[4], gp_buf[5]]);
            let ly = u16::from_le_bytes([gp_buf[6], gp_buf[7]]);
            let rx = u16::from_le_bytes([gp_buf[8], gp_buf[9]]);
            let ry = u16::from_le_bytes([gp_buf[10], gp_buf[11]]);
            let lt = u16::from_le_bytes([gp_buf[12], gp_buf[13]]);
            let rt = u16::from_le_bytes([gp_buf[14], gp_buf[15]]);
            if btns != prev_btns
                || hat != prev_hat
                || lt != prev_lt
                || rt != prev_rt
                || lx != prev_lx
                || ly != prev_ly
                || rx != prev_rx
                || ry != prev_ry
            {
                let names = gamepad_button_names(btns);
                let hat_name = if hat < 8 { hat_direction(hat) } else { "released" };
                println!(
                    "[Col01] btns=0x{:04X} ({}) hat={} ({}) LX={} LY={} RX={} RY={} LT={} RT={}",
                    btns, names, hat, hat_name, lx, ly, rx, ry, lt, rt
                );
                prev_btns = btns;
                prev_hat = hat;
                prev_lt = lt;
                prev_rt = rt;
                prev_lx = lx;
                prev_ly = ly;
                prev_rx = rx;
                prev_ry = ry;
            }
        }

        // Col03 consumer
        let n = cs.read_timeout(&mut cs_buf, 10).unwrap_or(0);
        if n >= 7 && cs_buf[0] == REPORT_ID_CONSUMER {
            let curr: HashSet<u16> = [
                u16::from_le_bytes([cs_buf[1], cs_buf[2]]),
                u16::from_le_bytes([cs_buf[3], cs_buf[4]]),
                u16::from_le_bytes([cs_buf[5], cs_buf[6]]),
            ]
            .into_iter()
            .filter(|&u| u != 0)
            .collect();
            if curr != prev_cs {
                if curr.is_empty() {
                    println!("[Col03] usages: (empty/released)");
                } else {
                    let names: Vec<String> = curr
                        .iter()
                        .map(|u| format!("0x{:04X}({})", u, consumer_usage_name(*u)))
                        .collect();
                    println!("[Col03] usages: {}", names.join(", "));
                }
                prev_cs = curr;
            }
        }
    }
}

/// debug-output 模式：交互式测试振动和 LED。
fn debug_output() -> Result<()> {
    println!("=== Output Debug Mode ===");
    println!("Tests vibration motors and LED zones (HID Output Report 0xB3).");
    println!("Press Ctrl+C to exit.\n");

    let hid = hidapi::HidApi::new().context("Failed to initialize hidapi")?;
    let gp_path = find_path(&hid, 0x0001, 0x0005).context("Col01 gamepad interface not found")?;
    let gp_cstr = CString::new(gp_path.as_bytes()).context("Invalid gamepad path")?;
    let dev = hid.open_path(&gp_cstr).context("Failed to open Col01 for output")?;
    println!("[HID] Col01 device opened for output testing\n");

    let stdin = io::stdin();
    loop {
        println!("--- Vibration (Rumble, cmd=0x02) ---");
        println!("  1  Left motor (large) pulse, full intensity");
        println!("  2  Right motor (small) pulse, full intensity");
        println!("  3  Both motors pulse, full intensity");
        println!("  4  Stop all motors");
        println!("  5  Left motor timed 1s");
        println!("  6  Right motor timed 1s");
        println!("  7  Both motors timed 1s");
        println!("--- LED (cmd=0x01) ---");
        println!("  r  Red only      g  Green only     b  Blue only");
        println!("  w  White (R+G+B)");
        println!("  h  HOME LED on   c  Consumer area LED on");
        println!("  o  All LED off");
        println!("--- Misc ---");
        println!("  0  Exit");
        print!("> ");
        io::stdout().flush().ok();
        let mut line = String::new();
        stdin.read_line(&mut line)?;
        let cmd = line.trim().to_lowercase();
        let result = match cmd.as_str() {
            // Vibration
            "1" => send_rumble(&dev, 0x01, 0xFF, 0, 0, 0),
            "2" => send_rumble(&dev, 0x04, 0, 0, 0xFF, 0),
            "3" => send_rumble(&dev, 0x05, 0xFF, 0, 0xFF, 0),
            "4" => send_rumble(&dev, 0x0A, 0, 0, 0, 0),
            "5" => send_rumble(&dev, 0x01, 0xFF, 4, 0, 0),
            "6" => send_rumble(&dev, 0x04, 0, 0, 0xFF, 4),
            "7" => send_rumble(&dev, 0x05, 0xFF, 4, 0xFF, 4),
            // LED
            "r" => send_led_red(&dev, 0xFF),
            "g" => send_led_green(&dev, 0xFF),
            "b" => send_led_blue(&dev, 0xFF),
            "w" => send_led_white(&dev),
            "h" => send_led_home(&dev, 0xFF),
            "c" => send_led_consumer(&dev, 0xFF),
            "o" => send_led_all_off(&dev),
            // Exit
            "0" | "exit" | "quit" | "q" => break,
            "" => continue,
            other => {
                println!("Unknown command: {}", other);
                continue;
            }
        };
        if let Err(e) = result {
            eprintln!("[ERROR] {}", e);
        }
    }
    Ok(())
}

/// 发送振动命令（Report ID 0xB3, cmd=0x02）。
///
/// 协议见 HID_PROTOCOL.md §7（注：文档偏移表有歧义，以代码为准）：
/// - cmd[0] = 0xB3 (Report ID)
/// - cmd[1] = 0x02 (Rumble 命令类型)
/// - cmd[2] = 马达使能 (bits[1:0]=左, bits[3:2]=右; 01=start, 10=stop)
/// - cmd[3] = 左马达强度 (脉冲模式有效)
/// - cmd[4] = 左马达持续时间 (×0.25s, 0=脉冲/持续)
/// - cmd[5] = 右马达强度
/// - cmd[6] = 右马达持续时间
fn send_rumble(
    dev: &hidapi::HidDevice,
    enable: u8,
    left_intensity: u8,
    left_duration: u8,
    right_intensity: u8,
    right_duration: u8,
) -> Result<()> {
    let mut cmd = [0u8; 13];
    cmd[0] = REPORT_ID_OUTPUT; // 0xB3
    cmd[1] = 0x02; // Rumble 命令类型
    cmd[2] = enable;
    cmd[3] = left_intensity;
    cmd[4] = left_duration;
    cmd[5] = right_intensity;
    cmd[6] = right_duration;
    dev.write(&cmd).context("HID write rumble failed")?;
    println!(
        "[Rumble] B3 02 {:02X} {:02X} {:02X} {:02X} {:02X} 00 00 00 00 00 00",
        enable, left_intensity, left_duration, right_intensity, right_duration
    );
    Ok(())
}

/// 发送 LED 命令（Report ID 0xB3, cmd=0x01）。
///
/// 协议见 HID_PROTOCOL.md §6（按 §7.5 描述，LED 命令也有 cmd[1]=0x01 命令类型字节）：
/// - cmd[0] = 0xB3 (Report ID)
/// - cmd[1] = 0x01 (LED 命令类型)
/// - cmd[2..3] = 红色 (操作码, 亮度)
/// - cmd[4..5] = 绿色
/// - cmd[6..7] = 蓝色
/// - cmd[8..9] = HOME 灯
/// - cmd[10..11] = 消费区灯
/// - cmd[12] = 未用
///
/// 操作码：0x00=无操作, 0x01=设置亮度, 0x02=关闭
fn send_led_raw(dev: &hidapi::HidDevice, zones: &[(u8, u8); 5]) -> Result<()> {
    let mut cmd = [0u8; 13];
    cmd[0] = REPORT_ID_OUTPUT; // 0xB3
    cmd[1] = 0x01; // LED 命令类型
    for (i, (op, bright)) in zones.iter().enumerate() {
        cmd[2 + i * 2] = *op;
        cmd[3 + i * 2] = *bright;
    }
    dev.write(&cmd).context("HID write LED failed")?;
    let bytes: Vec<String> = cmd.iter().map(|b| format!("{:02X}", b)).collect();
    println!("[LED] {}", bytes.join(" "));
    Ok(())
}

fn send_led_red(dev: &hidapi::HidDevice, brightness: u8) -> Result<()> {
    // 红色开，其他通道关闭
    send_led_raw(
        dev,
        &[(0x01, brightness), (0x02, 0), (0x02, 0), (0x00, 0), (0x00, 0)],
    )
}

fn send_led_green(dev: &hidapi::HidDevice, brightness: u8) -> Result<()> {
    send_led_raw(
        dev,
        &[(0x02, 0), (0x01, brightness), (0x02, 0), (0x00, 0), (0x00, 0)],
    )
}

fn send_led_blue(dev: &hidapi::HidDevice, brightness: u8) -> Result<()> {
    send_led_raw(
        dev,
        &[(0x02, 0), (0x02, 0), (0x01, brightness), (0x00, 0), (0x00, 0)],
    )
}

fn send_led_white(dev: &hidapi::HidDevice) -> Result<()> {
    // R+G+B 全开 = 白色
    send_led_raw(
        dev,
        &[(0x01, 0xFF), (0x01, 0xFF), (0x01, 0xFF), (0x00, 0), (0x00, 0)],
    )
}

fn send_led_all_off(dev: &hidapi::HidDevice) -> Result<()> {
    send_led_raw(
        dev,
        &[(0x02, 0), (0x02, 0), (0x02, 0), (0x02, 0), (0x02, 0)],
    )
}

fn send_led_home(dev: &hidapi::HidDevice, brightness: u8) -> Result<()> {
    let op = if brightness == 0 { 0x02 } else { 0x01 };
    send_led_raw(dev, &[(0x00, 0), (0x00, 0), (0x00, 0), (op, brightness), (0x00, 0)])
}

fn send_led_consumer(dev: &hidapi::HidDevice, brightness: u8) -> Result<()> {
    let op = if brightness == 0 { 0x02 } else { 0x01 };
    send_led_raw(dev, &[(0x00, 0), (0x00, 0), (0x00, 0), (0x00, 0), (op, brightness)])
}

/// Col01 button bit 位图 → 可读名称（A/B/X/Y/LB/RB/L3/R3）。
fn gamepad_button_names(btns: u16) -> String {
    let mut names = Vec::new();
    if btns & XButtons::A != 0 {
        names.push("A");
    }
    if btns & XButtons::B != 0 {
        names.push("B");
    }
    if btns & XButtons::X != 0 {
        names.push("X");
    }
    if btns & XButtons::Y != 0 {
        names.push("Y");
    }
    if btns & XButtons::LB != 0 {
        names.push("LB");
    }
    if btns & XButtons::RB != 0 {
        names.push("RB");
    }
    if btns & XButtons::LTHUMB != 0 {
        names.push("L3");
    }
    if btns & XButtons::RTHUMB != 0 {
        names.push("R3");
    }
    if names.is_empty() {
        "(none)".into()
    } else {
        names.join(",")
    }
}

/// Hat switch 值 → 方向名称。
fn hat_direction(hat: u8) -> &'static str {
    match hat {
        0 => "N",
        1 => "NE",
        2 => "E",
        3 => "SE",
        4 => "S",
        5 => "SW",
        6 => "W",
        7 => "NW",
        _ => "?",
    }
}

/// Col03 Consumer Usage → 可读名称。
fn consumer_usage_name(usage: u16) -> &'static str {
    match usage {
        USAGE_BACK => "BACK",
        USAGE_MENU => "MENU/START",
        USAGE_HOME => "HOME/GUIDE",
        _ => "unknown",
    }
}
