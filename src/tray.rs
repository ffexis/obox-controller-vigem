use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tray_icon::{
    menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, Submenu},
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
    Connecting,
}

pub type OutputDevice = Arc<Mutex<hidapi::HidDevice>>;

pub struct TrayState {
    pub output_device: Arc<Mutex<Option<OutputDevice>>>,
    pub connection_status: Arc<Mutex<ConnectionStatus>>,
    pub mac_address: Arc<Mutex<String>>,
    pub deadzone_enabled: Arc<AtomicBool>,
}

/// Host window for the tray menu, plus whether it is currently shown.
///
/// The menu is not popped up by `tray-icon`: its owner window is hidden and created with
/// `WS_EX_NOACTIVATE|WS_EX_TRANSPARENT`, so clicking the desktop or taskbar never closes
/// it, `TrackPopupMenu`'s modal loop never returns, and the whole taskbar only flashes.
///
/// Instead, the menu thread's own host window is the owner (visible, activatable), and a
/// `WM_NULL` is posted to give the foreground lock back. The menu closes on a click outside,
/// on the watcher posting `WM_CANCELMODE` after a foreground change, or on item selection.
#[cfg(windows)]
static MENU_OPENED: AtomicBool = AtomicBool::new(false);
#[cfg(windows)]
static MENU_HOST_HWND: AtomicIsize = AtomicIsize::new(0);
/// Whether the menu loop was actually entered (`WM_ENTERMENULOOP` / `WM_EXITMENULOOP`).
#[cfg(windows)]
static MENU_LOOP_ENTERED: AtomicBool = AtomicBool::new(false);
/// Whether this popup actually obtained foreground-window rights.
#[cfg(windows)]
static MENU_FOREGROUND_OK: AtomicBool = AtomicBool::new(false);

/// Custom message asking the host window to show the menu.
#[cfg(windows)]
const WM_OBOX_SHOW_MENU: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 1;
/// Requests a refresh of the dynamic items while the menu is open (posted by the watcher, handled by the menu thread).
#[cfg(windows)]
const WM_OBOX_REFRESH_MENU: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 2;

#[cfg(windows)]
fn tray_menu_open() -> bool {
    MENU_OPENED.load(Ordering::SeqCst)
}

