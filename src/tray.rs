//! Notification-area icon and the global overlay hotkey (Ctrl+Shift+O), both
//! owned by one thread with a message-only window. Plain Win32; no hooks.

use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};

use eframe::egui;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentProcessId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, RegisterHotKey};
use windows_sys::Win32::UI::Shell::{NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DispatchMessageW, EnumWindows, GetCursorPos,
    GetMessageW, GetWindowTextW, GetWindowThreadProcessId, HWND_MESSAGE, IDI_APPLICATION, LoadIconW, MF_SEPARATOR, MF_STRING,
    MSG, RegisterClassW, RegisterWindowMessageW, SW_RESTORE, SetForegroundWindow, ShowWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON,
    TrackPopupMenu, WM_APP, WM_HOTKEY, WM_LBUTTONUP, WM_RBUTTONUP, WNDCLASSW,
};

use crate::worker::Shared;

const WM_TRAY: u32 = WM_APP + 1;
const HOTKEY_OVERLAY: i32 = 1;
const CMD_OPEN: usize = 1;
const CMD_OVERLAY: usize = 2;
const CMD_QUIT: usize = 3;

struct Ctx {
    egui: egui::Context,
    shared: Arc<Shared>,
    main_title: &'static str,
    /// Explorer broadcasts this after it restarts; the icon must be re-added.
    taskbar_created: u32,
}

static CTX: OnceLock<Ctx> = OnceLock::new();

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Starts the tray thread. Returns whether the hotkey could be registered
/// (another program may own Ctrl+Shift+O).
pub fn spawn(egui: egui::Context, shared: Arc<Shared>, main_title: &'static str) -> std::sync::mpsc::Receiver<bool> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("riftlite-tray".into())
        .spawn(move || unsafe {
            let taskbar_created = RegisterWindowMessageW(wide("TaskbarCreated").as_ptr());
            let _ = CTX.set(Ctx { egui, shared, main_title, taskbar_created });
            let hinst = GetModuleHandleW(std::ptr::null());
            let class = wide("riftlite-tray");
            let wc = WNDCLASSW {
                style: 0,
                lpfnWndProc: Some(wndproc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: hinst,
                hIcon: std::ptr::null_mut(),
                hCursor: std::ptr::null_mut(),
                hbrBackground: std::ptr::null_mut(),
                lpszMenuName: std::ptr::null(),
                lpszClassName: class.as_ptr(),
            };
            RegisterClassW(&wc);
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                class.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                std::ptr::null_mut(),
                hinst,
                std::ptr::null(),
            );
            if hwnd.is_null() {
                let _ = tx.send(false);
                return;
            }
            add_icon(hwnd);
            let hotkey = RegisterHotKey(hwnd, HOTKEY_OVERLAY, MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT, 'O' as u32) != 0;
            let _ = tx.send(hotkey);
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                DispatchMessageW(&msg);
            }
        })
        .expect("spawn tray thread");
    rx
}

unsafe fn icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = 1;
    nid
}

unsafe fn add_icon(hwnd: HWND) {
    unsafe {
        let mut nid = icon_data(hwnd);
        nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        nid.uCallbackMessage = WM_TRAY;
        nid.hIcon = LoadIconW(std::ptr::null_mut(), IDI_APPLICATION);
        for (d, s) in nid.szTip.iter_mut().zip("riftlite · Ctrl+Shift+O toggles the overlay".encode_utf16()) {
            *d = s;
        }
        Shell_NotifyIconW(NIM_ADD, &nid);
    }
}

fn toggle_overlay(c: &Ctx) {
    c.shared.overlay_hidden.fetch_xor(true, Ordering::Relaxed);
    // The main pass declares/undeclares the overlay viewport; `logic` requests it even when minimized.
    c.egui.request_repaint();
    c.egui.request_repaint_of(crate::ui::overlay_id());
}

/// The main riftlite window (this process, exact title), so a folder or tab named "riftlite" is never matched.
fn main_window(title: &str) -> Option<HWND> {
    struct Find {
        title: Vec<u16>,
        pid: u32,
        found: HWND,
    }
    unsafe extern "system" fn each(hwnd: HWND, lp: LPARAM) -> windows_sys::core::BOOL {
        let f = unsafe { &mut *(lp as *mut Find) };
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        if pid != f.pid {
            return 1;
        }
        let mut buf = [0u16; 64];
        let n = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) } as usize;
        if buf[..n] == f.title[..] {
            f.found = hwnd;
            return 0;
        }
        1
    }
    let mut f = Find { title: title.encode_utf16().collect(), pid: unsafe { GetCurrentProcessId() }, found: std::ptr::null_mut() };
    unsafe { EnumWindows(Some(each), &mut f as *mut Find as LPARAM) };
    (!f.found.is_null()).then_some(f.found)
}

fn open_main(c: &Ctx) {
    if let Some(h) = main_window(c.main_title) {
        // A click on the tray icon grants this process the right to take the foreground.
        unsafe {
            ShowWindow(h, SW_RESTORE);
            SetForegroundWindow(h);
        }
        c.egui.request_repaint();
    }
}

unsafe fn menu(hwnd: HWND, c: &Ctx) {
    unsafe {
        let m = CreatePopupMenu();
        let hidden = c.shared.overlay_hidden.load(Ordering::Relaxed);
        AppendMenuW(m, MF_STRING, CMD_OPEN, wide("Open riftlite").as_ptr());
        AppendMenuW(m, MF_STRING, CMD_OVERLAY, wide(if hidden { "Show overlay\tCtrl+Shift+O" } else { "Hide overlay\tCtrl+Shift+O" }).as_ptr());
        AppendMenuW(m, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(m, MF_STRING, CMD_QUIT, wide("Quit").as_ptr());
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        // Required so the menu closes when the user clicks elsewhere.
        SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, 0, hwnd, std::ptr::null()) as usize;
        DestroyMenu(m);
        match cmd {
            CMD_OPEN => open_main(c),
            CMD_OVERLAY => toggle_overlay(c),
            CMD_QUIT => {
                Shell_NotifyIconW(NIM_DELETE, &icon_data(hwnd));
                c.egui.send_viewport_cmd(egui::ViewportCommand::Close);
                c.egui.request_repaint();
            }
            _ => {}
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let Some(c) = CTX.get() else { return unsafe { DefWindowProcW(hwnd, msg, wp, lp) } };
    match msg {
        WM_HOTKEY if wp as i32 == HOTKEY_OVERLAY => toggle_overlay(c),
        WM_TRAY => match (lp & 0xffff) as u32 {
            WM_LBUTTONUP => open_main(c),
            WM_RBUTTONUP => unsafe { menu(hwnd, c) },
            _ => {}
        },
        m if m == c.taskbar_created && m != 0 => unsafe { add_icon(hwnd) },
        _ => return unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
    0
}
