//! Écrans et captures.

use image::imageops::{crop_imm, resize, FilterType};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use xcap::Monitor;

/// Un écran, en pixels physiques.
pub struct Display {
    pub index: usize,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f32,
    pub primary: bool,
    monitor: Monitor,
}

impl Display {
    pub fn describe(&self) -> Value {
        json!({
            "index": self.index, "x": self.x, "y": self.y, "width": self.width,
            "height": self.height, "scale": self.scale, "primary": self.primary,
        })
    }
}

pub fn displays() -> Result<Vec<Display>, String> {
    let all = Monitor::all().map_err(|e| format!("écrans : {e}"))?;
    all.into_iter()
        .enumerate()
        .map(|(index, monitor)| {
            let err = |e: xcap::XCapError| format!("écran {index} : {e}");
            Ok(Display {
                index,
                x: monitor.x().map_err(err)?,
                y: monitor.y().map_err(err)?,
                width: monitor.width().map_err(err)?,
                height: monitor.height().map_err(err)?,
                scale: monitor.scale_factor().map_err(err)?,
                primary: monitor.is_primary().map_err(err)?,
                monitor,
            })
        })
        .collect()
}

/// Zone à capturer, en coordonnées écran.
#[derive(Clone, Copy)]
pub struct Region {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

pub struct Shot {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    /// Pixels d'écran représentés par un pixel de l'image : pour revenir aux
    /// coordonnées du curseur, multiplier par cette valeur.
    pub image_scale: f64,
    pub origin: (i32, i32),
    pub display: usize,
}

/// Capture un écran (ou une zone) dans `dir`, réduite à `max_width` au plus.
pub fn capture(
    display: Option<usize>,
    region: Option<Region>,
    max_width: u32,
    dir: &Path,
) -> Result<Shot, String> {
    let all = displays()?;
    let chosen = match display {
        Some(i) => all
            .get(i)
            .ok_or_else(|| format!("Écran {i} inexistant ({} écran(s))", all.len()))?,
        None => all
            .iter()
            .find(|d| d.primary)
            .or(all.first())
            .ok_or("Aucun écran")?,
    };
    let full = chosen
        .monitor
        .capture_image()
        .map_err(|e| format!("capture : {e}"))?;
    let (mut img, origin) = match region {
        Some(r) => {
            let (lx, ly) = (
                (r.x - chosen.x).max(0) as u32,
                (r.y - chosen.y).max(0) as u32,
            );
            let w = r.width.min(full.width().saturating_sub(lx));
            let h = r.height.min(full.height().saturating_sub(ly));
            if w == 0 || h == 0 {
                return Err("La zone demandée est hors de l'écran".into());
            }
            (
                crop_imm(&full, lx, ly, w, h).to_image(),
                (chosen.x + lx as i32, chosen.y + ly as i32),
            )
        }
        None => (full, (chosen.x, chosen.y)),
    };
    let mut image_scale = 1.0;
    if img.width() > max_width {
        image_scale = f64::from(img.width()) / f64::from(max_width);
        let h = (f64::from(img.height()) / image_scale).round().max(1.0) as u32;
        img = resize(&img, max_width, h, FilterType::Triangle);
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("dossier de capture : {e}"))?;
    let path = dir.join(format!("desktop-{}.png", crate::now_ms()));
    img.save(&path)
        .map_err(|e| format!("écriture de la capture : {e}"))?;
    Ok(Shot {
        path,
        width: img.width(),
        height: img.height(),
        image_scale,
        origin,
        display: chosen.index,
    })
}
