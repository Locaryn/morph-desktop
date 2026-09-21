//! Fenêtres de premier niveau : lister, mettre au premier plan, agir dessus.

use serde_json::{json, Value};
use std::ffi::c_void;
use std::path::Path;
use windows_sys::Win32::Foundation::{CloseHandle, LPARAM, RECT};
use windows_sys::Win32::Graphics::Dwm::DwmGetWindowAttribute;
use windows_sys::Win32::System::Threading::{
    AttachThreadInput, GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumWindows, GetForegroundWindow, GetWindowLongPtrW, GetWindowRect,
    GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
    IsZoomed, PostMessageW, SetForegroundWindow, ShowWindow, GWL_EXSTYLE, SW_MAXIMIZE, SW_MINIMIZE,
    SW_RESTORE, WM_CLOSE, WS_EX_TOOLWINDOW,
};

const DWMWA_CLOAKED: u32 = 14;

#[derive(Debug, Clone)]
pub struct Win {
    pub hwnd: i64,
    pub title: String,
    pub exe: String,
    pub pid: u32,
    pub rect: [i32; 4],
    pub minimized: bool,
    pub maximized: bool,
    pub foreground: bool,
}

impl Win {
    pub fn to_json(&self) -> Value {
        json!({
            "hwnd": self.hwnd, "title": self.title, "app": self.exe, "pid": self.pid,
            "rect": self.rect, "minimized": self.minimized, "maximized": self.maximized,
            "foreground": self.foreground,
        })
    }
}

fn ptr(hwnd: i64) -> *mut c_void {
    hwnd as isize as *mut c_void
}

unsafe extern "system" fn collect(hwnd: *mut c_void, lparam: LPARAM) -> i32 {
    // SAFETY: `lparam` est le pointeur vers le Vec passé par `list`, vivant pendant l'énumération.
    let out = &mut *(lparam as *mut Vec<Win>);
    if let Some(w) = describe(hwnd) {
        out.push(w);
    }
    1
}

/// Une fenêtre visible, titrée, qui n'est pas une fenêtre d'outil ni cachée par Windows.
fn describe(hwnd: *mut c_void) -> Option<Win> {
    // SAFETY: appels de lecture sur un HWND fourni par EnumWindows ; les tampons sont locaux.
    unsafe {
        if IsWindowVisible(hwnd) == 0
            || GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW != 0
        {
            return None;
        }
        let mut cloaked: u32 = 0;
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut c_void,
            4,
        );
        let len = GetWindowTextLengthW(hwnd);
        if cloaked != 0 || len == 0 {
            return None;
        }
        let mut buf = vec![0u16; len as usize + 1];
        let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        let title = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
        let mut r = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        GetWindowRect(hwnd, &mut r);
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        Some(Win {
            hwnd: hwnd as isize as i64,
            title,
            exe: exe_name(pid),
            pid,
            rect: [r.left, r.top, r.right - r.left, r.bottom - r.top],
            minimized: IsIconic(hwnd) != 0,
            maximized: IsZoomed(hwnd) != 0,
            foreground: GetForegroundWindow() == hwnd,
        })
    }
}

fn exe_name(pid: u32) -> String {
    // SAFETY: le handle est fermé avant de sortir ; le tampon est local.
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return String::new();
        }
        let mut buf = vec![0u16; 520];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len);
        CloseHandle(h);
        if ok == 0 {
            return String::new();
        }
        let full = String::from_utf16_lossy(&buf[..len as usize]);
        Path::new(&full)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

pub fn list() -> Vec<Win> {
    let mut out: Vec<Win> = Vec::new();
    // SAFETY: `out` vit plus longtemps que l'énumération, synchrone.
    unsafe {
        EnumWindows(Some(collect), &mut out as *mut Vec<Win> as LPARAM);
    }
    out
}

pub fn foreground() -> Option<i64> {
    // SAFETY: appel sans pointeur.
    let h = unsafe { GetForegroundWindow() };
    (!h.is_null()).then_some(h as isize as i64)
}

/// Met la fenêtre au premier plan. Windows refuse à un processus d'arrière-plan
/// de voler le focus ; se rattacher au fil de la fenêtre active lève la limite.
pub fn focus(hwnd: i64) -> Result<(), String> {
    let h = ptr(hwnd);
    // SAFETY: appels de gestion de fenêtre sur un HWND ; l'attache est toujours défaite.
    unsafe {
        if IsIconic(h) != 0 {
            ShowWindow(h, SW_RESTORE);
        }
        let fg = GetForegroundWindow();
        let fg_thread = GetWindowThreadProcessId(fg, std::ptr::null_mut());
        let me = GetCurrentThreadId();
        let attached = fg_thread != me && AttachThreadInput(me, fg_thread, 1) != 0;
        BringWindowToTop(h);
        let ok = SetForegroundWindow(h);
        if attached {
            AttachThreadInput(me, fg_thread, 0);
        }
        if ok == 0 {
            return Err("Windows a refusé de mettre cette fenêtre au premier plan".into());
        }
    }
    Ok(())
}

pub fn act(hwnd: i64, action: &str) -> Result<(), String> {
    let h = ptr(hwnd);
    // SAFETY: appels de gestion de fenêtre sur un HWND.
    unsafe {
        match action {
            "minimize" => ShowWindow(h, SW_MINIMIZE),
            "maximize" => ShowWindow(h, SW_MAXIMIZE),
            "restore" => ShowWindow(h, SW_RESTORE),
            "close" => PostMessageW(h, WM_CLOSE, 0, 0),
            autre => {
                return Err(format!(
                    "Action inconnue : {autre} (minimize, maximize, restore, close)"
                ))
            }
        };
    }
    Ok(())
}
