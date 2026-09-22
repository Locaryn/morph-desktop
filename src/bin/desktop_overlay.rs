//! Overlay plein écran : cadre animé, curseur, ondes de clic et bouton d'arrêt
//! d'urgence. Lancé et piloté par `locaryn-desktop-mcp` (une ligne JSON par
//! commande sur stdin, événements sur stdout).
//!
//! Si le serveur disparaît, stdin se ferme : l'overlay rend alors les curseurs
//! au système et s'arrête, il ne laisse jamais le réticule en place.
#![cfg(windows)]

use locaryn_plugin_desktop::overlay_proto::{Command, Event as Out, PulseKind};
use locaryn_plugin_desktop::sys;
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tao::dpi::{PhysicalPosition, PhysicalSize};
use tao::event::Event;
use tao::event_loop::{ControlFlow, EventLoop, EventLoopBuilder, EventLoopProxy};
use tao::monitor::MonitorHandle;
use tao::platform::windows::{WindowBuilderExtWindows, WindowExtWindows};
use tao::window::{Window, WindowBuilder};
use wry::{WebView, WebViewBuilder};

const FRAME_HTML: &str = include_str!("../../overlay_ui/frame.html");
const CONTROL_HTML: &str = include_str!("../../overlay_ui/control.html");
const CONTROL_SIZE: (f64, f64) = (300.0, 84.0); // deux boutons côte à côte en mode actif
/// Le bouton se place entre le centre et le bas de l'écran.
const CONTROL_HEIGHT_RATIO: f64 = 0.74;
const FADE_OUT: Duration = Duration::from_millis(550);

enum Msg {
    Cmd(Command),
    Cursor(i32, i32),
    /// `true` = arrêt complet (bouton Arrêt) ; `false` = pause (bouton Pause,
    /// ou Ctrl+Alt+Échap).
    Halt(bool),
    Resume,
    HideIfIdle,
    Quit,
}

struct Screen {
    window: Window,
    view: WebView,
    origin: (i32, i32),
    size: (i32, i32),
    scale: f64,
}

struct Control {
    window: Window,
    view: WebView,
}

fn main() {
    sys::enable_dpi_awareness();
    sys::restore_cursors(); // un précédent overlay a pu mourir avec le réticule
    let event_loop: EventLoop<Msg> = EventLoopBuilder::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let (screens, control) = match build_windows(&event_loop, &proxy) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("overlay : {e}");
            std::process::exit(1);
        }
    };
    let active = Arc::new(AtomicBool::new(false));
    spawn_stdin_reader(proxy.clone());
    spawn_cursor_poller(proxy.clone(), active.clone());
    emit(&Out::Ready);
    run(event_loop, proxy, screens, control, active);
}

fn emit(event: &Out) {
    match serde_json::to_string(event) {
        Ok(line) => {
            println!("{line}");
            if let Err(e) = std::io::stdout().flush() {
                eprintln!("overlay : sortie fermée : {e}");
            }
        }
        Err(e) => eprintln!("overlay : événement illisible : {e}"),
    }
}

// ── Construction des fenêtres ───────────────────────────────────────────────

fn build_windows(
    target: &EventLoop<Msg>,
    proxy: &EventLoopProxy<Msg>,
) -> Result<(Vec<Screen>, Control), String> {
    let monitors: Vec<MonitorHandle> = target.available_monitors().collect();
    if monitors.is_empty() {
        return Err("aucun écran détecté".into());
    }
    let screens = monitors
        .iter()
        .map(|m| build_screen(target, m))
        .collect::<Result<Vec<_>, _>>()?;
    let primary = target
        .primary_monitor()
        .unwrap_or_else(|| monitors[0].clone());
    let control = build_control(target, &primary, proxy.clone())?;
    Ok((screens, control))
}

fn base_window(
    target: &EventLoop<Msg>,
    pos: (i32, i32),
    size: (u32, u32),
) -> Result<Window, String> {
    let window = WindowBuilder::new()
        .with_decorations(false)
        .with_transparent(true)
        .with_always_on_top(true)
        .with_skip_taskbar(true)
        .with_undecorated_shadow(false)
        .with_visible(false)
        .with_focused(false)
        .with_resizable(false)
        .with_position(PhysicalPosition::new(pos.0, pos.1))
        .with_inner_size(PhysicalSize::new(size.0, size.1))
        .build(target)
        .map_err(|e| format!("fenêtre : {e}"))?;
    let hwnd = window.hwnd();
    sys::exclude_from_capture(hwnd);
    sys::make_no_activate(hwnd);
    Ok(window)
}