#[cfg(windows)]
unsafe extern "system" fn menu_host_proc(
    hwnd: windows::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::ClientToScreen;
    use windows::Win32::UI::WindowsAndMessaging::{
        DefWindowProcW, EndMenu, WM_ENTERMENULOOP, WM_EXITMENULOOP, WM_LBUTTONDOWN,
        WM_MBUTTONDOWN, WM_NCLBUTTONDOWN, WM_NCMBUTTONDOWN, WM_NCRBUTTONDOWN, WM_RBUTTONDOWN,
        WM_XBUTTONDOWN,
    };

    unsafe {
        // Menu-host window procedure. Runs on the menu thread, inside the
        // TrackPopupMenu modal loop while the menu is visible.
        if msg == WM_OBOX_REFRESH_MENU {
            if sync_menu_items() {
                repaint_menus();
            }
            return windows::Win32::Foundation::LRESULT(0);
        }

        // A click outside the popup is delivered here because the mouse is
        // captured by the host window while the menu is open.
        if MENU_LOOP_ENTERED.load(Ordering::SeqCst)
            && matches!(
                msg,
                WM_LBUTTONDOWN
                    | WM_RBUTTONDOWN
                    | WM_MBUTTONDOWN
                    | WM_XBUTTONDOWN
                    | WM_NCLBUTTONDOWN
                    | WM_NCRBUTTONDOWN
                    | WM_NCMBUTTONDOWN
            )
        {
            let x = (lparam.0 & 0xFFFF) as u16 as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
            let mut pt = POINT { x, y };
            if !matches!(msg, WM_NCLBUTTONDOWN | WM_NCRBUTTONDOWN | WM_NCMBUTTONDOWN) {
                let _ = ClientToScreen(hwnd, &mut pt);
            }
            if !point_in_any_menu(pt) {
                let _ = EndMenu();
                return windows::Win32::Foundation::LRESULT(0);
            }
        }

        match msg {
            WM_ENTERMENULOOP => {
                MENU_LOOP_ENTERED.store(true, Ordering::SeqCst);
                return windows::Win32::Foundation::LRESULT(0);
            }
            WM_EXITMENULOOP => {
                MENU_LOOP_ENTERED.store(false, Ordering::SeqCst);
            }
            _ => {}
        }

        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

/// Shared state the menu thread needs (all `Arc`, safe to share across threads).
#[cfg(windows)]
struct MenuContext {
    status: Arc<Mutex<ConnectionStatus>>,
    mac: Arc<Mutex<String>>,
    deadzone: Arc<AtomicBool>,
    printscreen: Arc<AtomicBool>,
}

/// Menu handles held locally by the menu thread, plus the last synced values.
///
/// `muda` menu items are `Rc`-wrapped and not `Send`, so the whole menu can only be
/// used by the thread that created it (the menu thread).
#[cfg(windows)]
struct MenuItems {
    status_item: MenuItem,
    mac_item: MenuItem,
    deadzone_item: MenuItem,
    printscreen_item: CheckMenuItem,
    led_submenu: Submenu,
    last_status: Option<ConnectionStatus>,
    last_mac: String,
    last_deadzone: bool,
    last_printscreen: bool,
    last_led_enabled: Option<bool>,
}

#[cfg(windows)]
impl MenuItems {
    /// Write the latest state into the menu before popping up. Not called while the menu is open (never touches items being tracked).
    fn sync(&mut self, ctx: &MenuContext) {
        let status = match ctx.status.try_lock() {
            Ok(s) => s.clone(),
            Err(_) => self.last_status.clone().unwrap_or(ConnectionStatus::Disconnected),
        };
        let mac = match ctx.mac.try_lock() {
            Ok(m) => m.clone(),
            Err(_) => self.last_mac.clone(),
        };
        let deadzone = ctx.deadzone.load(Ordering::SeqCst);
        let printscreen = ctx.printscreen.load(Ordering::SeqCst);

        if self.last_status.as_ref() != Some(&status) {
            self.status_item.set_text(match status {
                ConnectionStatus::Disconnected => "Status: Disconnected",
                ConnectionStatus::Connected => "Status: Connected",
                ConnectionStatus::Connecting => "Status: Waiting for connection",
            });
            let connected = status == ConnectionStatus::Connected;
            if self.last_led_enabled != Some(connected) {
                self.led_submenu.set_enabled(connected);
                self.last_led_enabled = Some(connected);
            }
            self.last_status = Some(status);
        }
        if self.last_mac != mac {
            self.mac_item.set_text(&format!(
                "MAC: {}",
                if mac.is_empty() { "N/A" } else { &mac }
            ));
            self.last_mac = mac;
        }
        if self.last_deadzone != deadzone {
            self.deadzone_item
                .set_text(if deadzone { "Deadzone: ON" } else { "Deadzone: OFF" });
            self.last_deadzone = deadzone;
        }
        if self.last_printscreen != printscreen {
            self.printscreen_item.set_checked(printscreen);
            self.last_printscreen = printscreen;
        }
    }
}

/// Menu-thread-local runtime state: menu handles + shared state.
/// Only accessed on the menu thread (`muda` menu items are not `Send`).
#[cfg(windows)]
struct MenuRuntime {
    items: MenuItems,
    ctx: MenuContext,
}

#[cfg(windows)]
thread_local! {
    static MENU_RT: std::cell::RefCell<Option<MenuRuntime>> =
        const { std::cell::RefCell::new(None) };
}

/// Menu thread: owns the menu, the host window, and its own message loop.
///
/// The menu must never be built on winit's thread: winit registers `RIDEV_DEVNOTIFY`
/// on its message window, so controller plug/unplug makes `WM_INPUT_DEVICE_CHANGE` flood
/// that thread with wake-up messages. The modal loop drains them one by one, the queue
/// never empties, hardware mouse input and `WM_TIMER` starve — the menu stays visible but
/// hover and click do nothing; only another thread's `PostMessage(WM_CANCELMODE)` closes it.
#[cfg(windows)]
fn start_menu_thread(ctx: MenuContext) {
    use tray_icon::menu::{Menu, MenuId, MenuItem, PredefinedMenuItem, Submenu};

    std::thread::spawn(move || {
        let menu = Menu::new();

        let status_item = MenuItem::new("Status: Disconnected", true, None);
        let mac_item = MenuItem::new("MAC: N/A", true, None);
        let deadzone_item = MenuItem::with_id(
            MenuId::new("toggle_deadzone"),
            if ctx.deadzone.load(Ordering::SeqCst) {
                "Deadzone: ON"
            } else {
                "Deadzone: OFF"
            },
            true,
            None,
        );
        let printscreen_item = CheckMenuItem::with_id(
            MenuId::new("toggle_printscreen"),
            "Block PrintScreen Key",
            true,
            ctx.printscreen.load(Ordering::SeqCst),
            None,
        );

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

        let consumer_on =
            MenuItem::with_id(MenuId::new("consumer_on"), "Consumer Area ON", true, None);
        let consumer_off =
            MenuItem::with_id(MenuId::new("consumer_off"), "Consumer Area OFF", true, None);
        let home_on = MenuItem::with_id(MenuId::new("home_on"), "HOME Button ON", true, None);
        let home_off = MenuItem::with_id(MenuId::new("home_off"), "HOME Button OFF", true, None);

        let led_submenu = Submenu::new("LED Control", false);
        led_submenu.append(&rgb_submenu).ok();
        led_submenu.append(&consumer_on).ok();
        led_submenu.append(&consumer_off).ok();
        led_submenu.append(&home_on).ok();
        led_submenu.append(&home_off).ok();

        let separator1 = PredefinedMenuItem::separator();
        let separator2 = PredefinedMenuItem::separator();
        let separator3 = PredefinedMenuItem::separator();
        let exit_item = MenuItem::with_id(MenuId::new("exit"), "Exit", true, None);

        menu.append(&status_item).ok();
        menu.append(&mac_item).ok();
        menu.append(&separator1).ok();
        menu.append(&deadzone_item).ok();
        menu.append(&printscreen_item).ok();
        menu.append(&separator2).ok();
        menu.append(&led_submenu).ok();
        menu.append(&separator3).ok();
        menu.append(&exit_item).ok();

        let items = MenuItems {
            last_status: None,
            last_mac: String::new(),
            last_deadzone: ctx.deadzone.load(Ordering::SeqCst),
            last_printscreen: ctx.printscreen.load(Ordering::SeqCst),
            last_led_enabled: None,
            status_item,
            mac_item,
            deadzone_item,
            printscreen_item,
            led_submenu,
        };

        // The runtime state lives in the menu thread's thread-local, so the host window's
        // WndProc (same thread) can modify these menu items when it gets a refresh message.
        MENU_RT.with(|cell| {
            *cell.borrow_mut() = Some(MenuRuntime { items, ctx });
        });

        let hwnd = create_menu_host(&menu);
        if hwnd.0 == 0 {
            return;
        }
        MENU_HOST_HWND.store(hwnd.0, Ordering::SeqCst);

        unsafe {
            use windows::Win32::UI::WindowsAndMessaging::{
                DispatchMessageW, GetMessageW, TranslateMessage, MSG,
            };

            let mut msg = MSG::default();
            while GetMessageW(&mut msg, windows::Win32::Foundation::HWND::default(), 0, 0).as_bool() {
                if msg.message == WM_OBOX_SHOW_MENU {
                    sync_menu_items();
                    show_menu(hwnd, &menu);
                } else {
                    let _ = TranslateMessage(&msg);
                    let _ = DispatchMessageW(&msg);
                }
            }
        }
    });
}

/// Sync shared state into the menu items, returning whether anything changed. Menu thread only.
#[cfg(windows)]
fn sync_menu_items() -> bool {
    MENU_RT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(MenuRuntime { items, ctx }) = borrow.as_mut() else {
            return false;
        };
        let before = (
            items.last_status.clone(),
            items.last_mac.clone(),
            items.last_deadzone,
            items.last_printscreen,
        );
        items.sync(ctx);
        before
            != (
                items.last_status.clone(),
                items.last_mac.clone(),
                items.last_deadzone,
                items.last_printscreen,
            )
    })
}

