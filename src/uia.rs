//! Arbre d'accessibilité (UI Automation) : lire une fenêtre comme une liste
//! d'éléments nommés, plutôt que de deviner des pixels.
//!
//! Les objets COM ne sont pas `Send` : un fil dédié les possède et répond aux
//! demandes par un canal.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tokio::sync::oneshot;
use uiautomation::patterns::{UIInvokePattern, UIValuePattern};
use uiautomation::types::{ControlType, Handle};
use uiautomation::{UIAutomation, UIElement, UITreeWalker};

const MAX_DEPTH: usize = 14;
const MAX_NODES: usize = 1500;
const TIME_BUDGET: Duration = Duration::from_millis(3500);
const TEXT_MAX: usize = 2500;
const VALUE_MAX: usize = 300;

#[derive(Debug, Clone)]
pub struct Item {
    pub id: u32,
    pub kind: String,
    pub name: String,
    pub automation_id: String,
    pub rect: [i32; 4],
    pub enabled: bool,
    /// Contenu d'un champ de saisie, quand l'application l'expose.
    pub value: Option<String>,
}

impl Item {
    pub fn center(&self) -> (i32, i32) {
        (
            self.rect[0] + self.rect[2] / 2,
            self.rect[1] + self.rect[3] / 2,
        )
    }

    pub fn to_json(&self) -> Value {
        let (cx, cy) = self.center();
        json!({
            "id": self.id, "type": self.kind, "name": self.name, "automation_id": self.automation_id,
            "rect": self.rect, "center": [cx, cy], "enabled": self.enabled, "value": self.value,
        })
    }
}

pub struct Snapshot {
    pub window: String,
    pub items: Vec<Item>,
    pub text: String,
    pub truncated: bool,
}

type Reply<T> = oneshot::Sender<Result<T, String>>;

enum Req {
    Snapshot {
        hwnd: Option<i64>,
        max: usize,
        reply: Reply<Snapshot>,
    },
    Invoke {
        id: u32,
        reply: Reply<()>,
    },
    SetValue {
        id: u32,
        text: String,
        reply: Reply<()>,
    },
}

#[derive(Clone)]
pub struct Uia {
    tx: mpsc::Sender<Req>,
}

impl Uia {
    pub fn start() -> Self {
        let (tx, rx) = mpsc::channel::<Req>();
        std::thread::spawn(move || worker(rx));
        Self { tx }
    }

    pub async fn snapshot(&self, hwnd: Option<i64>, max: usize) -> Result<Snapshot, String> {
        self.ask(|reply| Req::Snapshot { hwnd, max, reply }).await
    }

    pub async fn invoke(&self, id: u32) -> Result<(), String> {
        self.ask(|reply| Req::Invoke { id, reply }).await
    }

    pub async fn set_value(&self, id: u32, text: String) -> Result<(), String> {
        self.ask(|reply| Req::SetValue { id, text, reply }).await
    }

    async fn ask<T>(&self, build: impl FnOnce(Reply<T>) -> Req) -> Result<T, String> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(build(reply))
            .map_err(|_| "Le lecteur d'interface s'est arrêté".to_string())?;
        rx.await
            .map_err(|_| "Le lecteur d'interface n'a pas répondu".to_string())?
    }
}

fn worker(rx: mpsc::Receiver<Req>) {
    let automation = match UIAutomation::new() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("UI Automation indisponible : {e}");
            for req in rx {
                fail(req, format!("UI Automation indisponible : {e}"));
            }
            return;
        }
    };
    let mut cache: HashMap<u32, UIElement> = HashMap::new();
    for req in rx {
        match req {
            Req::Snapshot { hwnd, max, reply } => {
                cache.clear();
                send(reply, snapshot(&automation, hwnd, max, &mut cache));
            }
            Req::Invoke { id, reply } => send(reply, invoke(&cache, id)),
            Req::SetValue { id, text, reply } => send(reply, set_value(&cache, id, &text)),
        }
    }
}

fn fail(req: Req, msg: String) {
    match req {
        Req::Snapshot { reply, .. } => send(reply, Err(msg)),
        Req::Invoke { reply, .. } | Req::SetValue { reply, .. } => send(reply, Err(msg)),
    }
}

fn send<T>(reply: Reply<T>, value: Result<T, String>) {
    if reply.send(value).is_err() {
        eprintln!("UI Automation : demandeur parti");
    }
}

fn element(cache: &HashMap<u32, UIElement>, id: u32) -> Result<&UIElement, String> {
    cache
        .get(&id)
        .ok_or_else(|| format!("Élément {id} inconnu : refaites desktop_ui_tree (les numéros changent à chaque lecture)"))
}

fn invoke(cache: &HashMap<u32, UIElement>, id: u32) -> Result<(), String> {
    let pattern = element(cache, id)?
        .get_pattern::<UIInvokePattern>()
        .map_err(|_| {
            format!("L'élément {id} ne s'active pas par accessibilité : utilisez desktop_click")
        })?;
    pattern
        .invoke()
        .map_err(|e| format!("Activation refusée : {e}"))
}