fn build_screen(target: &EventLoop<Msg>, mon: &MonitorHandle) -> Result<Screen, String> {
    let (p, s) = (mon.position(), mon.size());
    let window = base_window(target, (p.x, p.y), (s.width, s.height))?;
    window
        .set_ignore_cursor_events(true)
        .map_err(|e| format!("clics traversants : {e}"))?;
    let view = WebViewBuilder::new()
        .with_transparent(true)
        .with_background_color((0, 0, 0, 0))
        .with_html(FRAME_HTML)
        .build(&window)
        .map_err(|e| format!("webview du cadre : {e}"))?;
    Ok(Screen {
        window,
        view,
        origin: (p.x, p.y),
        size: (s.width as i32, s.height as i32),
        scale: mon.scale_factor(),
    })
}

fn build_control(
    target: &EventLoop<Msg>,
    mon: &MonitorHandle,
    proxy: EventLoopProxy<Msg>,
) -> Result<Control, String> {
    let (p, s, k) = (mon.position(), mon.size(), mon.scale_factor());
    let (w, h) = ((CONTROL_SIZE.0 * k) as u32, (CONTROL_SIZE.1 * k) as u32);
    let x = p.x + (s.width as i32 - w as i32) / 2;
    let y = p.y + (s.height as f64 * CONTROL_HEIGHT_RATIO) as i32;
    let window = base_window(target, (x, y), (w, h))?;
    let view = WebViewBuilder::new()
        .with_transparent(true)
        .with_background_color((0, 0, 0, 0))
        .with_html(CONTROL_HTML)
        .with_ipc_handler(move |req| {
            let msg = match req.body().as_str() {
                "pause" => Msg::Halt(false),
                "kill" => Msg::Halt(true),
                "resume" => Msg::Resume,
                _ => return,
            };
            if proxy.send_event(msg).is_err() {
                eprintln!("overlay : boucle d'événements fermée");
            }
        })
        .build(&window)
        .map_err(|e| format!("webview du bouton : {e}"))?;
    Ok(Control { window, view })
}

// ── Sources d'événements ────────────────────────────────────────────────────

fn spawn_stdin_reader(proxy: EventLoopProxy<Msg>) {
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            match serde_json::from_str::<Command>(&line) {
                Ok(cmd) => {
                    if proxy.send_event(Msg::Cmd(cmd)).is_err() {
                        return;
                    }
                }
                Err(e) => eprintln!("overlay : commande illisible : {e}"),
            }
        }
        // Le serveur est parti : on ne laisse rien derrière soi.
        if proxy.send_event(Msg::Quit).is_err() {
            sys::restore_cursors();
            std::process::exit(0);
        }
    });
}

fn spawn_cursor_poller(proxy: EventLoopProxy<Msg>, active: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let mut last = (i32::MIN, i32::MIN);
        loop {
            if !active.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(120));
                continue;
            }
            std::thread::sleep(Duration::from_millis(16));
            let now = sys::cursor_pos();
            let sent = if sys::emergency_hotkey_down() {
                proxy.send_event(Msg::Halt(false))
            } else if now != last {
                last = now;
                proxy.send_event(Msg::Cursor(now.0, now.1))
            } else {
                Ok(())
            };
            if sent.is_err() {
                return;
            }
        }
    });
}

// ── Boucle principale ───────────────────────────────────────────────────────

