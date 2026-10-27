//! Tekeningnummer en titel uit de titelhoek halen.

use crate::config::IdentifyCfg;
use crate::geom::RectPt;
use crate::pdf::TextRun;
use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IdMethod {
    /// Tekstlaag, in de titelhoek (rechtsonder).
    TitleBlock,
    /// Tekstlaag, buiten de titelhoek (enige treffer).
    TextLayer,
    /// Uit de bestandsnaam.
    FileName,
    /// Niet gevonden.
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identity {
    pub number: Option<String>,
    pub title: String,
    pub method: IdMethod,
    /// Pagina bevat veel nummers buiten de titelhoek: waarschijnlijk een tekeninglijst.
    pub looks_like_list: bool,
    pub number_bbox: Option<RectPt>,
}

pub struct Identifier {
    re: Regex,
    file_re: Regex,
    cfg: IdentifyCfg,
}

impl Identifier {
    pub fn new(cfg: &IdentifyCfg) -> anyhow::Result<Self> {
        Ok(Self {
            re: Regex::new(&cfg.regex)?,
            file_re: Regex::new(&cfg.file_regex)?,
            cfg: cfg.clone(),
        })
    }

    pub fn identify(&self, runs: &[TextRun], page_size: (f64, f64), file_stem: &str) -> Identity {
        let (w, h) = page_size;
        let zone = RectPt::new(
            w * (1.0 - self.cfg.title_zone_frac),
            h * (1.0 - self.cfg.title_zone_frac),
            w,
            h,
        );
        let mut in_zone: Vec<(&TextRun, String)> = Vec::new();
        let mut elsewhere: std::collections::BTreeSet<String> = Default::default();
        for r in runs {
            let t = r.text.trim();
            if let Some(c) = self.re.captures(t) {
                let nr = c
                    .name("nr")
                    .map(|m| m.as_str())
                    .unwrap_or_else(|| c.get(0).map(|m| m.as_str()).unwrap_or(t))
                    .to_string();
                if zone.contains(&r.bbox) {
                    in_zone.push((r, nr));
                } else {
                    elsewhere.insert(nr);
                }
            }
        }
        let looks_like_list = elsewhere.len() > self.cfg.list_page_threshold && in_zone.is_empty();
        // In de titelhoek: de treffer die het dichtst bij de rechteronderhoek staat.
        if let Some((run, nr)) = in_zone.iter().min_by(|a, b| {
            let da = (w - a.0.bbox.x1).powi(2) + (h - a.0.bbox.y1).powi(2);
            let db = (w - b.0.bbox.x1).powi(2) + (h - b.0.bbox.y1).powi(2);
            da.total_cmp(&db)
        }) {
            return Identity {
                number: Some(nr.clone()),
                title: guess_title(runs, &zone, &run.bbox),
                method: IdMethod::TitleBlock,
                looks_like_list: false,
                number_bbox: Some(run.bbox),
            };
        }
        if !looks_like_list && elsewhere.len() == 1 {
            let nr = elsewhere.into_iter().next().expect("lengte 1");
            return Identity {
                number: Some(nr),
                title: String::new(),
                method: IdMethod::TextLayer,
                looks_like_list,
                number_bbox: None,
            };
        }
        if let Some(c) = self.file_re.captures(file_stem) {
            if let Some(m) = c.name("nr").or_else(|| c.get(0)) {
                return Identity {
                    number: Some(m.as_str().to_string()),
                    title: String::new(),
                    method: IdMethod::FileName,
                    looks_like_list,
                    number_bbox: None,
                };
            }
        }
        Identity {
            number: None,
            title: String::new(),
            method: IdMethod::None,
            looks_like_list,
            number_bbox: None,
        }
    }
}

/// Titel: de hoogste tekstregel in de titelhoek die geen nummer, datum of projectgegeven is.
fn guess_title(runs: &[TextRun], zone: &RectPt, nr_bbox: &RectPt) -> String {
    let date = Regex::new(r"^\d{1,2}[-/]\d{1,2}[-/]\d{2,4}").expect("vaste regex");
    // Alleen het titelvak rond het nummer: tot 120 mm erboven en 150 mm links ervan. Zo valt
    // een sleutelplan of noordpijl ("noordwest") boven de titelhoek erbuiten.
    let near = RectPt::new(
        nr_bbox.x0 - 150.0 * crate::geom::PT_PER_MM,
        nr_bbox.y0 - 120.0 * crate::geom::PT_PER_MM,
        zone.x1,
        nr_bbox.y1 + 10.0 * crate::geom::PT_PER_MM,
    );
    runs.iter()
        .filter(|r| zone.contains(&r.bbox) && near.contains(&r.bbox) && r.bbox != *nr_bbox)
        .filter(|r| r.text.chars().filter(|c| c.is_alphabetic()).count() >= 4)
        .filter(|r| !date.is_match(&r.text))
        .max_by(|a, b| {
            a.bbox
                .height()
                .total_cmp(&b.bbox.height())
                .then(b.bbox.y0.total_cmp(&a.bbox.y0))
        })
        .map(|r| r.text.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn run(t: &str, x: f64, y: f64) -> TextRun {
        TextRun {
            text: t.into(),
            bbox: RectPt::new(x, y, x + 40.0, y + 8.0),
        }
    }

    #[test]
    fn number_in_title_block_wins_over_references() {
        let id = Identifier::new(&Config::default().identify).unwrap();
        let runs = vec![
            run("TO-501", 100.0, 100.0),
            run("TO-502", 200.0, 100.0),
            run("Noordblok gevelaanzicht Noordoost", 2830.0, 2120.0),
            run("TO-204n", 3111.0, 2305.0),
        ];
        let r = id.identify(&runs, (3370.0, 2384.0), "x");
        assert_eq!(r.number.as_deref(), Some("TO-204n"));
        assert_eq!(r.method, IdMethod::TitleBlock);
    }

    #[test]
    fn drawing_list_is_not_a_sheet() {
        let id = Identifier::new(&Config::default().identify).unwrap();
        let runs: Vec<TextRun> = (0..20)
            .map(|i| run(&format!("TO-{:03}", 100 + i), 100.0, 50.0 + i as f64 * 10.0))
            .collect();
        let r = id.identify(&runs, (1190.0, 842.0), "1234-0 Tekeninglijst");
        assert!(r.looks_like_list);
        assert_eq!(r.number, None);
    }

    #[test]
    fn suffixes_and_filename_fallback() {
        let id = Identifier::new(&Config::default().identify).unwrap();
        let r = id.identify(&[run("TO-099t", 3111.0, 2305.0)], (3370.0, 2384.0), "x");
        assert_eq!(r.number.as_deref(), Some("TO-099t"));
        let r = id.identify(
            &[],
            (3370.0, 2384.0),
            "1234-TO-100n plattegrond begane grond Noord",
        );
        assert_eq!(r.number.as_deref(), Some("TO-100n"));
        assert_eq!(r.method, IdMethod::FileName);
    }
}
