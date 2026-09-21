//! Souris et clavier, par `SendInput`.
//!
//! Le curseur est déplacé en douceur, pas téléporté : l'utilisateur suit le
//! geste, et l'overlay peut dessiner le halo qui l'accompagne.

use std::mem::size_of;
use std::thread::sleep;
use std::time::Duration;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_RIGHTDOWN,
    MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL, MOUSEINPUT, VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN,
    VK_END, VK_ESCAPE, VK_HOME, VK_INSERT, VK_LEFT, VK_LWIN, VK_MENU, VK_NEXT, VK_PRIOR, VK_RETURN,
    VK_RIGHT, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::SetCursorPos;

const STEP_PAUSE: Duration = Duration::from_millis(12);
const CLICK_HOLD: Duration = Duration::from_millis(45);
const TYPE_PAUSE: Duration = Duration::from_millis(6);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Middle,
}

impl Button {
    pub fn parse(s: Option<&str>) -> Result<Self, String> {
        match s.unwrap_or("left") {
            "left" => Ok(Self::Left),
            "right" => Ok(Self::Right),
            "middle" => Ok(Self::Middle),
            autre => Err(format!("bouton inconnu : {autre} (left, right, middle)")),
        }
    }

    fn flags(self) -> (u32, u32) {
        match self {
            Self::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
            Self::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
            Self::Middle => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
        }
    }
}

fn send(inputs: &[INPUT]) -> Result<(), String> {
    // SAFETY: `inputs` est un tableau d'INPUT initialisés, la taille est celle du type.
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            size_of::<INPUT>() as i32,
        )
    };
    if sent as usize == inputs.len() {
        Ok(())
    } else {
        Err("Windows a refusé l'entrée (fenêtre élevée en administrateur au premier plan ?)".into())
    }
}