/// Force a repaint of the menu windows after editing open items; otherwise the new text only shows on the next mouse move.
#[cfg(windows)]
fn repaint_menus() {
    use windows::Win32::Graphics::Gdi::{
        RedrawWindow, HRGN, RDW_ALLCHILDREN, RDW_INVALIDATE, RDW_UPDATENOW,
    };

    for h in menu_windows() {
        unsafe {
            let _ = RedrawWindow(
                h,
                None,
                HRGN::default(),
                RDW_INVALIDATE | RDW_ALLCHILDREN | RDW_UPDATENOW,
            );
        }
    }
}

/// Pop up the menu on the menu thread. Called on the menu thread only.
#[cfg(windows)]
fn show_menu(hwnd: windows::Win32::Foundation::HWND, menu: &Menu) {
    use tray_icon::menu::ContextMenu;
    use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetCursorPos, GetForegroundWindow, GetWindowThreadProcessId, PostMessageW,
        SetForegroundWindow, SetWindowPos, ShowWindow, TrackPopupMenu, HMENU, SWP_NOACTIVATE,
        SWP_SHOWWINDOW, SW_HIDE, TPM_BOTTOMALIGN, TPM_LEFTALIGN, WM_NULL,
    };

    unsafe {
        // Ignore duplicate requests while it is already showing
        if MENU_OPENED.swap(true, Ordering::SeqCst) {
            return;
        }

        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);

        // The owner must be visible, otherwise the focus-loss check fails and clicking outside does not auto-close
        let _ = SetWindowPos(hwnd, HWND(0), pt.x, pt.y, 1, 1, SWP_NOACTIVATE | SWP_SHOWWINDOW);

        // Ask for foreground directly first; only if the foreground lock blocks it, briefly attach to the current foreground thread and ask again
        let mut fg_ok = SetForegroundWindow(hwnd).as_bool();
        if !fg_ok {
            let fg = GetForegroundWindow();
            let self_tid = GetCurrentThreadId();
            let fg_tid = if fg.0 != 0 {
                GetWindowThreadProcessId(fg, None)
            } else {
                0
            };
            if fg_tid != 0 && fg_tid != self_tid && AttachThreadInput(fg_tid, self_tid, true).as_bool()
            {
                fg_ok = SetForegroundWindow(hwnd).as_bool();
                let _ = AttachThreadInput(fg_tid, self_tid, false);
            }
        }
        MENU_FOREGROUND_OK.store(fg_ok && GetForegroundWindow() == hwnd, Ordering::SeqCst);
        MENU_LOOP_ENTERED.store(true, Ordering::SeqCst);

        let _ = TrackPopupMenu(
            HMENU(menu.hpopupmenu() as isize),
            TPM_BOTTOMALIGN | TPM_LEFTALIGN,
            pt.x,
            pt.y,
            0,
            hwnd,
            None,
        );

        MENU_LOOP_ENTERED.store(false, Ordering::SeqCst);
        MENU_FOREGROUND_OK.store(false, Ordering::SeqCst);
        let _ = ShowWindow(hwnd, SW_HIDE);
        // Release the foreground lock back to the shell, otherwise the next
        // click on another window would be swallowed.
        let _ = PostMessageW(hwnd, WM_NULL, WPARAM(0), LPARAM(0));
        MENU_OPENED.store(false, Ordering::SeqCst);
    }
}

