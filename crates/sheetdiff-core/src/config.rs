//! Instellingen. Alle drempels staan hier als default, niet als losse getallen in de code.

use crate::raster::DiffParams;
use crate::register::RegParams;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub identify: IdentifyCfg,
    pub normalize: NormalizeCfg,
    pub compare: CompareCfg,
    pub register: RegParams,
    pub comments: CommentsCfg,
}

impl Config {
    /// Leest een TOML-profiel; ontbrekende velden krijgen de default.
    pub fn load(path: &Path) -> anyhow::Result<Config> {
        let s = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&s)?)
    }
    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct IdentifyCfg {
    /// Regex voor een tekeningnummer op een losse tekstregel; groep `nr` is het nummer.
    pub regex: String,
    /// Regex voor het nummer in een bestandsnaam.
    pub file_regex: String,
    /// Deel van breedte en hoogte (vanaf rechtsonder) dat als titelhoek geldt.
    pub title_zone_frac: f64,
    /// Meer dan zoveel nummers buiten de titelhoek (en geen in de titelhoek) = tekeninglijst.
    pub list_page_threshold: usize,
}

impl Default for IdentifyCfg {
    fn default() -> Self {
        Self {
            regex: r"^(?P<nr>[A-Z]{1,3}-\d{3}[a-zA-Z]?)(\s*\([a-z]\))?$".into(),
            file_regex: r"(?P<nr>[A-Z]{1,3}-\d{3}[a-zA-Z]?)".into(),
            title_zone_frac: 0.25,
            list_page_threshold: 5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NormalizeCfg {
    /// Printstempels (datum + tijd). Een kale datum is GEEN stempel: een gewijzigde datum in
    /// de titelhoek is een echte wijziging.
    pub stamp_patterns: Vec<String>,
}

impl Default for NormalizeCfg {
    fn default() -> Self {
        Self {
            stamp_patterns: vec![
                r"\b\d{1,2}-\d{1,2}-\d{4} \d{1,2}:\d{2}(:\d{2})?\b".into(),
                r"\b\d{1,2}/\d{1,2}/\d{4} \d{1,2}:\d{2}(:\d{2})?( ?[AP]M)?\b".into(),
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CompareCfg {
    /// Analyse-dpi voor de pixeldiff (minimaal 300 volgens opdracht).
    pub dpi: f64,
    pub diff: DiffParams,
    /// Strookhoogte in pixels bij renderen (geheugengrens).
    pub strip_px: usize,
    /// Blokgrootte van het grove verschilraster (pixels op analyse-dpi).
    pub coarse_block_px: usize,
    /// Een grof blok telt mee bij minstens zoveel verschilpixels.
    pub min_block_px: u32,
    /// Regio's binnen deze afstand worden samengevoegd (mm).
    pub merge_gap_mm: f64,
    /// Regio's met minder verschilpixels dan dit worden genegeerd (ruis).
    pub min_region_px: u64,
    /// Tekst die minder dan dit verschuift telt als "zelfde plek" (mm).
    pub text_pos_tol_mm: f64,
    /// Een vectorwijziging zonder pixelverschil telt pas mee vanaf deze totale lengte (mm).
    pub min_vector_len_mm: f64,
}

impl Default for CompareCfg {
    fn default() -> Self {
        Self {
            dpi: 300.0,
            diff: DiffParams::default(),
            strip_px: 1536,
            coarse_block_px: 8,
            min_block_px: 2,
            merge_gap_mm: 4.0,
            min_region_px: 12,
            text_pos_tol_mm: 1.0,
            min_vector_len_mm: 5.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CommentsCfg {
    /// Auteurs die NIET als aannemer gelden (eigen markeringen, bijv. afvinken).
    pub own_authors: Vec<String>,
    /// Markering zonder tekst hoort bij een tekstopmerking binnen deze afstand (mm).
    pub attach_mm: f64,
    /// Zoekzone rond een opmerking voor verschillen A->B (mm).
    pub zone_margin_mm: f64,
    /// Rode inhoud: tint-drempels voor detectie van ingetekende markeringen.
    pub red_min: u8,
    pub red_max_other: u8,
    /// Onderwerpen die als akkoordvink/informatief gelden (geen actie).
    pub info_subjects: Vec<String>,
    /// Voorstel "waarschijnlijk verwerkt" vanaf zoveel verschilpixels (150 dpi) in de zone.
    pub done_min_px: u64,
    /// Daaronder tot dit aantal: "onduidelijk"; nog minder: "niet verwerkt".
    pub unclear_min_px: u64,
}

impl Default for CommentsCfg {
    fn default() -> Self {
        Self {
            own_authors: vec!["Jochem".into()],
            attach_mm: 40.0,
            zone_margin_mm: 8.0,
            red_min: 170,
            red_max_other: 110,
            info_subjects: vec!["akkoord".into(), "Zie Opmerking".into()],
            done_min_px: 150,
            unclear_min_px: 20,
        }
    }
}
