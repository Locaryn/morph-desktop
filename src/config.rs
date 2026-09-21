//! Réglages du morph, lus dans le fichier que l'hôte désigne.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopConfig {
    /// Refuser d'agir si l'overlay ne peut pas s'afficher. Désactiver serait
    /// piloter l'ordinateur sans que l'utilisateur le voie : à ne faire que
    /// pour des essais.
    #[serde(default = "vrai")]
    pub require_overlay: bool,
    /// Demander confirmation avant une action jugée irréversible.
    #[serde(default = "vrai")]
    pub confirm_risky: bool,
    /// Faire juger le risque par Laya en plus du repli lexical.
    #[serde(default = "vrai")]
    pub laya_risk_check: bool,
    /// Autoriser `desktop_run_command` (PowerShell).
    #[serde(default = "vrai")]
    pub allow_run_command: bool,
    /// Durée maximale d'une commande, en secondes.
    #[serde(default = "delai_commande")]
    pub max_command_seconds: u64,
}

fn vrai() -> bool {
    true
}

fn delai_commande() -> u64 {
    60
}

impl Default for DesktopConfig {
    fn default() -> Self {
        Self {
            require_overlay: true,
            confirm_risky: true,
            laya_risk_check: true,
            allow_run_command: true,
            max_command_seconds: delai_commande(),
        }
    }
}