/// Visible popup-menu windows of this process (`muda`'s menu window class is `#32768`).
#[cfg(windows)]
fn menu_windows() -> Vec<windows::Win32::Foundation::HWND> {
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowExW, GetWindowThreadProcessId, IsWindowVisible,
    };

    let mut out = Vec::new();
    unsafe {
        let self_pid = GetCurrentProcessId();
        let mut after = HWND(0);
        loop {
            let hwnd = FindWindowExW(HWND(0), after, w!("#32768"), PCWSTR::null());
            if hwnd.0 == 0 {
                break;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == self_pid && IsWindowVisible(hwnd).as_bool() {
                out.push(hwnd);
            }
            after = hwnd;
        }
    }
    out
}

/// Whether the click landed inside a menu.
#[cfg(windows)]
fn point_in_any_menu(pt: windows::Win32::Foundation::POINT) -> bool {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

    menu_windows().into_iter().any(|h| unsafe {
        let mut r = RECT::default();
        GetWindowRect(h, &mut r).is_ok()
            && pt.x >= r.left
            && pt.x < r.right
            && pt.y >= r.top
            && pt.y < r.bottom
    })
}

/// Create the menu host window on the **calling thread** and route the menu's `WM_COMMAND` to `MenuEvent`.
/// Called by the menu thread; a return value of 0 means creation failed.
#[cfg(windows)]
fn create_menu_host(menu: &Menu) -> windows::Win32::Foundation::HWND {
    use tray_icon::menu::ContextMenu;
    use windows::core::w;
    use windows::Win32::Foundation::{HINSTANCE, HWND};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, RegisterClassW, HMENU, WNDCLASSW, WS_EX_TOOLWINDOW, WS_POPUP,
    };

    unsafe {
        let hinstance = HINSTANCE(GetModuleHandleW(None).map(|h| h.0).unwrap_or(0));
        let wc = WNDCLASSW {
            lpfnWndProc: Some(menu_host_proc),
            hInstance: hinstance,
            lpszClassName: w!("OBOXMenuHost"),
            ..Default::default()
        };
        // It is fine if the class is already registered; CreateWindowExW still finds it
        let _ = RegisterClassW(&wc);

        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            w!("OBOXMenuHost"),
            w!("OBOX Menu Host"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            HWND::default(),
            HMENU::default(),
            hinstance,
            None,
        );
        if hwnd.0 == 0 {
            return hwnd;
        }

        menu.attach_menu_subclass_for_hwnd(hwnd.0);
        hwnd
    }
}

