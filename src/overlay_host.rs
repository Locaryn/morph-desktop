//! Côté serveur : lance l'overlay, lui parle, et retient l'arrêt d'urgence.
//!
//! L'arrêt reste en vigueur jusqu'à ce que **l'utilisateur** presse « reprendre »
//! sur l'overlay : aucun outil ne permet au modèle de le lever.

use crate::overlay_proto::{Command, Event, PulseKind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command as Proc};
use tokio::sync::{oneshot, Mutex};
use tokio::time::timeout;

/// Sans nouvelle action pendant ce délai, l'overlay s'efface.
const IDLE_AFTER: Duration = Duration::from_secs(5);
const READY_TIMEOUT: Duration = Duration::from_secs(20);

struct Live {
    child: Child,
    stdin: ChildStdin,
}

pub struct OverlayHost {
    live: Mutex<Option<Live>>,
    stopped: Arc<AtomicBool>,
    generation: AtomicU64,
}

impl OverlayHost {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            live: Mutex::new(None),
            stopped: Arc::new(AtomicBool::new(false)),
            generation: AtomicU64::new(0),
        })
    }

    /// L'utilisateur a-t-il coupé le contrôle ?
    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Relaxed)
    }

    /// Montre l'overlay avec l'action en cours et programme son effacement.
    pub async fn active(self: &Arc<Self>, label: &str) -> Result<(), String> {
        self.send(&Command::Active {
            label: label.into(),
        })
        .await?;
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let host = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(IDLE_AFTER).await;
            if host.generation.load(Ordering::SeqCst) == generation {
                if let Err(e) = host.send(&Command::Idle).await {
                    eprintln!("overlay : effacement impossible : {e}");
                }
            }
        });
        Ok(())
    }

    pub async fn pulse(&self, kind: PulseKind, x: i32, y: i32) {
        if let Err(e) = self.send(&Command::Pulse { kind, x, y }).await {
            eprintln!("overlay : onde non affichée : {e}");
        }
    }

    async fn send(&self, cmd: &Command) -> Result<(), String> {
        let mut garde = self.live.lock().await;
        if garde.is_none() {
            *garde = Some(self.spawn().await?);
        }
        let line = serde_json::to_string(cmd).map_err(|e| format!("commande : {e}"))? + "\n";
        let live = garde.as_mut().ok_or("overlay absent")?;
        let written = async {
            live.stdin.write_all(line.as_bytes()).await?;
            live.stdin.flush().await
        };
        if let Err(e) = written.await {
            *garde = None; // le suivant relance un overlay propre
            return Err(format!("L'overlay ne répond plus : {e}"));
        }
        Ok(())
    }

    async fn spawn(&self) -> Result<Live, String> {
        let exe = overlay_path()?;
        let mut child = Proc::new(&exe)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("Impossible de lancer l'overlay ({}) : {e}", exe.display()))?;
        let stdin = child
            .stdin
            .take()
            .ok_or("stdin de l'overlay indisponible")?;
        let stdout = child
            .stdout
            .take()
            .ok_or("stdout de l'overlay indisponible")?;
        let (ready_tx, ready_rx) = oneshot::channel();
        tokio::spawn(read_events(
            BufReader::new(stdout),
            self.stopped.clone(),
            ready_tx,
        ));
        timeout(READY_TIMEOUT, ready_rx)
            .await
            .map_err(|_| "L'overlay n'a pas démarré à temps".to_string())?
            .map_err(|_| "L'overlay s'est arrêté au démarrage (WebView2 installé ?)".to_string())?;
        let mut live = Live { child, stdin };
        if self.is_stopped() {
            // Un overlay relancé après un arrêt doit encore proposer de reprendre.
            let halt = serde_json::to_string(&Command::Halt).map_err(|e| e.to_string())? + "\n";
            live.stdin
                .write_all(halt.as_bytes())
                .await
                .map_err(|e| format!("overlay : {e}"))?;
        }
        Ok(live)
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        if let Err(e) = self.child.start_kill() {
            eprintln!("overlay : arrêt du processus : {e}");
        }
    }
}

async fn read_events(
    mut lines: BufReader<tokio::process::ChildStdout>,
    stopped: Arc<AtomicBool>,
    ready: oneshot::Sender<()>,
) {
    let mut ready = Some(ready);
    let mut buf = String::new();
    loop {
        buf.clear();
        match lines.read_line(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        match serde_json::from_str::<Event>(buf.trim()) {
            Ok(Event::Ready) => {
                if let Some(tx) = ready.take() {
                    if tx.send(()).is_err() {
                        eprintln!("overlay : démarrage abandonné");
                    }
                }
            }
            Ok(Event::Stop) => stopped.store(true, Ordering::SeqCst),
            Ok(Event::Resume) => stopped.store(false, Ordering::SeqCst),
            Err(e) => eprintln!("overlay : événement illisible : {e}"),
        }
    }
}

/// L'overlay est livré à côté du serveur.
fn overlay_path() -> Result<PathBuf, String> {
    let me = std::env::current_exe().map_err(|e| format!("chemin du serveur : {e}"))?;
    let exe = me.with_file_name("locaryn-desktop-overlay.exe");
    if exe.exists() {
        Ok(exe)
    } else {
        Err(format!("Overlay introuvable : {}", exe.display()))
    }
}