fn mouse(flags: u32, data: i32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: data as u32,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn key(vk: u16, scan: u16, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

// ── Souris ──────────────────────────────────────────────────────────────────

/// Amène le curseur en (x, y) par un mouvement adouci, proportionnel à la distance.
pub fn move_to(x: i32, y: i32) -> Result<(), String> {
    let (sx, sy) = crate::sys::cursor_pos();
    let dist = f64::from(x - sx).hypot(f64::from(y - sy));
    let steps = ((dist / 40.0) as usize).clamp(6, 28);
    for i in 1..=steps {
        let t = i as f64 / steps as f64;
        let e = 1.0 - (1.0 - t).powi(3); // départ vif, arrivée douce
        let px = f64::from(sx) + f64::from(x - sx) * e;
        let py = f64::from(sy) + f64::from(y - sy) * e;
        set_cursor(px.round() as i32, py.round() as i32)?;
        sleep(STEP_PAUSE);
    }
    set_cursor(x, y)
}

fn set_cursor(x: i32, y: i32) -> Result<(), String> {
    // SAFETY: appel sans pointeur.
    if unsafe { SetCursorPos(x, y) } == 0 {
        return Err(format!("Impossible de placer le curseur en ({x}, {y})"));
    }
    Ok(())
}

pub fn click(button: Button, count: u32) -> Result<(), String> {
    let (down, up) = button.flags();
    for _ in 0..count {
        send(&[mouse(down, 0)])?;
        sleep(CLICK_HOLD);
        send(&[mouse(up, 0)])?;
        sleep(CLICK_HOLD);
    }
    Ok(())
}

pub fn drag(from: (i32, i32), to: (i32, i32)) -> Result<(), String> {
    move_to(from.0, from.1)?;
    let (down, up) = Button::Left.flags();
    send(&[mouse(down, 0)])?;
    sleep(CLICK_HOLD);
    let moved = move_to(to.0, to.1);
    sleep(CLICK_HOLD);
    send(&[mouse(up, 0)])?; // relâcher même si le déplacement a échoué
    moved
}

/// `dy` > 0 : vers le bas ; `dx` > 0 : vers la droite. Unité : crans de molette.
pub fn scroll(dy: i32, dx: i32) -> Result<(), String> {
    if dy != 0 {
        send(&[mouse(MOUSEEVENTF_WHEEL, -dy * 120)])?;
    }
    if dx != 0 {
        send(&[mouse(MOUSEEVENTF_HWHEEL, dx * 120)])?;
    }
    Ok(())
}

// ── Clavier ─────────────────────────────────────────────────────────────────

/// Saisit du texte tel quel (Unicode), sans dépendre de la disposition du clavier.
pub fn type_text(text: &str) -> Result<(), String> {
    for unit in text.replace("\r\n", "\n").encode_utf16() {
        if unit == u16::from(b'\n') {
            tap(VK_RETURN, false)?;
            continue;
        }
        send(&[
            key(0, unit, KEYEVENTF_UNICODE),
            key(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP),
        ])?;
        sleep(TYPE_PAUSE);
    }
    Ok(())
}

fn tap(vk: u16, extended: bool) -> Result<(), String> {
    let ext = if extended { KEYEVENTF_EXTENDEDKEY } else { 0 };
    send(&[key(vk, 0, ext), key(vk, 0, ext | KEYEVENTF_KEYUP)])
}

/// Une combinaison comme `ctrl+shift+t`, `alt+f4`, `enter`, `win+d`.
pub fn press_combo(combo: &str) -> Result<(), String> {
    let parts: Vec<&str> = combo
        .split('+')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    let Some((last, mods)) = parts.split_last() else {
        return Err("combinaison vide".into());
    };
    let mod_keys = mods
        .iter()
        .map(|m| vk_of(m).ok_or_else(|| format!("touche inconnue : {m}")))
        .collect::<Result<Vec<_>, _>>()?;
    let (vk, extended) = vk_of(last).ok_or_else(|| format!("touche inconnue : {last}"))?;
    for (m, e) in &mod_keys {
        send(&[key(*m, 0, ext_flag(*e))])?;
    }
    let pressed = tap(vk, extended);
    for (m, e) in mod_keys.iter().rev() {
        send(&[key(*m, 0, ext_flag(*e) | KEYEVENTF_KEYUP)])?; // toujours relâcher
    }
    pressed
}

fn ext_flag(extended: bool) -> u32 {
    if extended {
        KEYEVENTF_EXTENDEDKEY
    } else {
        0
    }
}

/// Code de touche virtuelle et drapeau « étendue ».
fn vk_of(name: &str) -> Option<(u16, bool)> {
    let n = name.to_lowercase();
    let fixed = match n.as_str() {
        "ctrl" | "control" => (VK_CONTROL, false),
        "shift" => (VK_SHIFT, false),
        "alt" => (VK_MENU, false),
        "win" | "meta" | "cmd" | "super" => (VK_LWIN, true),
        "enter" | "return" => (VK_RETURN, false),
        "tab" => (VK_TAB, false),
        "esc" | "escape" => (VK_ESCAPE, false),
        "space" => (VK_SPACE, false),
        "backspace" => (VK_BACK, false),
        "delete" | "del" => (VK_DELETE, true),
        "insert" => (VK_INSERT, true),
        "home" => (VK_HOME, true),
        "end" => (VK_END, true),
        "pageup" => (VK_PRIOR, true),
        "pagedown" => (VK_NEXT, true),
        "up" | "arrowup" => (VK_UP, true),
        "down" | "arrowdown" => (VK_DOWN, true),
        "left" | "arrowleft" => (VK_LEFT, true),
        "right" | "arrowright" => (VK_RIGHT, true),
        _ => return simple_key(&n),
    };
    Some(fixed)
}

fn simple_key(n: &str) -> Option<(u16, bool)> {
    let mut chars = n.chars();
    let first = chars.next()?;
    if chars.next().is_none() && first.is_ascii_alphanumeric() {
        return Some((first.to_ascii_uppercase() as u16, false)); // VK_A..VK_Z, VK_0..VK_9
    }
    let num: u16 = n.strip_prefix('f')?.parse().ok()?;
    (1..=24).contains(&num).then_some((0x6F + num, false)) // VK_F1 = 0x70
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_touches_courantes_sont_reconnues() {
        assert_eq!(vk_of("Ctrl"), Some((VK_CONTROL, false)));
        assert_eq!(vk_of("a"), Some((0x41, false)));
        assert_eq!(vk_of("5"), Some((0x35, false)));
        assert_eq!(vk_of("F4"), Some((0x73, false)));
        assert_eq!(vk_of("PageDown"), Some((VK_NEXT, true)));
    }

    #[test]
    fn une_touche_inconnue_est_refusee() {
        assert_eq!(vk_of("f99"), None);
        assert_eq!(vk_of("hyperkey"), None);
    }

    #[test]
    fn un_bouton_inconnu_est_refuse() {
        assert!(Button::parse(Some("gauche")).is_err());
        assert_eq!(Button::parse(None), Ok(Button::Left));
    }
}
