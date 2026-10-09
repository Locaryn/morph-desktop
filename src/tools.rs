//! Outils MCP de l'ordinateur.
//!
//! Chaque outil commence par montrer l'overlay : si l'utilisateur a pressé
//! Stop, ou si l'overlay ne peut pas s'afficher, rien ne s'exécute. Laya
//! tranche vite les questions fermées (quel élément, quelle fenêtre, action
//! risquée, objectif atteint) ; le modèle de conversation garde la conduite.

use crate::config::DesktopConfig;
use crate::input::{self, Button};
use crate::overlay_host::OverlayHost;
use crate::overlay_proto::PulseKind;
use crate::screen::{self, Region};
use crate::shell;
use crate::uia::{Item, Snapshot, Uia};
use crate::{sys, winmgr};
use locaryn_morph_kit::laya::{read_config, Candidate, Laya};
use locaryn_morph_kit::mcp::{str_prop, tool};
use locaryn_morph_kit::risk::assess;
use locaryn_morph_kit::text::{shortlist, text_arg};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

const SHORTLIST: usize = 40;
const SEUIL_CONFIANCE: f32 = 0.6;
const CAPTURE_LARGEUR: u32 = 1568;
/// Le temps que le compositeur retire l'overlay voilé de l'écran (deux images
/// à 60 Hz, plus une marge) avant la capture.
const VOILE_COMPOSITEUR: Duration = Duration::from_millis(80);
/// Pause (bouton Pause, ou Ctrl+Alt+Échap) : réversible, la tâche reprendra.
const PAUSED: &str = "L'utilisateur a mis le contrôle de l'ordinateur en pause (bouton « Pause », ou Ctrl+Alt+Échap). \
Ce n'est pas un arrêt définitif : ne cherchez pas à le contourner, attendez qu'il clique sur « Reprendre » pour continuer la même tâche.";

/// Arrêt (bouton Arrêt) : l'utilisateur ne veut plus que cette tâche continue.
const HARD_STOPPED: &str = "L'utilisateur a arrêté complètement le contrôle de l'ordinateur (bouton « Arrêt »), pas seulement mis en pause. \
Ne réessayez pas et ne cherchez pas à reprendre cette tâche : arrêtez-vous là, dites ce qui a été fait, et attendez de nouvelles instructions. \
Le contrôle reviendra à sa prochaine demande.";

/// Raccourcis qui ferment, déconnectent ou effacent.
const TOUCHES_A_RISQUE: &[&str] = &[
    "alt+f4",
    "ctrl+w",
    "ctrl+shift+w",
    "ctrl+shift+delete",
    "ctrl+alt+delete",
    "win+l",
];

/// Commandes qui détruisent, arrêtent ou changent la sécurité de la machine.
const COMMANDES_A_RISQUE: &[&str] = &[
    "remove-item",
    "rm ",
    "del ",
    "rmdir",
    "rd ",
    "format-volume",
    "format c:",
    "format d:",
    "clear-disk",
    "diskpart",
    "shutdown",
    "restart-computer",
    "stop-computer",
    "reg delete",
    "taskkill",
    "stop-process",
    "set-executionpolicy",
    "invoke-expression",
    "iex ",
    "net user",
    "icacls",
    "takeown",
];

/// Ce qu'on retient de la dernière lecture d'interface.
#[derive(Default)]
struct Memo {
    items: HashMap<u32, Item>,
}

pub struct Desktop {
    overlay: Arc<OverlayHost>,
    laya: Arc<Laya>,
    warmed: AtomicBool,
    cfg: DesktopConfig,
    uia: Uia,
    memo: Mutex<Memo>,
}

impl Default for Desktop {
    fn default() -> Self {
        Self::new()
    }
}

impl Desktop {
    pub fn new() -> Self {
        Self {
            overlay: OverlayHost::new(),
            laya: Arc::new(Laya::new()),
            warmed: AtomicBool::new(false),
            cfg: read_config(),
            uia: Uia::start(),
            memo: Mutex::new(Memo::default()),
        }
    }

