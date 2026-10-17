//! Thin wrappers over the Win32 calls riftlite needs: process enumeration,
//! another process's image path and command line, local time conversion and
//! a single-instance mutex.

use std::path::PathBuf;
use std::ptr::null_mut;

use windows_sys::Wdk::System::Threading::{NtQueryInformationProcess, ProcessCommandLineInformation};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, FILETIME, GetLastError, HANDLE, HWND, INVALID_HANDLE_VALUE, LPARAM, LRESULT,
    SYSTEMTIME, UNICODE_STRING, WPARAM,
};
use windows_sys::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    CreateMutexW, GetCurrentProcessId, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowW, GetWindowTextW, GetWindowThreadProcessId, SW_RESTORE, SWP_FRAMECHANGED, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetForegroundWindow, SetWindowPos, ShowWindow, WM_NCCALCSIZE,
};

pub struct Process {
    pub pid: u32,
    /// Executable file name, lowercased (e.g. `leagueclientux.exe`).
    pub name: String,
}

/// Unix milliseconds → local (year, month, day, hour, minute).
pub fn local_time(unix_ms: i64) -> (u16, u16, u16, u16, u16) {
    // FILETIME counts 100 ns ticks since 1601-01-01.
    let ticks = (unix_ms.max(0) as u64) * 10_000 + 116_444_736_000_000_000;
    let ft = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
    let mut utc: SYSTEMTIME = unsafe { std::mem::zeroed() };
    let mut local: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe {
        if FileTimeToSystemTime(&ft, &mut utc) == 0 {
            return (1970, 1, 1, 0, 0);
        }
        if SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local) == 0 {
            local = utc;
        }
    }
    (local.wYear, local.wMonth, local.wDay, local.wHour, local.wMinute)
}

struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn from_wide(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

pub fn processes() -> Vec<Process> {
    let mut out = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return out;
        }
        let snap = Handle(snap);
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snap.0, &mut entry) != 0 {
            loop {
                out.push(Process { pid: entry.th32ProcessID, name: from_wide(&entry.szExeFile).to_lowercase() });
                if Process32NextW(snap.0, &mut entry) == 0 {
                    break;
                }
            }
        }
    }
    out
}

fn open(pid: u32) -> Option<Handle> {
    let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    (!h.is_null()).then_some(Handle(h))
}

pub fn image_path(pid: u32) -> Option<PathBuf> {
    let h = open(pid)?;
    let mut buf = vec![0u16; 32768];
    let mut len = buf.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(h.0, 0, buf.as_mut_ptr(), &mut len) };
    (ok != 0).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])))
}

/// Reads another (non-elevated) process's command line via
/// `NtQueryInformationProcess(ProcessCommandLineInformation)`; needs only
/// `PROCESS_QUERY_LIMITED_INFORMATION`, no debug privileges or injection.
pub fn command_line(pid: u32) -> Option<String> {
    let h = open(pid)?;
    let mut len = 0u32;
    unsafe { NtQueryInformationProcess(h.0, ProcessCommandLineInformation, null_mut(), 0, &mut len) };
    if len == 0 {
        return None;
    }
    // u64 backing keeps the UNICODE_STRING header aligned.
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    let status = unsafe {
        NtQueryInformationProcess(h.0, ProcessCommandLineInformation, buf.as_mut_ptr().cast(), len, &mut len)
    };
    if status < 0 {
        return None;
    }
    let us = unsafe { &*(buf.as_ptr() as *const UNICODE_STRING) };
    if us.Buffer.is_null() {
        return None;
    }
    let chars = unsafe { std::slice::from_raw_parts(us.Buffer, us.Length as usize / 2) };
    Some(String::from_utf16_lossy(chars))
}

/// The window belongs to the League game process (`League of Legends.exe`).
pub fn is_league_game(hwnd: HWND) -> bool {
    if hwnd.is_null() {
        return false;
    }
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    pid != 0
        && image_path(pid)
            .and_then(|p| p.file_name().map(|n| n.eq_ignore_ascii_case("League of Legends.exe")))
            .unwrap_or(false)
}

/// This process's top-level window with exactly `title`, so a folder or browser tab
/// with the same name is never matched.
pub fn own_window(title: &str) -> Option<HWND> {
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
    let mut f = Find { title: title.encode_utf16().collect(), pid: unsafe { GetCurrentProcessId() }, found: null_mut() };
    unsafe { EnumWindows(Some(each), &mut f as *mut Find as LPARAM) };
    (!f.found.is_null()).then_some(f.found)
}

/// Removes the drop shadow winit draws around this process's undecorated window `title`.
///
/// winit keeps `WS_CAPTION` on undecorated windows and, with its undecorated-shadow flag
/// (which egui-winit always sets on Windows), leaves a 1 px non-client strip in
/// `WM_NCCALCSIZE` so DWM draws a shadow. Answering `WM_NCCALCSIZE` with 0 makes the whole
/// window client area: no strip, no shadow. Disabling DWM non-client rendering instead
/// would paint that strip as a classic 1 px frame line. False if the window is not found (yet).
pub fn remove_window_shadow(title: &str) -> bool {
    unsafe extern "system" fn proc_(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _id: usize, _data: usize) -> LRESULT {
        if msg == WM_NCCALCSIZE && wp != 0 {
            return 0;
        }
        unsafe { DefSubclassProc(hwnd, msg, wp, lp) }
    }
    let Some(hwnd) = own_window(title) else { return false };
    unsafe {
        // Must run on the window's thread (eframe's event loop thread, where viewport callbacks run).
        if SetWindowSubclass(hwnd, Some(proc_), 1, 0) == 0 {
            return false;
        }
        // Recompute the frame through the subclass now.
        SetWindowPos(hwnd, null_mut(), 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
    }
    true
}

/// Returns false if another riftlite instance already holds the mutex; in that
/// case the existing window is brought to the front.
pub fn acquire_single_instance(window_title: &str) -> bool {
    let name = wide("Local\\riftlite-single-instance");
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    if handle.is_null() {
        return true; // cannot enforce; run anyway
    }
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        let title = wide(window_title);
        let hwnd = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
        if !hwnd.is_null() {
            unsafe {
                ShowWindow(hwnd, SW_RESTORE);
                SetForegroundWindow(hwnd);
            }
        }
        return false;
    }
    // Intentionally leaked: the mutex must live as long as the process.
    true
}
