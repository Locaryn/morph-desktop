//! Contrôle de l'ordinateur : capture, souris, clavier, fenêtres, commandes.
//! Windows uniquement pour l'instant.
//!
//! Toute action passe par l'overlay plein écran : tant que l'ordinateur est
//! piloté, l'utilisateur le voit, et il peut couper à tout moment.
#![cfg(windows)]

pub mod config;
pub mod input;
pub mod overlay_host;
pub mod overlay_proto;
pub mod screen;
pub mod shell;
pub mod sys;
pub mod tools;
pub mod uia;
pub mod winmgr;

/// Horodatage en millisecondes, pour nommer les fichiers produits.
pub fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}