    pub async fn call(&self, name: &str, args: Value) -> Result<Value, String> {
        if name == "desktop_status" {
            return self.status().await;
        }
        if name == "desktop_task_done" {
            // Un nettoyage, pas une nouvelle action : on ne le bloque pas
            // derrière un arrêt d'urgence, et il ne rouvre jamais l'overlay.
            self.overlay.finish().await;
            // Seul l'hôte, en fin de réponse, lève un Arrêt : le modèle arrêté
            // ne doit pas pouvoir se rendre la main en se déclarant fini.
            if args.get("end_of_response").and_then(Value::as_bool) == Some(true) {
                self.overlay.lift_hard_stop();
            }
            return Ok(json!({ "done": true }));
        }
        if self.overlay.is_stopped() {
            if !self.overlay.is_hard_stopped() {
                return Err(PAUSED.into());
            }
            if self.overlay.hard_stop_holds() {
                return Err(HARD_STOPPED.into());
            }
        }
        self.warm_up();
        self.begin(name).await?;
        self.dispatch(name, args).await
    }

    /// Montre l'overlay. Sans lui, pas d'action : piloter l'ordinateur à
    /// l'insu de l'utilisateur est précisément ce qu'on veut éviter.
    async fn begin(&self, name: &str) -> Result<(), String> {
        match self.overlay.active(label_for(name)).await {
            Ok(()) => Ok(()),
            Err(e) if self.cfg.require_overlay => Err(format!(
                "Overlay indisponible, action refusée par prudence : {e}"
            )),
            Err(e) => {
                eprintln!("overlay indisponible (require_overlay=false) : {e}");
                Ok(())
            }
        }
    }

    fn warm_up(&self) {
        if self.warmed.swap(true, Ordering::Relaxed) {
            return;
        }
        let laya = self.laya.clone();
        tokio::spawn(async move {
            if let Err(e) = laya.warm().await {
                eprintln!("préchauffage de Laya : {e}");
            }
        });
    }

    async fn dispatch(&self, name: &str, args: Value) -> Result<Value, String> {
        match name {
            "desktop_screen_info" => self.screen_info(),
            "desktop_screenshot" => self.screenshot(&args).await,
            "desktop_list_windows" => Ok(list_windows(&args)),
            "desktop_find_window" => self.find_window(&args).await,
            "desktop_focus_window" => focus_window(&args),
            "desktop_window_action" => self.window_action(&args).await,
            "desktop_launch" => launch(&args),
            "desktop_ui_tree" => self.ui_tree(&args).await,
            "desktop_find_element" => self.find_element(&args).await,
            "desktop_invoke" => self.invoke(&args).await,
            "desktop_set_value" => self.set_value(&args).await,
            "desktop_click" => self.click(&args).await,
            "desktop_move_mouse" => self.move_mouse(&args).await,
            "desktop_drag" => self.drag(&args).await,
            "desktop_scroll" => self.scroll(&args).await,
            "desktop_type_text" => type_text(&args).await,
            "desktop_press_key" => self.press_key(&args).await,
            "desktop_clipboard" => clipboard(&args).await,
            "desktop_run_command" => self.run_command(&args).await,
            "desktop_wait" => wait(&args).await,
            "desktop_goal_reached" => self.goal_reached(&args).await,
            "desktop_assess_action" => self.assess_action(&args).await,
            autre => Err(format!("Outil ordinateur inconnu : {autre}")),
        }
    }

    // ── État et écran ───────────────────────────────────────────────────────

    async fn status(&self) -> Result<Value, String> {
        let stopped = self.overlay.is_stopped();
        Ok(json!({
            "stopped_by_user": stopped,
            "stop_kind": stopped.then(|| if self.overlay.is_hard_stopped() { "hard" } else { "paused" }),
            "controls": "Pause et Arrêt à l'écran ; Ctrl+Alt+Échap vaut Pause",
            "resume": "une pause se lève par l'utilisateur (bouton sur l'overlay) ; un arrêt vaut jusqu'à la fin de la réponse",
            "laya": {
                "loaded": self.laya.is_running().await,
                "last_error": self.laya.last_error().await,
            },
            "settings": {
                "require_overlay": self.cfg.require_overlay,
                "confirm_risky": self.cfg.confirm_risky,
                "allow_run_command": self.cfg.allow_run_command,
            },
        }))
    }

    fn screen_info(&self) -> Result<Value, String> {
        let displays: Vec<Value> = screen::displays()?
            .iter()
            .map(screen::Display::describe)
            .collect();
        let (x, y) = sys::cursor_pos();
        let foreground = winmgr::list()
            .into_iter()
            .find(|w| w.foreground)
            .map(|w| w.to_json());
        Ok(json!({ "displays": displays, "cursor": [x, y], "foreground_window": foreground }))
    }

