//! Notification-area icon, the global overlay hotkeys (Ctrl+Shift+O, Ctrl+Shift+E) and
//! foreground tracking, all owned by one thread with a message-only window.
//! Plain Win32; the WinEvent hook is out-of-context (nothing is injected).

use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};

use eframe::egui;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, RegisterHotKey};
use windows_sys::Win32::UI::Shell::{NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DispatchMessageW, EVENT_SYSTEM_FOREGROUND,
    GetCursorPos, GetForegroundWindow, GetMessageW, GetSystemMetrics, HWND_MESSAGE, IDI_APPLICATION, IMAGE_ICON, LR_SHARED,
    LoadIconW, LoadImageW, MF_SEPARATOR, MF_STRING, MSG, RegisterClassW, RegisterWindowMessageW, SM_CXSMICON, SM_CYSMICON,
    SW_RESTORE, SetForegroundWindow, ShowWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WINEVENT_OUTOFCONTEXT,
    WM_APP, WM_HOTKEY, WM_LBUTTONUP, WM_RBUTTONUP, WNDCLASSW,
};

use crate::worker::Shared;

const WM_TRAY: u32 = WM_APP + 1;
const HOTKEY_OVERLAY: i32 = 1;
const HOTKEY_EDIT: i32 = 2;
const CMD_OPEN: usize = 1;
const CMD_OVERLAY: usize = 2;
const CMD_QUIT: usize = 3;
const CMD_EDIT: usize = 4;

/// Which global hotkeys could be registered (another program may own them).
#[derive(Clone, Copy, Debug)]
pub struct Hotkeys {
    /// Ctrl+Shift+O: hide/show the overlay.
    pub overlay: bool,
    /// Ctrl+Shift+E: overlay edit mode.
    pub edit: bool,
}

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

/// Starts the tray thread. Returns which hotkeys could be registered.
pub fn spawn(egui: egui::Context, shared: Arc<Shared>, main_title: &'static str) -> std::sync::mpsc::Receiver<Hotkeys> {
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
                let _ = tx.send(Hotkeys { overlay: false, edit: false });
                return;
            }
            add_icon(hwnd);
            let hotkey = |id, key: char| RegisterHotKey(hwnd, id, MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT, key as u32) != 0;
            let _ = tx.send(Hotkeys { overlay: hotkey(HOTKEY_OVERLAY, 'O'), edit: hotkey(HOTKEY_EDIT, 'E') });
            // Out-of-context callbacks arrive through this thread's message loop.
            SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                std::ptr::null_mut(),
                Some(on_foreground),
                0,
                0,
                WINEVENT_OUTOFCONTEXT,
            );
            foreground_changed(GetForegroundWindow());
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
        // Resource 1 is riftlite's icon, embedded by build.rs; LR_SHARED so re-adding after an
        // Explorer restart does not leak a handle.
        let size = (GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON));
        let own = LoadImageW(GetModuleHandleW(std::ptr::null()), 1 as _, IMAGE_ICON, size.0, size.1, LR_SHARED);
        nid.hIcon = if own.is_null() { LoadIconW(std::ptr::null_mut(), IDI_APPLICATION) } else { own };
        for (d, s) in nid.szTip.iter_mut().zip("riftlite · Ctrl+Shift+O toggles the overlay, Ctrl+Shift+E edits it".encode_utf16()) {
            *d = s;
        }
        Shell_NotifyIconW(NIM_ADD, &nid);
    }
}

/// The overlay is shown only while the League game window has the foreground. Clicking
/// the overlay in edit mode activates it; that keeps the previous state so the overlay
/// stays up and is still there after edit mode ends.
fn foreground_changed(hwnd: HWND) {
    let Some(c) = CTX.get() else { return };
    if !hwnd.is_null() && crate::win::own_window(crate::ui::OVERLAY_TITLE) == Some(hwnd) {
        return;
    }
    let focused = crate::win::is_league_game(hwnd);
    if c.shared.game_focused.swap(focused, Ordering::Relaxed) != focused {
        // Same path as the hotkey: the main pass (re)declares the overlay viewport.
        c.egui.request_repaint();
        c.egui.request_repaint_of(crate::ui::overlay_id());
    }
}

unsafe extern "system" fn on_foreground(_: HWINEVENTHOOK, _: u32, hwnd: HWND, _: i32, _: i32, _: u32, _: u32) {
    foreground_changed(hwnd);
}

fn toggle_overlay(c: &Ctx) {
    c.shared.overlay_hidden.fetch_xor(true, Ordering::Relaxed);
    // The main pass declares/undeclares the overlay viewport; `logic` requests it even when minimized.
    c.egui.request_repaint();
    c.egui.request_repaint_of(crate::ui::overlay_id());
}

fn toggle_edit(c: &Ctx) {
    let on = !c.shared.overlay_edit.load(Ordering::Relaxed);
    crate::ui::set_overlay_edit(&c.egui, &c.shared, on);
}

fn open_main(c: &Ctx) {
    if let Some(h) = crate::win::own_window(c.main_title) {
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
        let edit = c.shared.overlay_edit.load(Ordering::Relaxed);
        AppendMenuW(m, MF_STRING, CMD_OPEN, wide("Open riftlite").as_ptr());
        AppendMenuW(m, MF_STRING, CMD_OVERLAY, wide(if hidden { "Show overlay\tCtrl+Shift+O" } else { "Hide overlay\tCtrl+Shift+O" }).as_ptr());
        let edit_label = if edit { "Finish editing overlay\tCtrl+Shift+E" } else { "Edit overlay layout\tCtrl+Shift+E" };
        AppendMenuW(m, MF_STRING, CMD_EDIT, wide(edit_label).as_ptr());
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
            CMD_EDIT => toggle_edit(c),
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
        WM_HOTKEY if wp as i32 == HOTKEY_EDIT => toggle_edit(c),
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
