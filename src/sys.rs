//! Appels Win32 partagés par le serveur MCP et le processus d'overlay.

use std::ffi::c_void;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::POINT;
use windows_sys::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VK_CONTROL, VK_ESCAPE, VK_MENU,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateCursor, GetCursorPos, GetWindowLongPtrW, SetSystemCursor, SetWindowDisplayAffinity,
    SetWindowLongPtrW, SystemParametersInfoW, GWL_EXSTYLE, OCR_NORMAL, SPI_SETCURSORS,
    WDA_EXCLUDEFROMCAPTURE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
};

/// Coordonnées en pixels physiques : sans cela, un écran mis à l'échelle
/// décale chaque clic. À appeler avant toute création de fenêtre.
pub fn enable_dpi_awareness() {
    // SAFETY: appel sans pointeur ; une valeur de contexte documentée.
    let ok = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    if ok == 0 {
        eprintln!("DPI : contexte déjà fixé ou refusé");
    }
}

/// Position du curseur, en pixels physiques.
pub fn cursor_pos() -> (i32, i32) {
    let mut p = POINT { x: 0, y: 0 };
    // SAFETY: `p` est un POINT valide pour toute la durée de l'appel.
    let ok = unsafe { GetCursorPos(&mut p) };
    if ok == 0 {
        return (0, 0);
    }
    (p.x, p.y)
}

/// Ctrl + Alt + Échap : l'arrêt d'urgence au clavier.
pub fn emergency_hotkey_down() -> bool {
    let down = |vk: u16| {
        // SAFETY: lecture d'état de touche, sans pointeur.
        (unsafe { GetAsyncKeyState(i32::from(vk)) } as u16 & 0x8000) != 0
    };
    down(VK_CONTROL) && down(VK_MENU) && down(VK_ESCAPE)
}

/// Efface la flèche du système : le curseur « change » (il disparaît) tant que
/// l'ordinateur est piloté, et c'est le halo dessiné par l'overlay qui le
/// remplace. Un réticule système (IDC_CROSS) a été essayé d'abord : sa croix
/// se superposait au halo et faisait un double curseur confus. `restore_cursors`
/// rend la flèche.
pub fn hide_system_cursor() {
    const SIDE: i32 = 32;
    const BYTES: usize = (SIDE * SIDE / 8) as usize;
    let and_mask = [0xFFu8; BYTES]; // transparent partout
    let xor_mask = [0x00u8; BYTES];
    // SAFETY: masques de la taille annoncée (32×32 monochrome), vivants pour l'appel ;
    // le curseur produit est copié en interne par CreateCursor.
    unsafe {
        let invisible = CreateCursor(
            null_mut(),
            0,
            0,
            SIDE,
            SIDE,
            and_mask.as_ptr().cast(),
            xor_mask.as_ptr().cast(),
        );
        if invisible.is_null() || SetSystemCursor(invisible, OCR_NORMAL) == 0 {
            eprintln!("curseur : masquage refusé");
        }
    }
}

/// Rend au système ses curseurs d'origine.
pub fn restore_cursors() {
    // SAFETY: SPI_SETCURSORS ne lit aucun pointeur.
    let ok =
        unsafe { SystemParametersInfoW(SPI_SETCURSORS, 0, null::<c_void>() as *mut c_void, 0) };
    if ok == 0 {
        eprintln!("curseur : restauration refusée");
    }
}

/// Cache une fenêtre aux captures d'écran : le modèle ne doit pas voir
/// l'overlay, l'utilisateur si.
pub fn exclude_from_capture(hwnd: isize) {
    // Réservé aux essais : voir l'overlay sur une capture.
    if std::env::var_os("LOCARYN_OVERLAY_VISIBLE_IN_CAPTURE").is_some() {
        return;
    }
    // SAFETY: `hwnd` désigne une fenêtre de ce processus.
    let ok = unsafe { SetWindowDisplayAffinity(hwnd as *mut c_void, WDA_EXCLUDEFROMCAPTURE) };
    if ok == 0 {
        eprintln!("overlay : exclusion des captures refusée (Windows 10 2004 minimum)");
    }
}

/// Une fenêtre qui reçoit des clics sans prendre le focus ni entrer dans la
/// liste des fenêtres : le bouton d'arrêt ne doit pas voler la saisie.
pub fn make_no_activate(hwnd: isize) {
    // SAFETY: `hwnd` désigne une fenêtre de ce processus.
    unsafe {
        let hwnd = hwnd as *mut c_void;
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(
            hwnd,
            GWL_EXSTYLE,
            style | (WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW) as isize,
        );
    }
}