    async fn screenshot(&self, args: &Value) -> Result<Value, String> {
        let display = args["monitor"].as_u64().map(|m| m as usize);
        let region = region_of(args);
        let width = args["max_width"]
            .as_u64()
            .map_or(CAPTURE_LARGEUR, |w| w.clamp(200, 4000) as u32);
        let dir = media_dir();
        // L'overlay n'est plus exclu des captures (il en devenait opaque) : on
        // le voile le temps d'une ou deux images du compositeur, puis il revient.
        self.overlay.veil(true).await;
        tokio::time::sleep(VOILE_COMPOSITEUR).await;
        let shot =
            tokio::task::spawn_blocking(move || screen::capture(display, region, width, &dir))
                .await
                .map_err(|e| format!("capture interrompue : {e}"));
        self.overlay.veil(false).await;
        let shot = shot??;
        Ok(json!({
            "path": shot.path, "width": shot.width, "height": shot.height, "display": shot.display,
            "image_scale": shot.image_scale, "origin": [shot.origin.0, shot.origin.1],
            "note": "Coordonnées écran = origin + (pixel de l'image × image_scale).",
        }))
    }

    // ── Fenêtres ────────────────────────────────────────────────────────────

    async fn find_window(&self, args: &Value) -> Result<Value, String> {
        let wanted = text_arg(args, "description")?;
        let windows = winmgr::list();
        if windows.is_empty() {
            return Err("Aucune fenêtre ouverte.".into());
        }
        let candidates: Vec<Candidate> = windows
            .iter()
            .map(|w| Candidate {
                id: format!("w{}", w.hwnd),
                description: format!("{} — {}", w.title, w.exe),
            })
            .collect();
        let question = format!("Quelle fenêtre correspond à : {wanted} ?");
        let r = self
            .laya
            .rank(&json!({ "demande": wanted }), &question, &candidates)
            .await;
        let find = |id: &str| {
            windows
                .iter()
                .find(|w| format!("w{}", w.hwnd) == id)
                .map(|w| w.to_json())
        };
        Ok(json!({
            "best": r.ranking.first().and_then(|(id, _)| find(id)),
            "confidence": r.ranking.first().map(|b| b.1),
            "alternatives": r.ranking.iter().skip(1).take(3).filter_map(|(id, p)| find(id).map(|w| json!({"window": w, "score": p}))).collect::<Vec<_>>(),
            "engine": r.engine,
            "warning": r.warning,
        }))
    }

    async fn window_action(&self, args: &Value) -> Result<Value, String> {
        let hwnd = int_arg(args, "hwnd")?;
        let action = text_arg(args, "action")?;
        if action == "close" {
            let title = winmgr::list()
                .into_iter()
                .find(|w| w.hwnd == hwnd)
                .map(|w| w.title)
                .unwrap_or_default();
            if let Some(stop) = self
                .confirm_gate(args, &format!("fermer la fenêtre « {title} »"), &[])
                .await
            {
                return Ok(stop);
            }
        }
        winmgr::act(hwnd, &action)?;
        Ok(json!({ "done": action, "hwnd": hwnd }))
    }

    // ── Lecture de l'interface ──────────────────────────────────────────────

    async fn read_ui(&self, hwnd: Option<i64>, max: usize) -> Result<Snapshot, String> {
        let snap = self.uia.snapshot(hwnd, max).await?;
        let mut memo = self.memo.lock().await;
        memo.items = snap.items.iter().map(|i| (i.id, i.clone())).collect();
        Ok(snap)
    }

    async fn ui_tree(&self, args: &Value) -> Result<Value, String> {
        let max = args["max_elements"].as_u64().map_or(150, |m| m as usize);
        let snap = self.read_ui(args["hwnd"].as_i64(), max).await?;
        Ok(json!({
            "window": snap.window,
            "elements": snap.items.iter().map(Item::to_json).collect::<Vec<_>>(),
            "text": snap.text,
            "truncated": snap.truncated,
            "note": "Les numéros d'élément ne valent que jusqu'à la prochaine lecture.",
        }))
    }