/// On right-clicking the tray icon, ask the host window to show the menu.
#[cfg(windows)]
fn start_tray_click_listener() {
    use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

    std::thread::spawn(|| {
        let receiver = TrayIconEvent::receiver();
        loop {
            let Ok(event) = receiver.recv() else { continue };
            if let TrayIconEvent::Click {
                button: MouseButton::Right,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let hwnd = MENU_HOST_HWND.load(Ordering::SeqCst);
                if hwnd != 0 {
                    unsafe {
                        let _ = PostMessageW(HWND(hwnd), WM_OBOX_SHOW_MENU, WPARAM(0), LPARAM(0));
                    }
                }
            }
        }
    });
}

/// Small background thread with two jobs:
///
/// 1. **State change -> live refresh while the menu is open**: post `WM_OBOX_REFRESH_MENU`,
///    handled by the menu thread inside the modal loop (same-thread item edits, safe).
/// 2. **Detect "the user clicked outside the menu"**: while open, the host window is the
///    foreground window, so a switch to another window equals a click outside. This machine
///    never sends a cancel message itself (measured: the menu stays after a click and
///    `GUI_INMENUMODE` stays set), so post `WM_CANCELMODE` to leave the modal loop.
#[cfg(windows)]
fn start_menu_watcher(ctx: MenuContext) {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, PostMessageW, WM_CANCELMODE,
    };

    std::thread::spawn(move || {
        let mut opened_at: Option<Instant> = None;
        let mut last_post: Option<Instant> = None;
        let mut last_status = ctx.status.try_lock().ok().map(|s| s.clone());
        let mut last_mac = ctx.mac.try_lock().ok().map(|m| m.clone());
        let mut last_deadzone = ctx.deadzone.load(Ordering::SeqCst);
        let mut last_printscreen = ctx.printscreen.load(Ordering::SeqCst);

        loop {
            std::thread::sleep(Duration::from_millis(100));

            // 1. State diff
            let status = ctx.status.try_lock().ok().map(|s| s.clone());
            let mac = ctx.mac.try_lock().ok().map(|m| m.clone());
            let deadzone = ctx.deadzone.load(Ordering::SeqCst);
            let printscreen = ctx.printscreen.load(Ordering::SeqCst);

            let mut changed = false;
            if let Some(value) = status {
                if last_status.as_ref() != Some(&value) {
                    last_status = Some(value);
                    changed = true;
                }
            }
            if let Some(value) = mac {
                if last_mac.as_ref() != Some(&value) {
                    last_mac = Some(value);
                    changed = true;
                }
            }
            if deadzone != last_deadzone {
                last_deadzone = deadzone;
                changed = true;
            }
            if printscreen != last_printscreen {
                last_printscreen = printscreen;
                changed = true;
            }

            let in_menu = MENU_LOOP_ENTERED.load(Ordering::SeqCst);
            let host = MENU_HOST_HWND.load(Ordering::SeqCst);
            if changed && in_menu && host != 0 {
                unsafe {
                    let _ = PostMessageW(HWND(host), WM_OBOX_REFRESH_MENU, WPARAM(0), LPARAM(0));
                }
            }

            // 2. Click outside the menu
            if !in_menu {
                opened_at = None;
                last_post = None;
                continue;
            }
            // Right after popping up the foreground has not switched yet, so allow 400ms to settle;
            // only when foreground was really obtained do we use "foreground changed" to detect outside clicks.
            let started = *opened_at.get_or_insert_with(Instant::now);
            if host == 0
                || started.elapsed() < Duration::from_millis(400)
                || !MENU_FOREGROUND_OK.load(Ordering::SeqCst)
            {
                continue;
            }
            if last_post.is_some_and(|t| t.elapsed() < Duration::from_millis(300)) {
                continue;
            }

            let fg = unsafe { GetForegroundWindow() };
            if fg.0 == host {
                continue;
            }
            last_post = Some(Instant::now());
            unsafe {
                let _ = PostMessageW(HWND(host), WM_CANCELMODE, WPARAM(0), LPARAM(0));
            }
        }
    });
}

