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
    pub pair: PairCfg,
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
    /// Veldnaam in de titelhoek waar het tekeningnummer achter of onder staat ('Tekeningnr:',
    /// 'Tekening:', 'BLAD'). Leeg = uit.
    pub field_label_regex: String,
    /// Nummer in dat veld; geldt alleen naast een veldnaam, omdat een boekjesnummer als OVK_400
    /// ook elders op het blad kan staan. Groep `nr` is het nummer. Leeg = uit.
    pub field_regex: String,
    /// Tweede patroon: bouwnummer op een losse tekstregel in de titelhoek (verkoopbladen, die
    /// geen tekeningnummer dragen). Groep `nr` is het nummer; leeg = uit.
    pub building_regex: String,
    /// Bouwnummer in een bestandsnaam; leeg = uit.
    pub building_file_regex: String,
    /// Variant achter het bouwnummer in de bestandsnaam. Basis, OptieA en zwart dragen hetzelfde
    /// bouwnummer maar zijn verschillende bladen; elke treffer komt in de sleutel. Wat niet
    /// past (Basis, _oud) hoort bij het blad zelf. Leeg = uit.
    pub variant_regex: String,
    /// Deel van breedte en hoogte (vanaf rechtsonder) dat als titelhoek geldt.
    pub title_zone_frac: f64,
    /// Tekst in de titelhoek die nooit de titel is: windrichtingen uit het sleutelplan, veldnamen,
    /// adressen, vaste kreten. Leeg = uit.
    pub title_skip_regex: String,
    /// Meer dan zoveel nummers buiten de titelhoek (en geen in de titelhoek) = tekeninglijst.
    pub list_page_threshold: usize,
}

impl Default for IdentifyCfg {
    fn default() -> Self {
        Self {
            regex: r"^(?P<nr>[A-Z]{1,3}-\d{3}[a-zA-Z]?)(\s*\([a-z]\))?$".into(),
            file_regex: r"(?P<nr>[A-Z]{1,3}-\d{3}[a-zA-Z]?)".into(),
            field_label_regex: r"(?i)^(tekening(nr|nummer)?|blad)\s*[.:]?$".into(),
            field_regex: r"^(?P<nr>[A-Z]{1,4}_\d{3}[a-z]?|D\d{3}[a-z]?)$".into(),
            building_regex: r"^(?:Bouwnummer:?\s*)?(?P<nr>[A-Z]\.\d{2}\.\d{2}_[A-Z]\d+_\d{2})$"
                .into(),
            building_file_regex: r"(?P<nr>[A-Z]\.\d{2}\.\d{2}_[A-Z]\d+_\d{2})".into(),
            variant_regex: r"(?i)opti(?:e|on)[ _-]?[a-z0-9]+|zwart".into(),
            title_zone_frac: 0.25,
            title_skip_regex:
                r"(?i)^(noord|zuid|oost|west|wets)+$|:$|tel[.:]|@|www\.|\b\d{4} ?[a-z]{2}\b|^geprint\b|ongeschikt voor|^(definitief|concept)$|kolthof|^(kenmerk|schaal|formaat|projectnr|getekend|blad)$"
                    .into(),
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
    /// Zoveel seconden per blad in een werker; daarna is dat blad NIET VERGELIJKBAAR en gaat de
    /// rest door.
    pub sheet_timeout_s: u64,
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
            sheet_timeout_s: 600,
        }
    }
}

/// Koppelen op inhoud. Gekalibreerde waarden en scoreverdeling: zie STATUS.md.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PairCfg {
    /// Bladen zonder koppeling op sleutel scoren op inhoud.
    pub content: bool,
    /// Tekeningnummers en bouwnummers tellen als sleutel. Uit = alles op inhoud (blinde toets).
    pub use_names: bool,
    /// Onder deze score wordt niet gekoppeld: vervallen plus nieuw.
    pub min_score: f64,
    /// Tussen min_score en deze score: gekoppeld als 'twijfel'.
    pub doubt_score: f64,
    pub weight_lines: f64,
    pub weight_text: f64,
    /// Resolutie van de inktkaart.
    pub dpi: f64,
    /// Inkt = grijswaarde onder dit niveau. Hoog: dunne lijnen zijn op lage dpi lichtgrijs.
    pub ink_level: u8,
    /// Verdikking van de inkt in pixels, tegen kleine verschuivingen.
    pub dilate_px: usize,
    /// Grove uitlijning: grootste verschuiving (mm).
    pub max_shift_mm: f64,
    /// Bladen met een groter verschil in formaat (fractie) zijn nooit hetzelfde blad.
    pub size_tol: f64,
    /// Met minder woorden telt de tekst niet mee.
    pub min_tokens: usize,
    /// Inktkaarten bewaren in de tijdelijke map, zodat een volgende vergelijking ze niet opnieuw
    /// rendert. De sleutel bevat pad, grootte en wijzigingstijd van het bestand.
    pub ink_cache: bool,
}

impl Default for PairCfg {
    fn default() -> Self {
        Self {
            content: true,
            use_names: true,
            min_score: 0.5,
            doubt_score: 0.7,
            weight_lines: 0.6,
            weight_text: 0.4,
            dpi: 40.0,
            ink_level: 245,
            dilate_px: 1,
            max_shift_mm: 15.0,
            size_tol: 0.015,
            min_tokens: 5,
            ink_cache: true,
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
    /// Verwerkt elders: zoveel woorden uit de opmerking moeten in een nieuwe tekst op het blad staan.
    pub elsewhere_min_words: usize,
    /// Een woord uit de opmerking telt vanaf deze lengte.
    pub elsewhere_min_len: usize,
    /// Woorden die een plek aanwijzen; een treffer op zo'n woord is genoeg.
    pub elsewhere_place_words: Vec<String>,
    /// Woorden die niets zeggen over waar het over gaat.
    pub elsewhere_stop_words: Vec<String>,
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
            elsewhere_min_words: 2,
            elsewhere_min_len: 4,
            elsewhere_place_words: vec!["renvooi".into(), "legenda".into()],
            elsewhere_stop_words: [
                "tekst",
                "opnemen",
                "toevoegen",
                "verwijderen",
                "aanpassen",
                "wijzigen",
                "graag",
                "moet",
                "moeten",
                "deze",
                "dit",
                "hier",
                "niet",
                "geen",
                "losse",
                "algemeen",
                "aangeven",
                "opnieuw",
                "zoals",
                "conform",
                "alle",
                "ook",
                "bij",
                "naar",
                "voor",
                "zie",
                "tekening",
                "blad",
                "detail",
            ]
            .map(String::from)
            .to_vec(),
        }
    }
}