    async fn find_element(&self, args: &Value) -> Result<Value, String> {
        let goal = text_arg(args, "goal")?;
        let snap = self.read_ui(args["hwnd"].as_i64(), 400).await?;
        let picks = shortlist(
            &goal,
            &snap.items,
            SHORTLIST,
            |i| i.name.clone(),
            |i| usize::from(i.enabled),
        );
        if picks.is_empty() {
            return Err("Aucun élément interactif lisible dans cette fenêtre (application sans accessibilité : utilisez desktop_screenshot).".into());
        }
        let candidates: Vec<Candidate> = picks.iter().map(candidate_of).collect();
        let r = self
            .laya
            .rank(
                &json!({ "objectif": goal }),
                "Quel élément permet d'atteindre l'objectif ?",
                &candidates,
            )
            .await;
        let find = |id: &str| {
            picks
                .iter()
                .find(|i| format!("e{}", i.id) == id)
                .map(Item::to_json)
        };
        let best = r.ranking.first();
        Ok(json!({
            "window": snap.window,
            "best": best.and_then(|(id, p)| find(id).map(|e| json!({"element": e, "confidence": p, "confident": *p >= SEUIL_CONFIANCE}))),
            "alternatives": r.ranking.iter().skip(1).take(3).filter_map(|(id, p)| find(id).map(|e| json!({"element": e, "score": p}))).collect::<Vec<_>>(),
            "engine": r.engine,
            "warning": r.warning,
        }))
    }

    async fn item(&self, args: &Value) -> Result<Item, String> {
        let id = int_arg(args, "element_id")? as u32;
        self.memo
            .lock()
            .await
            .items
            .get(&id)
            .cloned()
            .ok_or_else(|| format!("Élément {id} inconnu : refaites desktop_ui_tree."))
    }

    // ── Actions sur les éléments ────────────────────────────────────────────

    async fn invoke(&self, args: &Value) -> Result<Value, String> {
        let item = self.item(args).await?;
        if let Some(stop) = self
            .confirm_gate(args, &describe_item(&item, "activer"), &[])
            .await
        {
            return Ok(stop);
        }
        let (x, y) = item.center();
        self.overlay.pulse(PulseKind::Left, x, y).await;
        self.uia.invoke(item.id).await?;
        Ok(json!({ "invoked": item.name }))
    }

    async fn set_value(&self, args: &Value) -> Result<Value, String> {
        let item = self.item(args).await?;
        let text = args["text"]
            .as_str()
            .ok_or("Paramètre `text` requis.")?
            .to_string();
        self.uia.set_value(item.id, text.clone()).await?;
        Ok(json!({ "set": item.name, "chars": text.chars().count() }))
    }

    async fn click(&self, args: &Value) -> Result<Value, String> {
        let (x, y, name) = if args.get("element_id").is_some() {
            let item = self.item(args).await?;
            if let Some(stop) = self
                .confirm_gate(args, &describe_item(&item, "cliquer sur"), &[])
                .await
            {
                return Ok(stop);
            }
            let (cx, cy) = item.center();
            (cx, cy, Some(item.name))
        } else {
            (int_arg(args, "x")? as i32, int_arg(args, "y")? as i32, None)
        };
        let button = Button::parse(args["button"].as_str())?;
        let count = if args["double"] == json!(true) { 2 } else { 1 };
        blocking(move || input::move_to(x, y)).await?;
        self.overlay.pulse(pulse_kind(button, count), x, y).await;
        blocking(move || input::click(button, count)).await?;
        Ok(json!({ "clicked_at": [x, y], "element": name }))
    }

    async fn move_mouse(&self, args: &Value) -> Result<Value, String> {
        let (x, y) = (int_arg(args, "x")? as i32, int_arg(args, "y")? as i32);
        blocking(move || input::move_to(x, y)).await?;
        Ok(json!({ "cursor": [x, y] }))
    }

    async fn drag(&self, args: &Value) -> Result<Value, String> {
        let from = (
            int_arg(args, "from_x")? as i32,
            int_arg(args, "from_y")? as i32,
        );
        let to = (int_arg(args, "to_x")? as i32, int_arg(args, "to_y")? as i32);
        self.overlay.pulse(PulseKind::Left, from.0, from.1).await;
        blocking(move || input::drag(from, to)).await?;
        Ok(json!({ "dragged": { "from": [from.0, from.1], "to": [to.0, to.1] } }))
    }

    async fn scroll(&self, args: &Value) -> Result<Value, String> {
        let (dy, dx) = (
            args["dy"].as_i64().unwrap_or(0) as i32,
            args["dx"].as_i64().unwrap_or(0) as i32,
        );
        if let (Some(x), Some(y)) = (args["x"].as_i64(), args["y"].as_i64()) {
            blocking(move || input::move_to(x as i32, y as i32)).await?;
        }
        blocking(move || input::scroll(dy, dx)).await?;
        Ok(json!({ "scrolled": { "dy": dy, "dx": dx } }))
    }

    async fn press_key(&self, args: &Value) -> Result<Value, String> {
        let keys = text_arg(args, "keys")?;
        if let Some(stop) = self
            .confirm_gate(args, &format!("appuyer sur {keys}"), TOUCHES_A_RISQUE)
            .await
        {
            return Ok(stop);
        }
        let combo = keys.clone();
        blocking(move || input::press_combo(&combo)).await?;
        Ok(json!({ "pressed": keys }))
    }