#[cfg(windows)]
pub fn send_toast(title: &str, message: &str) {
    use windows::Win32::UI::Shell::{
        Shell_NotifyIconW, NOTIFYICONDATAW, NIF_INFO, NIF_ICON, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, DispatchMessageW, LoadIconW, PeekMessageW,
        TranslateMessage, HMENU, IDI_APPLICATION, MSG, PM_REMOVE, WINDOW_STYLE, WS_EX_NOACTIVATE,
        WS_EX_TOOLWINDOW,
    };
    use windows::Win32::Foundation::{HINSTANCE, HWND};
    use windows::core::w;
    use std::mem;

    // Skip the notification while the menu is open: the balloon would steal the menu's activation and close it.
    if tray_menu_open() {
        return;
    }

    // Create window + icon + balloon on a background thread so the caller is not blocked
    let title = title.to_string();
    let message = message.to_string();
    std::thread::spawn(move || unsafe {
        // Create a temporary hidden window (Shell_NotifyIconW needs a valid HWND).
        // NOACTIVATE/TOOLWINDOW: it is never activated and never appears in the taskbar.
        let hwnd = CreateWindowExW(
            WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
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

        // Load the system icon
        let hicon = LoadIconW(None, IDI_APPLICATION).unwrap_or_default();

        // Add the tray icon (NIM_ADD requires NIF_ICON)
        let mut nid = NOTIFYICONDATAW::default();
        nid.cbSize = mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = 99;
        nid.uFlags = NIF_ICON;
        nid.hIcon = hicon;
        let add_ok = Shell_NotifyIconW(NIM_ADD, &nid).as_bool();
        if !add_ok {
            let _ = DestroyWindow(hwnd);
            return;
        }

        // Re-check: the menu may have opened within these few microseconds, so do not show the balloon then
        if tray_menu_open() {
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
            let _ = DestroyWindow(hwnd);
            return;
        }

        // Switch to balloon-notification mode
        nid.uFlags = NIF_INFO;

        let title_w: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
        let message_w: Vec<u16> = message.encode_utf16().chain(std::iter::once(0)).collect();
        for (i, c) in title_w.iter().enumerate().take(63) {
            nid.szInfoTitle[i] = *c;
        }
        for (i, c) in message_w.iter().enumerate().take(255) {
            nid.szInfo[i] = *c;
        }
        // Balloon timeout in milliseconds
        nid.Anonymous.uTimeout = 5000;

        // Show the balloon notification
        let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);

        // Wait for the balloon to show. This wait must pump messages: Explorer (the taskbar)
        // sends synchronous messages to this icon window, and since this thread owns the
        // window, without pumping it never replies — the taskbar hangs, icons only flash.
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut msg = MSG::default();
        loop {
            while PeekMessageW(&mut msg, HWND::default(), 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            if tray_menu_open() || Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        // Cleanup: delete the icon + destroy the window
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
        let _ = DestroyWindow(hwnd);
    });
}

#[cfg(not(windows))]
pub fn send_toast(title: &str, message: &str) {
    eprintln!("[Notification] {}: {}", title, message);
}

/// Read the Snipping Tool toggle state for the PrintScreen key.
///
/// Backed by `HKCU\Control Panel\Keyboard\PrintScreenKeyForSnippingEnabled`:
/// - `1` = PrintScreen opens the Snipping Tool (not blocked)
/// - `0` = the Snipping Tool is not opened (shown as blocked in the menu)
/// - when the value is missing or reading fails, treat it as "not blocked".
#[cfg(windows)]
fn print_screen_blocked() -> bool {
    use windows::core::w;
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE,
        REG_DWORD, REG_VALUE_TYPE,
    };

    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!("Control Panel\\Keyboard"),
            0,
            KEY_QUERY_VALUE,
            &mut hkey,
        ) != ERROR_SUCCESS
        {
            return false;
        }

        let mut data: u32 = 1;
        let mut size = std::mem::size_of::<u32>() as u32;
        let mut ty = REG_VALUE_TYPE::default();
        let status = RegQueryValueExW(
            hkey,
            w!("PrintScreenKeyForSnippingEnabled"),
            None,
            Some(&mut ty),
            Some(&mut data as *mut u32 as *mut u8),
            Some(&mut size),
        );
        let _ = RegCloseKey(hkey);

        status == ERROR_SUCCESS && ty == REG_DWORD && data == 0
    }
}

