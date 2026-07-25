use anyhow::{Context, Result};
use std::collections::HashSet;
use std::ffi::CString;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use vigem_client::{Client, TargetId, XGamepad, Xbox360Wired, XNotification};

mod hidhide;
mod tray;

const VENDOR_ID: u16 = 0x0A5C;
const PRODUCT_ID: u16 = 0x4502;

const STICK_CENTER: i32 = 32768;
const STICK_DEADZONE: i32 = 2000;
const TRIGGER_THRESHOLD: u16 = 100;

const REPORT_ID_GAMEPAD: u8 = 0x07;
const REPORT_ID_CONSUMER: u8 = 0x0A;
const REPORT_ID_OUTPUT: u8 = 0xB3;

const USAGE_BACK: u16 = 0x224;
const USAGE_MENU: u16 = 0x040;
const USAGE_HOME: u16 = 0x223;

const XBUTTON_UP: u16 = 0x0001;
const XBUTTON_DOWN: u16 = 0x0002;
const XBUTTON_LEFT: u16 = 0x0004;
const XBUTTON_RIGHT: u16 = 0x0008;
const XBUTTON_START: u16 = 0x0010;
const XBUTTON_BACK: u16 = 0x0020;
const XBUTTON_L3: u16 = 0x0040;
const XBUTTON_R3: u16 = 0x0080;
const XBUTTON_LB: u16 = 0x0100;
const XBUTTON_RB: u16 = 0x0200;
const XBUTTON_GUIDE: u16 = 0x0400;
const XBUTTON_A: u16 = 0x1000;
const XBUTTON_B: u16 = 0x2000;
const XBUTTON_X: u16 = 0x4000;
const XBUTTON_Y: u16 = 0x8000;

struct ControllerState {
    gamepad_buttons: u16,
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
            dirty: true,
        }
    }
}

#[cfg(windows)]
fn is_double_clicked() -> bool {
    use windows::Win32::System::Console::GetConsoleProcessList;
    
    let mut process_list = [0u32; 2];
    let count = unsafe { GetConsoleProcessList(&mut process_list) };
    count == 1
}

#[cfg(windows)]
fn check_single_instance() -> bool {
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::Win32::Foundation::ERROR_ALREADY_EXISTS;
    use windows::core::HSTRING;
    
    unsafe {
        let name = HSTRING::from("OBOXControllerDriverMutex");
        match CreateMutexW(None, false, &name) {
            Ok(_) => {
                let err = windows::Win32::Foundation::GetLastError();
                err != ERROR_ALREADY_EXISTS
            }
            Err(_) => false,
        }
    }
}