    async fn run_command(&self, args: &Value) -> Result<Value, String> {
        if !self.cfg.allow_run_command {
            return Err(
                "L'exécution de commandes est désactivée dans les réglages du morph.".into(),
            );
        }
        let command = text_arg(args, "command")?;
        if let Some(stop) = self.confirm_gate(args, &command, COMMANDES_A_RISQUE).await {
            return Ok(stop);
        }
        let asked = args["timeout_s"]
            .as_u64()
            .unwrap_or(self.cfg.max_command_seconds);
        let timeout = Duration::from_secs(asked.min(self.cfg.max_command_seconds));
        shell::run(&command, args["cwd"].as_str(), timeout).await
    }

    // ── Jugements ───────────────────────────────────────────────────────────

    /// Faut-il arrêter l'action pour demander confirmation ? Rend la réponse à
    /// donner au modèle si oui.
    ///
    /// Le repérage par mots (`extra`, les libellés dangereux) juge toujours.
    /// Laya s'y ajoute quand il est prêt ; tant qu'il se charge, seule une
    /// commande shell l'attend. Sans GPU libre — un modèle de chat l'occupe —
    /// son chargement prend des minutes, et chaque touche ou clic restait
    /// bloqué jusqu'à cinq minutes derrière lui.
    async fn confirm_gate(&self, args: &Value, action: &str, extra: &[&str]) -> Option<Value> {
        if !self.cfg.confirm_risky || args["confirmed"] == json!(true) {
            return None;
        }
        let shell = std::ptr::eq(extra, COMMANDES_A_RISQUE);
        let use_laya = self.cfg.laya_risk_check && (shell || self.laya.is_ready_now());
        let risk = assess(&self.laya, action, extra, use_laya).await;
        risk.risky().then(|| {
            json!({
                "needs_confirmation": true,
                "action": action,
                "risk": { "lexical": risk.lexical, "laya": risk.laya },
                "next": "Action non exécutée. Décrivez-la à l'utilisateur et demandez son accord ; \
                         s'il accepte, rappelez l'outil avec confirmed=true."
            })
        })
    }

    async fn goal_reached(&self, args: &Value) -> Result<Value, String> {
        let goal = text_arg(args, "goal")?;
        let snap = self.read_ui(args["hwnd"].as_i64(), 60).await?;
        let noms: Vec<&str> = snap
            .items
            .iter()
            .map(|i| i.name.as_str())
            .filter(|n| !n.is_empty())
            .collect();
        let state =
            json!({ "fenetre": snap.window, "texte": snap.text, "elements": noms.join(" | ") });
        let p = self
            .laya
            .judge(
                &state,
                &format!("L'objectif « {goal} » est-il atteint dans cette fenêtre ?"),
            )
            .await
            .map_err(|e| format!("Laya indisponible, impossible d'estimer : {e}"))?;
        Ok(json!({
            "probability": p, "reached": p >= 0.5,
            "note": "Estimation sur les textes de la fenêtre ; vérifiez par une capture si l'enjeu est important.",
        }))
    }

    async fn assess_action(&self, args: &Value) -> Result<Value, String> {
        let action = text_arg(args, "action")?;
        let mut extra: Vec<&str> = TOUCHES_A_RISQUE.to_vec();
        extra.extend_from_slice(COMMANDES_A_RISQUE);
        let risk = assess(&self.laya, &action, &extra, true).await;
        Ok(json!({
            "risky": risk.risky(), "lexical": risk.lexical, "laya_probability": risk.laya,
            "laya_error": if risk.laya.is_none() { self.laya.last_error().await } else { None },
        }))
    }
}

// ── Fonctions sans état ─────────────────────────────────────────────────────

fn list_windows(args: &Value) -> Value {
    let q = args["query"].as_str().unwrap_or("").to_lowercase();
    let windows: Vec<Value> = winmgr::list()
        .into_iter()
        .filter(|w| {
            q.is_empty() || w.title.to_lowercase().contains(&q) || w.exe.to_lowercase().contains(&q)
        })
        .map(|w| w.to_json())
        .collect();
    json!({ "windows": windows })
}

fn focus_window(args: &Value) -> Result<Value, String> {
    let hwnd = int_arg(args, "hwnd")?;
    winmgr::focus(hwnd)?;
    Ok(json!({ "focused": hwnd }))
}