/// Write the Snipping Tool toggle state for the PrintScreen key.
/// Returns `true` on success; on failure the caller should keep the previous state.
#[cfg(windows)]
fn set_print_screen_blocked(blocked: bool) -> bool {
    use windows::core::w;
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE,
        REG_DWORD,
    };

    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!("Control Panel\\Keyboard"),
            0,
            KEY_SET_VALUE,
            &mut hkey,
        ) != ERROR_SUCCESS
        {
            return false;
        }

        let bytes = (if blocked { 0u32 } else { 1u32 }).to_le_bytes();
        let status = RegSetValueExW(
            hkey,
            w!("PrintScreenKeyForSnippingEnabled"),
            0,
            REG_DWORD,
            Some(&bytes),
        );
        let _ = RegCloseKey(hkey);
        status == ERROR_SUCCESS
    }
}

#[cfg(not(windows))]
fn print_screen_blocked() -> bool {
    false
}

#[cfg(not(windows))]
fn set_print_screen_blocked(_blocked: bool) -> bool {
    false
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
        LedType::Consumer => (10, if on { 0xFF } else { 0x00 }),
        LedType::Home => (8, if on { 0xFF } else { 0x00 }),
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
    deadzone_enabled: &Arc<AtomicBool>,
    printscreen_blocked: &Arc<AtomicBool>,
    event_loop_proxy: &winit::event_loop::EventLoopProxy<()>,
) {
    if event.id == MenuId::new("exit") {
        let _ = event_loop_proxy.send_event(());
        return;
    }

    if event.id == MenuId::new("toggle_deadzone") {
        let prev = deadzone_enabled.load(Ordering::SeqCst);
        deadzone_enabled.store(!prev, Ordering::SeqCst);
        return;
    }

    if event.id == MenuId::new("toggle_printscreen") {
        // Invert the current registry value so we stay in sync with external changes (Windows Settings page)
        let target = !print_screen_blocked();
        if set_print_screen_blocked(target) {
            printscreen_blocked.store(target, Ordering::SeqCst);
        }
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
    let printscreen_blocked = Arc::new(AtomicBool::new(print_screen_blocked()));

    // The menu is not popped up by tray-icon (it uses an invisible owner window, so the menu
    // cannot be closed); it is shown from a dedicated thread instead — see `start_menu_thread`.
    let _tray_icon = TrayIconBuilder::new()
        .with_icon(icon)
        .with_tooltip("OBOX Controller")
        .build()
        .map_err(|_| ())?;

    // Release the console once the tray is created (only when launched by double-click)
    #[cfg(windows)]
    {
        use windows::Win32::System::Console::FreeConsole;
        unsafe { let _ = FreeConsole(); }

        start_menu_thread(MenuContext {
            status: state.connection_status.clone(),
            mac: state.mac_address.clone(),
            deadzone: state.deadzone_enabled.clone(),
            printscreen: printscreen_blocked.clone(),
        });
        start_menu_watcher(MenuContext {
            status: state.connection_status.clone(),
            mac: state.mac_address.clone(),
            deadzone: state.deadzone_enabled.clone(),
            printscreen: printscreen_blocked.clone(),
        });
        start_tray_click_listener();
    }

    let event_loop_proxy = event_loop.create_proxy();
    let output_clone = state.output_device.clone();
    let deadzone_clone = state.deadzone_enabled.clone();
    let printscreen_clone = printscreen_blocked.clone();

    std::thread::spawn(move || {
        let receiver = MenuEvent::receiver();
        loop {
            if let Ok(event) = receiver.recv() {
                handle_menu_event(&event, &output_clone, &deadzone_clone, &printscreen_clone, &event_loop_proxy);
            }
        }
    });

    let status_clone = state.connection_status.clone();
    let mac_clone = state.mac_address.clone();
    let printscreen_for_ui = printscreen_blocked.clone();
    let mut last_printscreen_check = Instant::now();
    let mut last_tooltip_status: Option<ConnectionStatus> = None;
    let mut last_tooltip_mac = String::new();

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
                // try_lock: the UI thread must never block on driver-thread
                // locks — a blocked message pump freezes the tray menu.
                // On contention we simply keep the previous text.
                let status = status_clone
                    .try_lock()
                    .map(|s| s.clone())
                    .unwrap_or(ConnectionStatus::Disconnected);
                let mac = mac_clone
                    .try_lock()
                    .map(|m| m.clone())
                    .unwrap_or_default();

                // Sync the PrintScreen toggle from the registry at low frequency so external (Settings) changes reach the menu
                if last_printscreen_check.elapsed() >= Duration::from_secs(1) {
                    last_printscreen_check = Instant::now();
                    printscreen_for_ui.store(print_screen_blocked(), Ordering::SeqCst);
                }

                // Menu items are synced by the menu thread before popping up; here we only handle the tooltip.
                // The tooltip goes through Shell_NotifyIconW (synchronous IPC to explorer), so it must be diffed:
                // AboutToWait can run dozens of times a second, and unconditional updates would flood the taskbar.
                if last_tooltip_status.as_ref() != Some(&status) || last_tooltip_mac != mac {
                    let tooltip = match status {
                        ConnectionStatus::Connected if !mac.is_empty() => {
                            format!("OBOX Controller — Connected ({})", mac)
                        }
                        ConnectionStatus::Connected => "OBOX Controller — Connected".to_string(),
                        _ => "OBOX Controller — Waiting for controller...".to_string(),
                    };
                    let _ = _tray_icon.set_tooltip(Some(&tooltip));
                    last_tooltip_status = Some(status.clone());
                    last_tooltip_mac = mac;
                }
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
