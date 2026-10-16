//! Thin wrappers over the Win32 calls riftlite needs: process enumeration,
//! another process's image path and command line, local time conversion and
//! a single-instance mutex.

use std::path::PathBuf;
use std::ptr::null_mut;

use windows_sys::Wdk::System::Threading::{NtQueryInformationProcess, ProcessCommandLineInformation};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, FILETIME, GetLastError, HANDLE, INVALID_HANDLE_VALUE, SYSTEMTIME, UNICODE_STRING,
};
use windows_sys::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    CreateMutexW, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, SW_RESTORE, SetForegroundWindow, ShowWindow};

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