fn launch(args: &Value) -> Result<Value, String> {
    shell::launch(&text_arg(args, "target")?, args["args"].as_str())
}

async fn type_text(args: &Value) -> Result<Value, String> {
    let text = args["text"]
        .as_str()
        .ok_or("Paramètre `text` requis.")?
        .to_string();
    let chars = text.chars().count();
    let submit = args["submit"] == json!(true);
    blocking(move || {
        input::type_text(&text)?;
        if submit {
            input::press_combo("enter")?;
        }
        Ok(())
    })
    .await?;
    Ok(json!({ "typed": chars, "submitted": submit }))
}

async fn clipboard(args: &Value) -> Result<Value, String> {
    let action = text_arg(args, "action")?;
    let text = args["text"].as_str().map(str::to_string);
    blocking_value(move || {
        let mut cb = arboard::Clipboard::new().map_err(|e| format!("presse-papiers : {e}"))?;
        match action.as_str() {
            "get" => {
                let t = cb
                    .get_text()
                    .map_err(|e| format!("presse-papiers vide ou non textuel : {e}"))?;
                Ok(json!({ "text": t.chars().take(8000).collect::<String>() }))
            }
            "set" => {
                let t = text.ok_or("Paramètre `text` requis pour set.")?;
                cb.set_text(t.clone())
                    .map_err(|e| format!("écriture du presse-papiers : {e}"))?;
                Ok(json!({ "set_chars": t.chars().count() }))
            }
            autre => Err(format!("Action inconnue : {autre} (get, set)")),
        }
    })
    .await
}

async fn wait(args: &Value) -> Result<Value, String> {
    let ms = args["ms"].as_u64().unwrap_or(1000).min(10_000);
    tokio::time::sleep(Duration::from_millis(ms)).await;
    Ok(json!({ "waited_ms": ms }))
}

async fn blocking(f: impl FnOnce() -> Result<(), String> + Send + 'static) -> Result<(), String> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| format!("action interrompue : {e}"))?
}

async fn blocking_value(
    f: impl FnOnce() -> Result<Value, String> + Send + 'static,
) -> Result<Value, String> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| format!("action interrompue : {e}"))?
}

fn int_arg(args: &Value, key: &str) -> Result<i64, String> {
    args[key]
        .as_i64()
        .or_else(|| args[key].as_f64().map(|f| f.round() as i64))
        .ok_or_else(|| format!("Paramètre `{key}` requis (nombre)."))
}

fn region_of(args: &Value) -> Option<Region> {
    let r = args.get("region")?;
    Some(Region {
        x: r["x"].as_i64()? as i32,
        y: r["y"].as_i64()? as i32,
        width: r["width"].as_u64()? as u32,
        height: r["height"].as_u64()? as u32,
    })
}

fn pulse_kind(button: Button, count: u32) -> PulseKind {
    match (button, count) {
        (Button::Right, _) => PulseKind::Right,
        (_, 2) => PulseKind::Double,
        _ => PulseKind::Left,
    }
}

fn describe_item(item: &Item, verb: &str) -> String {
    format!("{verb} {} « {} »", item.kind, item.name)
}

/// Le nom seul départage mieux (mesuré côté navigateur) ; sans nom, le type
/// et l'identifiant d'automatisation prennent le relais.
fn candidate_of(item: &Item) -> Candidate {
    let description = if item.name.trim().is_empty() {
        format!("{} {}", item.kind, item.automation_id)
            .trim()
            .to_string()
    } else {
        item.name.clone()
    };
    Candidate {
        id: format!("e{}", item.id),
        description,
    }
}

fn media_dir() -> PathBuf {
    std::env::var("LOCARYN_EXTENSION_MEDIA_DIR")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("locaryn-desktop"))
}

fn label_for(tool: &str) -> &'static str {
    match tool {
        "desktop_screenshot" | "desktop_screen_info" => "Lecture de l'écran",
        "desktop_ui_tree" | "desktop_find_element" | "desktop_goal_reached" => {
            "Lecture de la fenêtre"
        }
        "desktop_list_windows"
        | "desktop_find_window"
        | "desktop_focus_window"
        | "desktop_window_action" => "Fenêtres",
        "desktop_launch" => "Ouverture d'une application",
        "desktop_click" | "desktop_invoke" => "Clic",
        "desktop_move_mouse" | "desktop_drag" | "desktop_scroll" => "Souris",
        "desktop_type_text" | "desktop_set_value" | "desktop_press_key" => "Clavier",
        "desktop_clipboard" => "Presse-papiers",
        "desktop_run_command" => "Commande",
        _ => "Action",
    }
}

