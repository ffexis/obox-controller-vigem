use std::sync::{Arc, Mutex};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu},
    TrayIconBuilder,
};
use winit::{
    event::{Event, WindowEvent},
    event_loop::EventLoop,
    window::WindowBuilder,
};

use crate::REPORT_ID_OUTPUT;

#[derive(Clone, Debug, PartialEq)]
pub enum ConnectionStatus {
    Disconnected,
    Connected,
    Reconnecting,
}

pub type OutputDevice = Arc<Mutex<hidapi::HidDevice>>;

pub struct TrayState {
    pub output_device: Arc<Mutex<Option<OutputDevice>>>,
    pub connection_status: Arc<Mutex<ConnectionStatus>>,
    pub mac_address: Arc<Mutex<String>>,
}

#[cfg(windows)]
pub fn send_toast(title: &str, message: &str) {
    use windows::Win32::UI::Shell::{
        Shell_NotifyIconW, NOTIFYICONDATAW, NIF_INFO, NIF_ICON, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, LoadIconW, IDI_APPLICATION, WINDOW_EX_STYLE,
        WINDOW_STYLE,
    };
    use windows::Win32::Foundation::{HWND, HINSTANCE};
    use windows::Win32::UI::WindowsAndMessaging::{HMENU};
    use windows::core::w;
    use std::mem;

    // 后台线程中创建窗口+图标+气球，避免阻塞调用线程
    let title = title.to_string();
    let message = message.to_string();
    std::thread::spawn(move || unsafe {
        // 创建临时隐藏窗口（Shell_NotifyIconW 需要一个有效的 HWND）
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("Static"),
            w!(""),
            WINDOW_STYLE::default(),
            0, 0, 0, 0,
            HWND::default(),
            HMENU::default(),
            HINSTANCE::default(),
            None,
        );
        if hwnd.0 == 0 {
            return;
        }

        // 加载系统图标
        let hicon = LoadIconW(None, IDI_APPLICATION).unwrap_or_default();

        // 添加托盘图标（NIM_ADD 需要 NIF_ICON）
        let mut nid = NOTIFYICONDATAW::default();
        nid.cbSize = mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = 99;
        nid.uFlags = NIF_ICON;
        nid.hIcon = hicon;
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);

        // 切换为气球通知模式
        nid.uFlags = NIF_INFO;

        let title_w: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
        let message_w: Vec<u16> = message.encode_utf16().chain(std::iter::once(0)).collect();
        for (i, c) in title_w.iter().enumerate().take(63) {
            nid.szInfoTitle[i] = *c;
        }
        for (i, c) in message_w.iter().enumerate().take(255) {
            nid.szInfo[i] = *c;
        }
        // 气球超时时间（毫秒）
        nid.Anonymous.uTimeout = 5000;

        let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);

        // 等待气球显示
        std::thread::sleep(std::time::Duration::from_secs(1));

        // 清理：删除图标 + 销毁窗口
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
        let _ = DestroyWindow(hwnd);
    });
}

#[cfg(not(windows))]
pub fn send_toast(title: &str, message: &str) {
    eprintln!("[Notification] {}: {}", title, message);
}

enum LedType {
    Red,
    Green,
    Blue,
    Consumer,
    Home,
}

fn send_led_command(output: &Arc<Mutex<hidapi::HidDevice>>, led_type: LedType, on: bool) {
    let mut cmd = [0u8; 13];
    cmd[0] = REPORT_ID_OUTPUT;
    cmd[1] = 0x01;

    let (offset, brightness) = match led_type {
        LedType::Red => (2, if on { 0xFF } else { 0x00 }),
        LedType::Green => (4, if on { 0xFF } else { 0x00 }),
        LedType::Blue => (6, if on { 0xFF } else { 0x00 }),
        LedType::Consumer => (8, if on { 0xFF } else { 0x00 }),
        LedType::Home => (10, if on { 0xFF } else { 0x00 }),
    };

    cmd[offset] = if on { 0x01 } else { 0x02 };
    cmd[offset + 1] = brightness;

    if let Ok(dev) = output.lock() {
        let _ = dev.write(&cmd);
    }
}

fn handle_menu_event(
    event: &MenuEvent,
    output: &Arc<Mutex<Option<OutputDevice>>>,
    event_loop_proxy: &winit::event_loop::EventLoopProxy<()>,
) {
    if event.id == MenuId::new("exit") {
        let _ = event_loop_proxy.send_event(());
        return;
    }

    if let Ok(dev) = output.lock() {
        if let Some(d) = &*dev {
            match event.id.as_ref() {
                "red_on" => send_led_command(d, LedType::Red, true),
                "red_off" => send_led_command(d, LedType::Red, false),
                "green_on" => send_led_command(d, LedType::Green, true),
                "green_off" => send_led_command(d, LedType::Green, false),
                "blue_on" => send_led_command(d, LedType::Blue, true),
                "blue_off" => send_led_command(d, LedType::Blue, false),
                "consumer_on" => send_led_command(d, LedType::Consumer, true),
                "consumer_off" => send_led_command(d, LedType::Consumer, false),
                "home_on" => send_led_command(d, LedType::Home, true),
                "home_off" => send_led_command(d, LedType::Home, false),
                _ => {}
            }
        }
    }
}