fn run(
    event_loop: EventLoop<Msg>,
    proxy: EventLoopProxy<Msg>,
    screens: Vec<Screen>,
    control: Control,
    active: Arc<AtomicBool>,
) {
    let mut stopped = false;
    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        let Event::UserEvent(msg) = event else { return };
        match msg {
            Msg::Cmd(Command::Active { label }) if !stopped => {
                show(&screens, &control, &label, &active);
            }
            Msg::Cmd(Command::Active { .. }) => {}
            Msg::Cmd(Command::Halt { hard }) => {
                stopped = true;
                halt(&screens, &control, &active, hard);
            }
            Msg::Cmd(Command::Idle) => fade_out(&screens, &active, &proxy, stopped),
            Msg::Cmd(Command::Thinking) if !stopped && active.load(Ordering::Relaxed) => {
                think(&screens);
            }
            Msg::Cmd(Command::Thinking) => {}
            Msg::Cmd(Command::Pulse { kind, x, y }) => pulse(&screens, kind, x, y),
            Msg::Cursor(x, y) => move_cursor(&screens, x, y),
            Msg::HideIfIdle => {
                if !active.load(Ordering::Relaxed) {
                    hide(&screens, &control, stopped);
                }
            }
            Msg::Halt(hard) if !stopped && active.load(Ordering::Relaxed) => {
                stopped = true;
                emergency_stop(&screens, &control, &active, hard);
            }
            Msg::Halt(_) => {}
            Msg::Resume => {
                stopped = false;
                run_js(&control.view, "window.__mode('active')");
                control.window.set_visible(false);
                emit(&Out::Resume);
            }
            Msg::Quit => {
                sys::restore_cursors();
                *flow = ControlFlow::Exit;
            }
        }
    });
}

fn run_js(view: &WebView, script: &str) {
    if let Err(e) = view.evaluate_script(script) {
        eprintln!("overlay : script refusé : {e}");
    }
}

fn js_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

fn show(screens: &[Screen], control: &Control, label: &str, active: &AtomicBool) {
    if !active.swap(true, Ordering::Relaxed) {
        sys::hide_system_cursor();
    }
    for s in screens {
        s.window.set_visible(true);
        run_js(&s.view, &format!("window.__active({})", js_str(label)));
    }
    run_js(&control.view, "window.__mode('active')");
    control.window.set_visible(true);
}

fn think(screens: &[Screen]) {
    for s in screens {
        run_js(&s.view, "window.__thinking()");
    }
}

fn fade_out(screens: &[Screen], active: &AtomicBool, proxy: &EventLoopProxy<Msg>, stopped: bool) {
    if !active.swap(false, Ordering::Relaxed) && !stopped {
        return;
    }
    sys::restore_cursors();
    for s in screens {
        run_js(&s.view, "window.__idle()");
    }
    let proxy = proxy.clone();
    std::thread::spawn(move || {
        std::thread::sleep(FADE_OUT);
        if proxy.send_event(Msg::HideIfIdle).is_err() {
            eprintln!("overlay : boucle d'événements fermée");
        }
    });
}

fn hide(screens: &[Screen], control: &Control, stopped: bool) {
    for s in screens {
        s.window.set_visible(false);
    }
    if !stopped {
        control.window.set_visible(false);
    }
}

fn emergency_stop(screens: &[Screen], control: &Control, active: &AtomicBool, hard: bool) {
    halt(screens, control, active, hard);
    emit(&Out::Stop { hard });
}

fn halt(screens: &[Screen], control: &Control, active: &AtomicBool, hard: bool) {
    active.store(false, Ordering::Relaxed);
    sys::restore_cursors();
    for s in screens {
        run_js(&s.view, "window.__idle()");
        s.window.set_visible(false);
    }
    run_js(
        &control.view,
        &format!("window.__mode('{}')", if hard { "hard" } else { "soft" }),
    );
    control.window.set_visible(true);
}

fn local_point(s: &Screen, x: i32, y: i32) -> Option<(f64, f64)> {
    let (rx, ry) = (x - s.origin.0, y - s.origin.1);
    let inside = (0..s.size.0).contains(&rx) && (0..s.size.1).contains(&ry);
    inside.then(|| (f64::from(rx) / s.scale, f64::from(ry) / s.scale))
}

fn move_cursor(screens: &[Screen], x: i32, y: i32) {
    for s in screens {
        if let Some((lx, ly)) = local_point(s, x, y) {
            run_js(&s.view, &format!("window.__cur({lx:.1},{ly:.1})"));
        }
    }
}

fn pulse(screens: &[Screen], kind: PulseKind, x: i32, y: i32) {
    let name = match kind {
        PulseKind::Left => "left",
        PulseKind::Right => "right",
        PulseKind::Double => "double",
    };
    for s in screens {
        if let Some((lx, ly)) = local_point(s, x, y) {
            run_js(
                &s.view,
                &format!("window.__pulse({lx:.1},{ly:.1},'{name}')"),
            );
        }
    }
}