// ── Déclaration des outils ──────────────────────────────────────────────────

fn hwnd_prop() -> Value {
    json!({ "type": "integer", "description": "Identifiant de fenêtre (desktop_list_windows). Omis : la fenêtre au premier plan." })
}

fn confirmed_prop() -> Value {
    json!({ "type": "boolean", "description": "true seulement après accord explicite de l'utilisateur" })
}

fn xy_props() -> Value {
    json!({ "x": { "type": "integer", "description": "Abscisse écran (pixels physiques)" }, "y": { "type": "integer", "description": "Ordonnée écran" } })
}

fn screen_tools() -> Vec<Value> {
    vec![
        tool("desktop_status", "État du contrôle de l'ordinateur : arrêt d'urgence pressé ou non, Laya, réglages. À appeler si un outil répond que l'utilisateur a pressé Stop.", json!({}), &[]),
        tool("desktop_task_done", "Signale que la tâche de contrôle de l'ordinateur est terminée : l'overlay (cadre, curseur) disparaît de l'écran tout de suite. L'overlay reste sinon affiché sans interruption tant que d'autres outils desktop_* sont utilisés, même avec un temps de réflexion entre deux appels — à appeler dès que vous avez fini, pas après chaque action.", json!({}), &[]),
        tool("desktop_screen_info", "Écrans (position, taille, échelle), position du curseur et fenêtre au premier plan.", json!({}), &[]),
        tool("desktop_screenshot", "Capture un écran ou une zone dans un fichier PNG. Rend le chemin et l'échelle pour convertir un pixel de l'image en coordonnée écran.", json!({ "monitor": { "type": "integer" }, "region": { "type": "object", "properties": { "x": {"type": "integer"}, "y": {"type": "integer"}, "width": {"type": "integer"}, "height": {"type": "integer"} } }, "max_width": { "type": "integer" } }), &[]),
        tool("desktop_list_windows", "Liste les fenêtres ouvertes (titre, application, position). `query` filtre.", json!({ "query": str_prop("Filtre sur le titre ou l'application") }), &[]),
        tool("desktop_find_window", "Retrouve la fenêtre qui correspond à une description. Décidé par Laya.", json!({ "description": str_prop("Ce que la fenêtre montre") }), &["description"]),
        tool("desktop_focus_window", "Met une fenêtre au premier plan (la restaure si réduite).", json!({ "hwnd": hwnd_prop() }), &["hwnd"]),
        tool("desktop_window_action", "Réduit, agrandit, restaure ou ferme une fenêtre. Fermer demande confirmation.", json!({ "hwnd": hwnd_prop(), "action": { "type": "string", "enum": ["minimize", "maximize", "restore", "close"] }, "confirmed": confirmed_prop() }), &["hwnd", "action"]),
        tool("desktop_launch", "Ouvre une application, un document, un dossier ou une adresse (`notepad`, `calc`, `C:\\\\docs\\\\a.pdf`, `https://…`).", json!({ "target": str_prop("Nom, chemin ou adresse"), "args": str_prop("Arguments éventuels") }), &["target"]),
    ]
}

fn ui_tools() -> Vec<Value> {
    vec![
        tool("desktop_ui_tree", "Lit une fenêtre par l'accessibilité : éléments interactifs numérotés (id, type, nom, position) et textes. Bien plus fiable et léger qu'une capture quand l'application l'expose.", json!({ "hwnd": hwnd_prop(), "max_elements": { "type": "integer" } }), &[]),
        tool("desktop_find_element", "Trouve l'élément de la fenêtre qui sert un objectif (« enregistrer », « barre de recherche »). Laya départage ; rend le meilleur, sa confiance, des alternatives.", json!({ "hwnd": hwnd_prop(), "goal": str_prop("Ce qu'on veut faire") }), &["goal"]),
        tool("desktop_invoke", "Active un élément par accessibilité (sans déplacer la souris). Actions irréversibles : confirmation demandée.", json!({ "element_id": { "type": "integer" }, "confirmed": confirmed_prop() }), &["element_id"]),
        tool("desktop_set_value", "Remplace la valeur d'un champ par accessibilité.", json!({ "element_id": { "type": "integer" }, "text": str_prop("Nouvelle valeur") }), &["element_id", "text"]),
        tool("desktop_goal_reached", "Estime, par Laya, si l'objectif est atteint d'après les textes de la fenêtre. Une probabilité, pas une preuve.", json!({ "hwnd": hwnd_prop(), "goal": str_prop("L'objectif à vérifier") }), &["goal"]),
        tool("desktop_assess_action", "Juge, avant d'agir, si une action (clic, raccourci, commande) est irréversible ou lourde de conséquences.", json!({ "action": str_prop("L'action envisagée") }), &["action"]),
    ]
}

