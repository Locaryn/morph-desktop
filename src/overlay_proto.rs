//! Protocole entre le serveur MCP et le processus d'overlay : une ligne JSON
//! par message, commandes vers l'overlay, événements en retour.

use serde::{Deserialize, Serialize};

/// Ce que le serveur demande à l'overlay.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    /// Afficher le cadre animé avec l'action en cours.
    Active { label: String },
    /// Tout masquer.
    Idle,
    /// Reprendre l'état « arrêté » : le bouton propose de reprendre.
    Halt,
    /// Une onde de clic aux coordonnées écran (pixels physiques).
    Pulse { kind: PulseKind, x: i32, y: i32 },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PulseKind {
    Left,
    Right,
    Double,
}

/// Ce que l'overlay rapporte.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// L'overlay est affiché et prêt.
    Ready,
    /// L'utilisateur a pressé Stop (ou Ctrl+Alt+Échap).
    Stop,
    /// L'utilisateur a autorisé de nouveau le contrôle.
    Resume,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn une_commande_fait_l_aller_retour() {
        let c = Command::Pulse {
            kind: PulseKind::Right,
            x: 10,
            y: -4,
        };
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(json, r#"{"cmd":"pulse","kind":"right","x":10,"y":-4}"#);
        assert_eq!(serde_json::from_str::<Command>(&json).unwrap(), c);
    }

    #[test]
    fn un_evenement_se_lit() {
        assert_eq!(
            serde_json::from_str::<Event>(r#"{"event":"stop"}"#).unwrap(),
            Event::Stop
        );
    }
}