pub fn run_tray(state: TrayState) -> Result<(), ()> {
    let event_loop = EventLoop::new().map_err(|_| ())?;
    let _window = WindowBuilder::new()
        .with_visible(false)
        .build(&event_loop)
        .map_err(|_| ())?;

    let icon = load_icon();

    let menu = Menu::new();
    
    let status_item = MenuItem::new("Status: Disconnected", true, None);
    let mac_item = MenuItem::new("MAC: N/A", true, None);
    
    let red_on = MenuItem::with_id(MenuId::new("red_on"), "Red ON", true, None);
    let red_off = MenuItem::with_id(MenuId::new("red_off"), "Red OFF", true, None);
    let green_on = MenuItem::with_id(MenuId::new("green_on"), "Green ON", true, None);
    let green_off = MenuItem::with_id(MenuId::new("green_off"), "Green OFF", true, None);
    let blue_on = MenuItem::with_id(MenuId::new("blue_on"), "Blue ON", true, None);
    let blue_off = MenuItem::with_id(MenuId::new("blue_off"), "Blue OFF", true, None);
    
    let rgb_submenu = Submenu::new("RGB Status LED", true);
    rgb_submenu.append(&red_on).ok();
    rgb_submenu.append(&red_off).ok();
    rgb_submenu.append(&green_on).ok();
    rgb_submenu.append(&green_off).ok();
    rgb_submenu.append(&blue_on).ok();
    rgb_submenu.append(&blue_off).ok();
    
    let consumer_on = MenuItem::with_id(MenuId::new("consumer_on"), "Consumer Area ON", true, None);
    let consumer_off = MenuItem::with_id(MenuId::new("consumer_off"), "Consumer Area OFF", true, None);
    let home_on = MenuItem::with_id(MenuId::new("home_on"), "HOME Button ON", true, None);
    let home_off = MenuItem::with_id(MenuId::new("home_off"), "HOME Button OFF", true, None);
    
    let led_submenu = Submenu::new("LED Control", true);
    led_submenu.append(&rgb_submenu).ok();
    led_submenu.append(&consumer_on).ok();
    led_submenu.append(&consumer_off).ok();
    led_submenu.append(&home_on).ok();
    led_submenu.append(&home_off).ok();
    
    let separator1 = PredefinedMenuItem::separator();
    let separator2 = PredefinedMenuItem::separator();
    let exit_item = MenuItem::with_id(MenuId::new("exit"), "Exit", true, None);
    
    menu.append(&status_item).ok();
    menu.append(&mac_item).ok();
    menu.append(&separator1).ok();
    menu.append(&led_submenu).ok();
    menu.append(&separator2).ok();
    menu.append(&exit_item).ok();

    let _tray_icon = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_icon(icon)
        .build()
        .map_err(|_| ())?;

    // 托盘创建成功后释放控制台（仅双击启动时）
    #[cfg(windows)]
    {
        use windows::Win32::System::Console::FreeConsole;
        unsafe { let _ = FreeConsole(); }
    }

    let event_loop_proxy = event_loop.create_proxy();
    let output_clone = state.output_device.clone();
    
    std::thread::spawn(move || {
        let receiver = MenuEvent::receiver();
        loop {
            if let Ok(event) = receiver.recv() {
                handle_menu_event(&event, &output_clone, &event_loop_proxy);
            }
        }
    });

    let status_clone = state.connection_status.clone();
    let mac_clone = state.mac_address.clone();

    let _ = event_loop.run(move |event, _control_flow| {
        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                std::process::exit(0);
            }
            Event::UserEvent(()) => {
                std::process::exit(0);
            }
            Event::AboutToWait => {
                let status = status_clone.lock().unwrap().clone();
                let mac = mac_clone.lock().unwrap().clone();
                
                let status_text = match status {
                    ConnectionStatus::Disconnected => "Status: Disconnected",
                    ConnectionStatus::Connected => "Status: Connected",
                    ConnectionStatus::Reconnecting => "Status: Reconnecting...",
                };
                
                let mac_text = format!("MAC: {}", if mac.is_empty() { "N/A" } else { &mac });
                
                status_item.set_text(status_text);
                mac_item.set_text(mac_text);
            }
            _ => {}
        }
    });

    Ok(())
}

fn load_icon() -> tray_icon::Icon {
    let icon_data = include_bytes!("boxicons-joystick-filled.ico");
    
    let decoder = ico::IconDir::read(std::io::Cursor::new(icon_data)).ok();
    if let Some(dir) = decoder {
        for entry in dir.entries() {
            let image = entry.decode().ok();
            if let Some(img) = image {
                let rgba = img.rgba_data();
                let width = img.width();
                let height = img.height();
                return tray_icon::Icon::from_rgba(rgba.to_vec(), width as u32, height as u32).unwrap_or_else(|_| {
                    create_fallback_icon()
                });
            }
        }
    }
    
    create_fallback_icon()
}

fn create_fallback_icon() -> tray_icon::Icon {
    let mut data = vec![0u8; 32 * 32 * 4];
    for i in 0..32 * 32 {
        data[i * 4] = 0x4A;
        data[i * 4 + 1] = 0x96;
        data[i * 4 + 2] = 0x2A;
        data[i * 4 + 3] = 0xFF;
    }
    tray_icon::Icon::from_rgba(data, 32, 32).unwrap()
}