#[cfg(windows)]
fn show_error_dialog(message: &str) {
    use windows::Win32::UI::WindowsAndMessaging::MessageBoxW;
    use windows::core::HSTRING;
    
    let text = HSTRING::from(message);
    let title = HSTRING::from("OBOX Controller Driver");
    unsafe {
        let _ = MessageBoxW(None, &text, &title, windows::Win32::UI::WindowsAndMessaging::MB_ICONERROR);
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("");

    let has_cli_arg = args.contains(&"--cli".to_string()) ||
                      cmd == "--help" || cmd == "-h" ||
                      cmd == "--hidhide-status" ||
                      cmd == "--hidhide-disable" ||
                      cmd == "--debug-keys" ||
                      cmd == "--debug-output";

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

    #[cfg(windows)]
    {
        if !check_single_instance() {
            eprintln!("Another instance is already running.");
            std::process::exit(1);
        }
    }

    let is_cli_mode = has_cli_arg || !cfg!(windows) || !is_double_clicked();

    if is_cli_mode {
        run_cli_mode()
    } else {
        let result = run_tray_mode();
        if let Err(e) = &result {
            let msg = format!("Tray mode failed: {}", e);
            #[cfg(windows)]
            show_error_dialog(&msg);
            #[cfg(not(windows))]
            eprintln!("{}", msg);
        }
        result
    }
}

fn run_cli_mode() -> Result<()> {
    println!("OBOX Bluetooth Controller -> ViGEmBus Xbox360 (Rust v1.0)");
    println!("==========================================================");

    if let Err(e) = hidhide::ensure_enabled() {
        eprintln!("[HidHide] WARN: {}", e);
        eprintln!("[HidHide] Continuing without HidHide...");
    }
    println!();

    let client = Client::connect().context("Failed to connect to ViGEmBus driver")?;
    println!("[ViGEm] Connected to ViGEmBus driver");
    println!();

    let mut first_attempt = true;
    loop {
        match run_session(&client, None) {
            Ok(()) => {
                println!("[Main] Session ended normally, exiting.");
                return Ok(());
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("not connected") {
                    if first_attempt {
                        println!("[Main] Controller not connected. Waiting for pairing...");
                    } else {
                        eprint!("\r[Main] Waiting for controller... (retry in 3s)   ");
                        let _ = io::stdout().flush();
                    }
                } else {
                    eprintln!();
                    eprintln!("[Main] Session ended: {}", msg);
                    print!("[Main] Waiting 3s before reconnect...");
                    let _ = io::stdout().flush();
                }
                first_attempt = false;
                thread::sleep(Duration::from_secs(3));
                if !msg.contains("not connected") {
                    eprintln!();
                    println!("[Main] Attempting reconnect...");
                }
            }
        }
    }
}

fn run_tray_mode() -> Result<()> {
    let client = Client::connect().context("Failed to connect to ViGEmBus driver")?;

    if let Err(e) = hidhide::ensure_enabled() {
        eprintln!("[HidHide] WARN: {}", e);
    }

    let output_device = Arc::new(Mutex::new(None));
    let connection_status = Arc::new(Mutex::new(tray::ConnectionStatus::Disconnected));
    let mac_address = Arc::new(Mutex::new(String::new()));
    let should_exit = Arc::new(AtomicBool::new(false));

    let output_clone = output_device.clone();
    let status_clone = connection_status.clone();
    let mac_clone = mac_address.clone();
    let should_exit_clone = should_exit.clone();
    let client_clone = client.try_clone().context("Failed to clone ViGEmBus client")?;

    let driver_thread = thread::spawn(move || {
        let mut first_attempt = true;
        while !should_exit_clone.load(Ordering::SeqCst) {
            match run_session(&client_clone, Some((output_clone.clone(), status_clone.clone(), mac_clone.clone()))) {
                Ok(()) => break,
                Err(_) => {
                    let mut s = status_clone.lock().unwrap();
                    *s = tray::ConnectionStatus::Reconnecting;
                    if first_attempt {
                        tray::send_toast("OBOX Controller", "Waiting for connection...");
                    }
                    first_attempt = false;
                    thread::sleep(Duration::from_secs(3));
                }
            }
        }
    });

    // 托盘模式运行（控制台在 tray::run_tray 内部托盘创建成功后释放）
    tray::run_tray(tray::TrayState {
        output_device,
        connection_status,
        mac_address,
    }).map_err(|_| anyhow::anyhow!("Failed to run tray icon"))?;

    should_exit.store(true, Ordering::SeqCst);
    let _ = driver_thread.join();

    Ok(())
}

type OutputDevice = Arc<Mutex<hidapi::HidDevice>>;

fn run_session(
    client: &Client,
    tray_state: Option<(
        Arc<Mutex<Option<OutputDevice>>>,
        Arc<Mutex<tray::ConnectionStatus>>,
        Arc<Mutex<String>>,
    )>,
) -> Result<()> {
    if let Some((_, status, _)) = &tray_state {
        let mut s = status.lock().unwrap();
        *s = tray::ConnectionStatus::Connected;
    }

    let hid = hidapi::HidApi::new().context("Failed to initialize hidapi")?;
    
    let mac = get_mac_address(&hid);
    if let Some((_, _, mac_ptr)) = &tray_state {
        let mut m = mac_ptr.lock().unwrap();
        *m = mac;
    }
    
    let gamepad_path = find_path(&hid, 0x0001, 0x0005).ok_or_else(|| {
        anyhow::anyhow!("Controller not connected (Col01 gamepad interface not found)")
    })?;
    let consumer_path = find_path(&hid, 0x000C, 0x0001).ok_or_else(|| {
        anyhow::anyhow!("Controller not connected (Col03 consumer interface not found)")
    })?;

    let gp_cstr = CString::new(gamepad_path.as_bytes()).context("Invalid gamepad path")?;
    let cs_cstr = CString::new(consumer_path.as_bytes()).context("Invalid consumer path")?;

    let gamepad_device = hid.open_path(&gp_cstr).context("Failed to open Col01 gamepad device")?;
    gamepad_device.set_blocking_mode(false).ok();

    let output_device_inner = hid.open_path(&gp_cstr).context("Failed to open output handle")?;
    let output_device = Arc::new(Mutex::new(output_device_inner));

    if let Some((out_ptr, _, _)) = &tray_state {
        let mut ptr = out_ptr.lock().unwrap();
        *ptr = Some(output_device.clone());
    }

    let consumer_device = hid.open_path(&cs_cstr).context("Failed to open Col03 consumer")?;
    consumer_device.set_blocking_mode(false).ok();

    let mut gamepad = Xbox360Wired::new(client, TargetId::XBOX360_WIRED);
    gamepad.plugin().context("Failed to plug in virtual Xbox360")?;
    gamepad.wait_ready().context("Virtual controller not ready")?;

    let state = Arc::new(Mutex::new(ControllerState::default()));
    let running = Arc::new(AtomicBool::new(true));

    let rumble_state = Arc::new(Mutex::new(RumbleState::default()));
    let rumble_stop = Arc::new(AtomicBool::new(false));

    let rumble_state_cb = rumble_state.clone();
    let rumble_thread = gamepad
        .request_notification()
        .context("Failed to request ViGEmBus notification")?
        .spawn_thread(move |_, notif: XNotification| {
            let mut rs = rumble_state_cb.lock().unwrap();
            rs.large = notif.large_motor;
            rs.small = notif.small_motor;
            rs.active = notif.large_motor > 0 || notif.small_motor > 0;
        });

    let rumble_state_hb = rumble_state.clone();
    let rumble_stop_hb = rumble_stop.clone();
    let output_for_hb = output_device.clone();
    let heartbeat_handle = thread::Builder::new()
        .name("rumble-heartbeat".into())
        .spawn(move || {
            rumble_heartbeat_loop(&output_for_hb, &rumble_state_hb, &rumble_stop_hb);
        })
        .context("Failed to spawn rumble heartbeat thread")?;

    let state_for_consumer = state.clone();
    let running_for_consumer = running.clone();
    let consumer_handle = thread::Builder::new()
        .name("col03-consumer".into())
        .spawn(move || {
            consumer_thread(consumer_device, state_for_consumer, running_for_consumer);
        })
        .context("Failed to spawn Col03 consumer thread")?;

    tray::send_toast("OBOX Controller", "Connected successfully!");

    let mut buf = [0u8; 64];
    let disconnect_reason: String;
    loop {
        if !running.load(Ordering::SeqCst) {
            disconnect_reason = "Col03 (consumer) read error".into();
            break;
        }

        match gamepad_device.read_timeout(&mut buf, 5) {
            Ok(n) if n >= 16 && buf[0] == REPORT_ID_GAMEPAD => {
                update_gamepad_state(&state, &buf);
            }
            Ok(_) => {}
            Err(e) => {
                disconnect_reason = format!("Col01 read error: {}", e);
                break;
            }
        }

        let report_opt = {
            let mut s = state.lock().unwrap();
            if s.dirty {
                s.dirty = false;
                Some(XGamepad {
                    buttons: (s.gamepad_buttons | s.consumer_buttons).into(),
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

    tray::send_toast("OBOX Controller", "Disconnected");

    running.store(false, Ordering::SeqCst);
    rumble_stop.store(true, Ordering::SeqCst);

    let _ = gamepad.update(&XGamepad::default());
    let _ = gamepad.unplug();

    if let Some((out_ptr, status, _)) = &tray_state {
        let mut ptr = out_ptr.lock().unwrap();
        *ptr = None;
        let mut s = status.lock().unwrap();
        *s = tray::ConnectionStatus::Disconnected;
    }

    drop(rumble_thread);
    let _ = heartbeat_handle.join();
    let _ = consumer_handle.join();

    Err(anyhow::anyhow!("Controller disconnected: {}", disconnect_reason))
}

fn consumer_thread(
    device: hidapi::HidDevice,
    state: Arc<Mutex<ControllerState>>,
    running: Arc<AtomicBool>,
) {
    let mut prev_usages: HashSet<u16> = HashSet::new();
    let mut buf = [0u8; 64];
    while running.load(Ordering::SeqCst) {
        let result = device.read_timeout(&mut buf, 100);
        match result {
            Ok(n) if n >= 7 && buf[0] == REPORT_ID_CONSUMER => {
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
            Ok(_) => {}
            Err(e) => {
                eprintln!("[Col03] read error: {}, signaling disconnect", e);
                running.store(false, Ordering::SeqCst);
                break;
            }
        }
    }
}

fn consumer_usage_to_xbox(usage: u16) -> Option<u16> {
    match usage {
        USAGE_BACK => Some(XBUTTON_BACK),
        USAGE_MENU => Some(XBUTTON_START),
        USAGE_HOME => Some(XBUTTON_GUIDE),
        _ => None,
    }
}

fn apply_deadzone(val: u16) -> i16 {
    let delta = val as i32 - STICK_CENTER;
    if delta.abs() < STICK_DEADZONE {
        return 0;
    }
    if delta > 0 {
        ((delta - STICK_DEADZONE) * 32767 / (32767 - STICK_DEADZONE)) as i16
    } else {
        ((delta + STICK_DEADZONE) * 32767 / (32768 - STICK_DEADZONE)) as i16
    }
}

fn apply_deadzone_y(val: u16) -> i16 {
    -apply_deadzone(val)
}

fn apply_trigger(val: u16) -> u8 {
    if val < TRIGGER_THRESHOLD {
        0
    } else {
        (val >> 8).min(255) as u8
    }
}

fn update_gamepad_state(state: &Arc<Mutex<ControllerState>>, buf: &[u8]) {
    let mut s = state.lock().unwrap();

    let buttons = u16::from_le_bytes([buf[1], buf[2]]);
    let hat = buf[3] & 0x0F;
    let dpad = match hat {
        0 => XBUTTON_UP,
        1 => XBUTTON_UP | XBUTTON_RIGHT,
        2 => XBUTTON_RIGHT,
        3 => XBUTTON_RIGHT | XBUTTON_DOWN,
        4 => XBUTTON_DOWN,
        5 => XBUTTON_DOWN | XBUTTON_LEFT,
        6 => XBUTTON_LEFT,
        7 => XBUTTON_LEFT | XBUTTON_UP,
        _ => 0,
    };

    let mut btns = dpad;

    if (buttons & (1 << 0)) != 0 { btns |= XBUTTON_A; }
    if (buttons & (1 << 1)) != 0 { btns |= XBUTTON_B; }
    if (buttons & (1 << 3)) != 0 { btns |= XBUTTON_X; }
    if (buttons & (1 << 4)) != 0 { btns |= XBUTTON_Y; }
    if (buttons & (1 << 6)) != 0 { btns |= XBUTTON_LB; }
    if (buttons & (1 << 7)) != 0 { btns |= XBUTTON_RB; }
    if (buttons & (1 << 13)) != 0 { btns |= XBUTTON_L3; }
    if (buttons & (1 << 14)) != 0 { btns |= XBUTTON_R3; }

    let lx = apply_deadzone(u16::from_le_bytes([buf[4], buf[5]]));
    let ly = apply_deadzone_y(u16::from_le_bytes([buf[6], buf[7]]));
    let rx = apply_deadzone(u16::from_le_bytes([buf[8], buf[9]]));
    let ry = apply_deadzone_y(u16::from_le_bytes([buf[10], buf[11]]));

    let lt = apply_trigger(u16::from_le_bytes([buf[12], buf[13]]));
    let rt = apply_trigger(u16::from_le_bytes([buf[14], buf[15]]));

    if s.gamepad_buttons != btns || s.lt != lt || s.rt != rt ||
       s.thumb_lx != lx || s.thumb_ly != ly || s.thumb_rx != rx || s.thumb_ry != ry {
        s.gamepad_buttons = btns;
        s.lt = lt;
        s.rt = rt;
        s.thumb_lx = lx;
        s.thumb_ly = ly;
        s.thumb_rx = rx;
        s.thumb_ry = ry;
        s.dirty = true;
    }
}

#[derive(Default)]
struct RumbleState {
    large: u8,
    small: u8,
    active: bool,
}

fn rumble_heartbeat_loop(
    output: &Arc<Mutex<hidapi::HidDevice>>,
    state: &Arc<Mutex<RumbleState>>,
    stop: &AtomicBool,
) {
    let mut last_was_active = false;

    while !stop.load(Ordering::SeqCst) {
        let (large, small, active) = {
            let rs = state.lock().unwrap();
            (rs.large, rs.small, rs.active)
        };

        if active {
            let mut cmd = [0u8; 13];
            cmd[0] = REPORT_ID_OUTPUT;
            cmd[1] = 0x02;
            let mut enable: u8 = 0;
            if large > 0 { enable |= 0x01; }
            if small > 0 { enable |= 0x04; }
            cmd[2] = enable;
            cmd[3] = large;
            cmd[4] = 0;
            cmd[5] = small;
            cmd[6] = 0;
            if let Ok(dev) = output.lock() {
                let _ = dev.write(&cmd);
            }
            last_was_active = true;
            thread::sleep(Duration::from_millis(30));
        } else {
            if last_was_active {
                let mut cmd = [0u8; 13];
                cmd[0] = REPORT_ID_OUTPUT;
                cmd[1] = 0x02;
                cmd[2] = 0x0A;
                if let Ok(dev) = output.lock() {
                    let _ = dev.write(&cmd);
                }
                last_was_active = false;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    let mut cmd = [0u8; 13];
    cmd[0] = REPORT_ID_OUTPUT;
    cmd[1] = 0x02;
    cmd[2] = 0x0A;
    if let Ok(dev) = output.lock() {
        let _ = dev.write(&cmd);
    }
}

fn find_path(hid: &hidapi::HidApi, usage_page: u16, usage: u16) -> Option<String> {
    for device in hid.device_list() {
        if device.vendor_id() == VENDOR_ID && device.product_id() == PRODUCT_ID {
            let up = device.usage_page();
            let us = device.usage();
            if up == usage_page && us == usage {
                let path = device.path();
                return Some(path.to_string_lossy().to_string());
            }
        }
    }
    None
}

fn get_mac_address(hid: &hidapi::HidApi) -> String {
    for device in hid.device_list() {
        if device.vendor_id() == VENDOR_ID && device.product_id() == PRODUCT_ID {
            if let Some(serial) = device.serial_number() {
                if serial.len() >= 12 {
                    let mac_chars: Vec<char> = serial.chars().collect();
                    let mut mac = String::new();
                    for i in 0..6 {
                        if i > 0 { mac.push(':'); }
                        mac.push(mac_chars[i * 2]);
                        mac.push(mac_chars[i * 2 + 1]);
                    }
                    return mac.to_uppercase();
                }
            }
        }
    }
    String::new()
}

fn print_usage() {
    println!(
        "OBOX Bluetooth Controller -> ViGEmBus Xbox360 (v1.0.0)

Usage:
  obox-controller-driver                 Run in tray mode (auto when double-clicked)
  obox-controller-driver --cli           Run in CLI mode
  obox-controller-driver --hidhide-status   Show HidHide configuration
  obox-controller-driver --hidhide-disable  Disable HidHide for OBOX
  obox-controller-driver --debug-keys       Debug: real-time key input
  obox-controller-driver --debug-output     Debug: vibration/LED test
  obox-controller-driver -h, --help         Show this help

Features:
  - Col01 Gamepad + Col03 Consumer input forwarding
  - ViGEmBus Xbox360 virtual controller
  - Rumble callback (ViGEmBus -> physical gamepad)
  - LED control (RGB, HOME, Consumer area)
  - HidHide integration (auto-config)
  - Bluetooth disconnect/reconnect handling
  - System tray mode with notifications"
    );
}

fn debug_keys() -> Result<()> {
    let hid = hidapi::HidApi::new().context("Failed to initialize hidapi")?;

    let gamepad_path = find_path(&hid, 0x0001, 0x0005).context("Gamepad interface (Col01) not found")?;
    let consumer_path = find_path(&hid, 0x000C, 0x0001).context("Consumer interface (Col03) not found")?;

    let gp_cstr = CString::new(gamepad_path.as_bytes()).unwrap();
    let cs_cstr = CString::new(consumer_path.as_bytes()).unwrap();

    let mut gamepad_device = hid.open_path(&gp_cstr).unwrap();
    gamepad_device.set_blocking_mode(false).ok();

    let mut consumer_device = hid.open_path(&cs_cstr).unwrap();
    consumer_device.set_blocking_mode(false).ok();

    let mut prev_gamepad_btns: u16 = 0;
    let mut prev_consumer_usages: HashSet<u16> = HashSet::new();

    println!("Debug mode: Press any button on controller...");
    println!("Press Ctrl+C to exit.\n");

    loop {
        let mut buf = [0u8; 64];

        let n = gamepad_device.read_timeout(&mut buf, 5).unwrap_or(0);
        if n >= 16 && buf[0] == REPORT_ID_GAMEPAD {
            let buttons = u16::from_le_bytes([buf[1], buf[2]]);
            let hat = buf[3] & 0x0F;
            let dpad_str = match hat {
                0 => "UP", 1 => "UP+RIGHT", 2 => "RIGHT", 3 => "RIGHT+DOWN",
                4 => "DOWN", 5 => "DOWN+LEFT", 6 => "LEFT", 7 => "LEFT+UP",
                _ => "?",
            };

            let mut btns: u16 = 0;
            if (buttons & (1 << 0)) != 0 { btns |= XBUTTON_A; }
            if (buttons & (1 << 1)) != 0 { btns |= XBUTTON_B; }
            if (buttons & (1 << 3)) != 0 { btns |= XBUTTON_X; }
            if (buttons & (1 << 4)) != 0 { btns |= XBUTTON_Y; }
            if (buttons & (1 << 6)) != 0 { btns |= XBUTTON_LB; }
            if (buttons & (1 << 7)) != 0 { btns |= XBUTTON_RB; }
            if (buttons & (1 << 13)) != 0 { btns |= XBUTTON_L3; }
            if (buttons & (1 << 14)) != 0 { btns |= XBUTTON_R3; }

            let lx = i32::from_le_bytes([buf[4], buf[5], 0, 0]);
            let ly = i32::from_le_bytes([buf[6], buf[7], 0, 0]);
            let rx = i32::from_le_bytes([buf[8], buf[9], 0, 0]);
            let ry = i32::from_le_bytes([buf[10], buf[11], 0, 0]);
            let lt = u16::from_le_bytes([buf[12], buf[13]]);
            let rt = u16::from_le_bytes([buf[14], buf[15]]);

            if btns != prev_gamepad_btns || hat != 8 {
                prev_gamepad_btns = btns;

                let btn_names: Vec<&str> = vec![
                    ("A", XBUTTON_A), ("B", XBUTTON_B), ("X", XBUTTON_X), ("Y", XBUTTON_Y),
                    ("LB", XBUTTON_LB), ("RB", XBUTTON_RB), ("L3", XBUTTON_L3), ("R3", XBUTTON_R3),
                ]
                .into_iter()
                .filter(|(_, bit)| (btns & bit) != 0)
                .map(|(name, _)| name)
                .collect();

                let btn_str = if btn_names.is_empty() { "none".to_string() } else { btn_names.join(" ") };

                println!(
                    "[Col01] btns=0x{:04X} ({}), hat={} ({}), LX={} LY={} RX={} RY={} LT={} RT={}",
                    btns, btn_str, hat, dpad_str, lx, ly, rx, ry, lt, rt
                );
            }
        }

        let n = consumer_device.read_timeout(&mut buf, 5).unwrap_or(0);
        if n >= 7 && buf[0] == REPORT_ID_CONSUMER {
            let curr_usages: HashSet<u16> = [
                u16::from_le_bytes([buf[1], buf[2]]),
                u16::from_le_bytes([buf[3], buf[4]]),
                u16::from_le_bytes([buf[5], buf[6]]),
            ]
            .into_iter()
            .filter(|&u| u != 0)
            .collect();

            if curr_usages != prev_consumer_usages {
                prev_consumer_usages = curr_usages.clone();

                let usage_names: Vec<String> = curr_usages.into_iter().map(|u| match u {
                    USAGE_BACK => "0x0224(BACK)".to_string(),
                    USAGE_MENU => "0x0040(MENU/START)".to_string(),
                    USAGE_HOME => "0x0223(HOME/GUIDE)".to_string(),
                    _ => format!("0x{:04X}(unknown)", u),
                }).collect();

                let usage_str = if usage_names.is_empty() { "none".to_string() } else { usage_names.join(", ") };

                println!("[Col03] usages: {}", usage_str);
            }
        }
    }
}

fn debug_output() -> Result<()> {
    let hid = hidapi::HidApi::new().context("Failed to initialize hidapi")?;

    let gamepad_path = find_path(&hid, 0x0001, 0x0005).context("Gamepad interface (Col01) not found")?;

    let gp_cstr = CString::new(gamepad_path.as_bytes()).unwrap();
    let mut device = hid.open_path(&gp_cstr).unwrap();

    println!("Debug mode: Output command tester");
    println!("Press Ctrl+C to exit.\n");

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
        println!("  r  Red LED on");
        println!("  g  Green LED on");
        println!("  b  Blue LED on");
        println!("  w  White (R+G+B)");
        println!("  h  HOME LED on");
        println!("  c  Consumer area LED on");
        println!("  o  All LED off");
        print!("Select command: ");
        let _ = io::stdout().flush();

        let mut input = String::new();
        if let Err(_) = io::stdin().read_line(&mut input) {
            break;
        }

        let cmd_char = input.trim().chars().next().unwrap_or('\0');

        let cmd = match cmd_char {
            '1' => {
                let mut c = [0u8; 13];
                c[0] = REPORT_ID_OUTPUT; c[1] = 0x02; c[2] = 0x01; c[3] = 0xFF; c[4] = 0x00;
                Some(c)
            }
            '2' => {
                let mut c = [0u8; 13];
                c[0] = REPORT_ID_OUTPUT; c[1] = 0x02; c[2] = 0x04; c[5] = 0xFF; c[6] = 0x00;
                Some(c)
            }
            '3' => {
                let mut c = [0u8; 13];
                c[0] = REPORT_ID_OUTPUT; c[1] = 0x02; c[2] = 0x05; c[3] = 0xFF; c[4] = 0x00; c[5] = 0xFF; c[6] = 0x00;
                Some(c)
            }
            '4' => {
                let mut c = [0u8; 13];
                c[0] = REPORT_ID_OUTPUT; c[1] = 0x02; c[2] = 0x0A;
                Some(c)
            }
            '5' => {
                let mut c = [0u8; 13];
                c[0] = REPORT_ID_OUTPUT; c[1] = 0x02; c[2] = 0x01; c[3] = 0xFF; c[4] = 0x04;
                Some(c)
            }
            '6' => {
                let mut c = [0u8; 13];
                c[0] = REPORT_ID_OUTPUT; c[1] = 0x02; c[2] = 0x04; c[5] = 0xFF; c[6] = 0x04;
                Some(c)
            }
            '7' => {
                let mut c = [0u8; 13];
                c[0] = REPORT_ID_OUTPUT; c[1] = 0x02; c[2] = 0x05; c[3] = 0xFF; c[4] = 0x04; c[5] = 0xFF; c[6] = 0x04;
                Some(c)
            }
            'r' => {
                let mut c = [0u8; 13];
                c[0] = REPORT_ID_OUTPUT; c[1] = 0x01; c[2] = 0x01; c[3] = 0xFF;
                Some(c)
            }
            'g' => {
                let mut c = [0u8; 13];
                c[0] = REPORT_ID_OUTPUT; c[1] = 0x01; c[4] = 0x01; c[5] = 0xFF;
                Some(c)
            }
            'b' => {
                let mut c = [0u8; 13];
                c[0] = REPORT_ID_OUTPUT; c[1] = 0x01; c[6] = 0x01; c[7] = 0xFF;
                Some(c)
            }
            'w' => {
                let mut c = [0u8; 13];
                c[0] = REPORT_ID_OUTPUT; c[1] = 0x01;
                c[2] = 0x01; c[3] = 0xFF; c[4] = 0x01; c[5] = 0xFF; c[6] = 0x01; c[7] = 0xFF;
                Some(c)
            }
            'h' => {
                let mut c = [0u8; 13];
                c[0] = REPORT_ID_OUTPUT; c[1] = 0x01; c[8] = 0x01; c[9] = 0xFF;
                Some(c)
            }
            'c' => {
                let mut c = [0u8; 13];
                c[0] = REPORT_ID_OUTPUT; c[1] = 0x01; c[10] = 0x01; c[11] = 0xFF;
                Some(c)
            }
            'o' => {
                let mut c = [0u8; 13];
                c[0] = REPORT_ID_OUTPUT; c[1] = 0x01;
                c[2] = 0x02; c[3] = 0x00; c[4] = 0x02; c[5] = 0x00; c[6] = 0x02; c[7] = 0x00;
                c[8] = 0x02; c[9] = 0x00; c[10] = 0x02; c[11] = 0x00;
                Some(c)
            }
            _ => {
                println!("Unknown command");
                None
            }
        };

        if let Some(c) = cmd {
            print!("Sending: ");
            for (i, &b) in c.iter().enumerate() {
                if i > 0 { print!(" "); }
                print!("{:02X}", b);
            }
            println!();

            let _ = device.write(&c);
        }

        println!();
    }

    Ok(())
}