fn input_tools() -> Vec<Value> {
    let mut click_props = xy_props();
    click_props["element_id"] = json!({ "type": "integer", "description": "Élément de desktop_ui_tree, à la place de x/y" });
    click_props["button"] = json!({ "type": "string", "enum": ["left", "right", "middle"] });
    click_props["double"] = json!({ "type": "boolean" });
    click_props["confirmed"] = confirmed_prop();
    vec![
        tool("desktop_click", "Clique à des coordonnées écran ou sur un élément. Le curseur se déplace visiblement, avec une onde à l'écran. Clic sur un élément risqué : confirmation demandée.", click_props, &[]),
        tool("desktop_move_mouse", "Déplace le curseur.", xy_props(), &["x", "y"]),
        tool("desktop_drag", "Glisser-déposer d'un point à un autre.", json!({ "from_x": {"type": "integer"}, "from_y": {"type": "integer"}, "to_x": {"type": "integer"}, "to_y": {"type": "integer"} }), &["from_x", "from_y", "to_x", "to_y"]),
        tool("desktop_scroll", "Fait défiler (crans de molette : dy > 0 vers le bas, dx > 0 vers la droite), éventuellement après avoir placé le curseur en x, y.", json!({ "x": {"type": "integer"}, "y": {"type": "integer"}, "dy": {"type": "integer"}, "dx": {"type": "integer"} }), &[]),
        tool("desktop_type_text", "Saisit du texte dans l'élément qui a le focus (Unicode). submit=true termine par Entrée.", json!({ "text": str_prop("Texte à saisir"), "submit": {"type": "boolean"} }), &["text"]),
        tool("desktop_press_key", "Appuie sur une touche ou un raccourci : `enter`, `ctrl+s`, `alt+tab`, `win+d`. Les raccourcis qui ferment ou déconnectent demandent confirmation.", json!({ "keys": str_prop("Touches séparées par +"), "confirmed": confirmed_prop() }), &["keys"]),
        tool("desktop_clipboard", "Lit (get) ou écrit (set) le texte du presse-papiers.", json!({ "action": {"type": "string", "enum": ["get", "set"]}, "text": str_prop("Texte à copier (set)") }), &["action"]),
        tool("desktop_run_command", "Exécute une commande PowerShell et rend sa sortie. Commande destructrice ou touchant la sécurité : confirmation demandée. Peut être désactivée dans les réglages.", json!({ "command": str_prop("Commande PowerShell"), "cwd": str_prop("Dossier de travail"), "timeout_s": {"type": "integer"}, "confirmed": confirmed_prop() }), &["command"]),
        tool("desktop_wait", "Attend (10 s au plus) que l'interface réagisse.", json!({ "ms": {"type": "integer"} }), &[]),
    ]
}

pub fn tools_list() -> Value {
    let mut all = screen_tools();
    all.extend(ui_tools());
    all.extend(input_tools());
    json!({ "tools": all })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chaque_outil_declare_a_une_route() {
        let d = Desktop::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        for t in tools_list()["tools"].as_array().unwrap() {
            let n = t["name"].as_str().unwrap();
            if n == "desktop_status" || n == "desktop_task_done" {
                continue; // gérés dans `call`, avant `dispatch`
            }
            // Un outil inconnu répond « inconnu » ; un outil routé répond autre chose.
            let r = rt.block_on(d.dispatch(n, json!({})));
            let inconnu = matches!(&r, Err(e) if e.starts_with("Outil ordinateur inconnu"));
            assert!(!inconnu, "{n} sans route");
        }
    }

    #[test]
    fn les_coordonnees_manquantes_sont_refusees() {
        assert!(int_arg(&json!({}), "x").is_err());
        assert_eq!(int_arg(&json!({"x": 10.6}), "x").unwrap(), 11);
    }

    #[test]
    fn un_element_sans_nom_se_decrit_par_son_type() {
        let item = Item {
            id: 3,
            kind: "edit".into(),
            name: " ".into(),
            automation_id: "SearchBox".into(),
            rect: [0, 0, 10, 10],
            enabled: true,
            value: None,
        };
        assert_eq!(candidate_of(&item).description, "edit SearchBox");
    }
}