fn set_value(cache: &HashMap<u32, UIElement>, id: u32, text: &str) -> Result<(), String> {
    let pattern = element(cache, id)?
        .get_pattern::<UIValuePattern>()
        .map_err(|_| format!("L'élément {id} n'accepte pas de valeur directe : cliquez-le puis desktop_type_text"))?;
    pattern
        .set_value(text)
        .map_err(|e| format!("Saisie refusée : {e}"))
}

// ── Lecture de l'arbre ──────────────────────────────────────────────────────

struct Walk<'a> {
    walker: UITreeWalker,
    max_items: usize,
    started: Instant,
    nodes: usize,
    items: Vec<Item>,
    text: String,
    cache: &'a mut HashMap<u32, UIElement>,
}

fn snapshot(
    automation: &UIAutomation,
    hwnd: Option<i64>,
    max: usize,
    cache: &mut HashMap<u32, UIElement>,
) -> Result<Snapshot, String> {
    let hwnd = hwnd
        .or_else(crate::winmgr::foreground)
        .ok_or("Aucune fenêtre au premier plan")?;
    let root = automation
        .element_from_handle(Handle::from(hwnd as isize))
        .map_err(|e| format!("Fenêtre illisible : {e}"))?;
    let walker = automation
        .get_control_view_walker()
        .map_err(|e| format!("Parcours : {e}"))?;
    let mut walk = Walk {
        walker,
        max_items: max,
        started: Instant::now(),
        nodes: 0,
        items: Vec::new(),
        text: String::new(),
        cache,
    };
    walk.visit(&root, 0);
    let truncated =
        walk.nodes >= MAX_NODES || walk.started.elapsed() >= TIME_BUDGET || walk.items.len() >= max;
    Ok(Snapshot {
        window: root.get_name().unwrap_or_default(),
        items: walk.items,
        text: walk.text,
        truncated,
    })
}

impl Walk<'_> {
    fn exhausted(&self) -> bool {
        self.nodes >= MAX_NODES
            || self.items.len() >= self.max_items
            || self.started.elapsed() >= TIME_BUDGET
    }

    fn visit(&mut self, el: &UIElement, depth: usize) {
        if depth > MAX_DEPTH || self.exhausted() {
            return;
        }
        self.nodes += 1;
        if el.is_offscreen().unwrap_or(false) {
            return; // ce qui est hors écran (menus repliés, onglets cachés) ne se voit pas
        }
        self.record(el);
        let mut child = self.walker.get_first_child(el).ok();
        while let Some(c) = child {
            self.visit(&c, depth + 1);
            if self.exhausted() {
                return;
            }
            child = self.walker.get_next_sibling(&c).ok();
        }
    }

    fn record(&mut self, el: &UIElement) {
        let Ok(kind) = el.get_control_type() else {
            return;
        };
        let name = el.get_name().unwrap_or_default();
        if matches!(kind, ControlType::Text | ControlType::Document) && !name.trim().is_empty() {
            self.push_text(&name);
        }
        if !is_interactive(kind) {
            return;
        }
        let Ok(r) = el.get_bounding_rectangle() else {
            return;
        };
        let (w, h) = (r.get_right() - r.get_left(), r.get_bottom() - r.get_top());
        if w <= 0 || h <= 0 {
            return;
        }
        let id = self.items.len() as u32 + 1;
        self.cache.insert(id, el.clone());
        self.items.push(Item {
            id,
            kind: kind_name(kind),
            name,
            automation_id: el.get_automation_id().unwrap_or_default(),
            rect: [r.get_left(), r.get_top(), w, h],
            enabled: el.is_enabled().unwrap_or(true),
            value: field_value(el, kind),
        });
    }

    fn push_text(&mut self, s: &str) {
        if self.text.len() >= TEXT_MAX {
            return;
        }
        let line: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
        self.text.push_str(&line);
        self.text.push('\n');
    }
}

/// Le contenu d'un champ, pour vérifier ce qu'on y a saisi. Jamais celui d'un
/// champ de mot de passe : UI Automation ne le livre pas, et on ne le demande pas.
fn field_value(el: &UIElement, kind: ControlType) -> Option<String> {
    if !matches!(
        kind,
        ControlType::Edit | ControlType::ComboBox | ControlType::Document
    ) {
        return None;
    }
    if el.is_password().unwrap_or(false) {
        return None;
    }
    let value = el.get_pattern::<UIValuePattern>().ok()?.get_value().ok()?;
    Some(value.chars().take(VALUE_MAX).collect())
}

fn is_interactive(kind: ControlType) -> bool {
    matches!(
        kind,
        ControlType::Button
            | ControlType::CheckBox
            | ControlType::ComboBox
            | ControlType::Edit
            | ControlType::Hyperlink
            | ControlType::ListItem
            | ControlType::MenuItem
            | ControlType::RadioButton
            | ControlType::SplitButton
            | ControlType::TabItem
            | ControlType::TreeItem
            | ControlType::Slider
            | ControlType::Spinner
            | ControlType::Document
    )
}

fn kind_name(kind: ControlType) -> String {
    format!("{kind:?}").to_lowercase()
}
