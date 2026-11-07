//! Tekeningnummer en titel uit de titelhoek halen.

use crate::config::IdentifyCfg;
use crate::geom::RectPt;
use crate::pdf::TextRun;
use regex::Regex;
use serde::{Deserialize, Serialize};

/// Zo ver mag het nummer van zijn veldnaam staan.
const FIELD_REACH_MM: f64 = 30.0;
/// Speling in de uitlijning tussen veldnaam en nummer.
const FIELD_ALIGN_MM: f64 = 3.0;

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
    /// Variant van een bouwnummerblad (OptieA, zwart); hoort in de sleutel, niet in het nummer.
    pub variant: Option<String>,
    pub title: String,
    pub method: IdMethod,
    /// Pagina bevat veel nummers buiten de titelhoek: waarschijnlijk een tekeninglijst.
    pub looks_like_list: bool,
    pub number_bbox: Option<RectPt>,
}

/// Tekeningnummer gevonden bij een veldnaam.
struct FieldHit {
    bbox: RectPt,
    number: String,
    /// Het nummer staat onder de veldnaam, niet erachter.
    stacked: bool,
}

pub struct Identifier {
    re: Regex,
    file_re: Regex,
    field_label: Option<Regex>,
    field_re: Option<Regex>,
    building_re: Option<Regex>,
    building_file_re: Option<Regex>,
    variant_re: Option<Regex>,
    title_skip: Option<Regex>,
    cfg: IdentifyCfg,
}

/// Leeg patroon = uit (een lege regex zou op alles passen).
fn optional_regex(s: &str) -> anyhow::Result<Option<Regex>> {
    Ok(if s.is_empty() {
        None
    } else {
        Some(Regex::new(s)?)
    })
}

/// Tekst die twee keer over elkaar geprint is komt als 'OVK_099OVK_099' uit de tekstlaag.
fn undouble(t: &str) -> &str {
    let half = t.len() / 2;
    if t.len() >= 4 && t.len() % 2 == 0 && t.is_char_boundary(half) && t[..half] == t[half..] {
        &t[..half]
    } else {
        t
    }
}

/// Groep `nr`, of anders de hele treffer.
fn capture_nr(re: &Regex, text: &str) -> Option<String> {
    let c = re.captures(text)?;
    c.name("nr").or_else(|| c.get(0)).map(|m| m.as_str().into())
}

impl Identifier {
    pub fn new(cfg: &IdentifyCfg) -> anyhow::Result<Self> {
        Ok(Self {
            re: Regex::new(&cfg.regex)?,
            file_re: Regex::new(&cfg.file_regex)?,
            field_label: optional_regex(&cfg.field_label_regex)?,
            field_re: optional_regex(&cfg.field_regex)?,
            building_re: optional_regex(&cfg.building_regex)?,
            building_file_re: optional_regex(&cfg.building_file_regex)?,
            variant_re: optional_regex(&cfg.variant_regex)?,
            title_skip: optional_regex(&cfg.title_skip_regex)?,
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
        if let Some(f) = self.field_number(runs, &zone, page_size) {
            let bbox = f.bbox;
            // een titelhoek met gestapelde velden heeft geen titelveld; wat daar in de buurt staat
            // hoort bij de tekening. In het brede kader staat de titel links van de zone.
            let page = RectPt::new(0.0, 0.0, w, h);
            return Identity {
                title: if f.stacked {
                    String::new()
                } else {
                    self.guess_title(runs, &page, &bbox)
                },
                number: Some(f.number),
                variant: None,
                method: IdMethod::TitleBlock,
                looks_like_list: false,
                number_bbox: Some(bbox),
            };
        }
        let mut in_zone: Vec<(&TextRun, String)> = Vec::new();
        let mut building_in_zone: Vec<(&TextRun, String)> = Vec::new();
        let mut elsewhere: std::collections::BTreeSet<String> = Default::default();
        for r in runs {
            let t = undouble(r.text.trim());
            if let Some(nr) = capture_nr(&self.re, t) {
                if zone.contains(&r.bbox) {
                    in_zone.push((r, nr));
                } else {
                    elsewhere.insert(nr);
                }
            } else if zone.contains(&r.bbox) {
                // alleen in de titelhoek: daarbuiten is een bouwnummer een label op de tekening
                if let Some(nr) = self.building_re.as_ref().and_then(|re| capture_nr(re, t)) {
                    building_in_zone.push((r, nr));
                }
            }
        }
        let looks_like_list = elsewhere.len() > self.cfg.list_page_threshold && in_zone.is_empty();
        // In de titelhoek: de treffer die het dichtst bij de rechteronderhoek staat.
        let nearest = |v: &[(&TextRun, String)]| -> Option<(RectPt, String)> {
            v.iter()
                .min_by(|a, b| {
                    let da = (w - a.0.bbox.x1).powi(2) + (h - a.0.bbox.y1).powi(2);
                    let db = (w - b.0.bbox.x1).powi(2) + (h - b.0.bbox.y1).powi(2);
                    da.total_cmp(&db)
                })
                .map(|(r, nr)| (r.bbox, nr.clone()))
        };
        if let Some((bbox, nr)) = nearest(&in_zone) {
            return Identity {
                number: Some(nr),
                variant: None,
                title: self.guess_title(runs, &zone, &bbox),
                method: IdMethod::TitleBlock,
                looks_like_list: false,
                number_bbox: Some(bbox),
            };
        }
        if !looks_like_list {
            if let Some((bbox, nr)) = nearest(&building_in_zone) {
                return Identity {
                    variant: self.variant(file_stem, &nr),
                    number: Some(nr),
                    title: self.guess_title(runs, &zone, &bbox),
                    method: IdMethod::TitleBlock,
                    looks_like_list: false,
                    number_bbox: Some(bbox),
                };
            }
        }
        if !looks_like_list && elsewhere.len() == 1 {
            let nr = elsewhere.into_iter().next().expect("lengte 1");
            return Identity {
                number: Some(nr),
                variant: None,
                title: String::new(),
                method: IdMethod::TextLayer,
                looks_like_list,
                number_bbox: None,
            };
        }
        // Bestandsnaam: bouwnummer eerst, zodat "1234-TO-100 Z.09.09_A1_09" dezelfde sleutel
        // krijgt als hetzelfde blad waarvan het bouwnummer in de titelhoek is gelezen.
        let building = self
            .building_file_re
            .as_ref()
            .and_then(|re| capture_nr(re, file_stem));
        if let Some(nr) = building {
            return Identity {
                variant: self.variant(file_stem, &nr),
                number: Some(nr),
                title: String::new(),
                method: IdMethod::FileName,
                looks_like_list,
                number_bbox: None,
            };
        }
        if let Some(nr) = capture_nr(&self.file_re, file_stem) {
            return Identity {
                number: Some(nr),
                variant: None,
                title: String::new(),
                method: IdMethod::FileName,
                looks_like_list,
                number_bbox: None,
            };
        }
        Identity {
            number: None,
            variant: None,
            title: String::new(),
            method: IdMethod::None,
            looks_like_list,
            number_bbox: None,
        }
    }

    /// Nummer uit het veld 'Tekeningnr' van de titelhoek: in dezelfde tekstregel, rechts van de
    /// veldnaam of eronder. Bij meer treffers telt die het dichtst bij de rechteronderhoek.
    fn field_number(
        &self,
        runs: &[TextRun],
        zone: &RectPt,
        (w, h): (f64, f64),
    ) -> Option<FieldHit> {
        let (label, value) = (self.field_label.as_ref()?, self.field_re.as_ref()?);
        let reach = FIELD_REACH_MM * crate::geom::PT_PER_MM;
        let align = FIELD_ALIGN_MM * crate::geom::PT_PER_MM;
        let zone_runs: Vec<&TextRun> = runs.iter().filter(|r| zone.contains(&r.bbox)).collect();
        let mut hits: Vec<FieldHit> = Vec::new();
        for r in &zone_runs {
            let t = undouble(r.text.trim());
            if let Some((l, v)) = t.split_once(':') {
                if label.is_match(l.trim()) {
                    if let Some(number) = capture_nr(value, v.trim()) {
                        hits.push(FieldHit {
                            bbox: r.bbox,
                            number,
                            stacked: false,
                        });
                        continue;
                    }
                }
            }
            if !label.is_match(t) {
                continue;
            }
            let mid = (r.bbox.y0 + r.bbox.y1) / 2.0;
            for v in &zone_runs {
                let beside = v.bbox.y0 <= mid
                    && mid <= v.bbox.y1
                    && v.bbox.x0 >= r.bbox.x1 - align
                    && v.bbox.x0 - r.bbox.x1 <= reach;
                let below = (v.bbox.x0 - r.bbox.x0).abs() <= align
                    && v.bbox.y0 >= r.bbox.y1 - align
                    && v.bbox.y0 - r.bbox.y1 <= reach;
                if beside || below {
                    if let Some(number) = capture_nr(value, undouble(v.text.trim())) {
                        hits.push(FieldHit {
                            bbox: v.bbox,
                            number,
                            stacked: !beside,
                        });
                    }
                }
            }
        }
        hits.into_iter().min_by(|a, b| {
            let da = (w - a.bbox.x1).powi(2) + (h - a.bbox.y1).powi(2);
            let db = (w - b.bbox.x1).powi(2) + (h - b.bbox.y1).powi(2);
            da.total_cmp(&db)
        })
    }

    /// Variant uit het deel van de bestandsnaam na het bouwnummer: "_OptieA_zwart" -> "OptieA_zwart".
    fn variant(&self, file_stem: &str, nr: &str) -> Option<String> {
        let re = self.variant_re.as_ref()?;
        let (_, tail) = file_stem.split_once(nr)?;
        let parts: Vec<&str> = re.find_iter(tail).map(|m| m.as_str()).collect();
        (!parts.is_empty()).then(|| parts.join("_"))
    }

    /// Titel: de hoogste tekstregel in de titelhoek die geen nummer, datum of projectgegeven is.
    fn guess_title(&self, runs: &[TextRun], zone: &RectPt, nr_bbox: &RectPt) -> String {
        let date = Regex::new(r"^\d{1,2}[-/]\d{1,2}[-/]\d{2,4}").expect("vaste regex");
        // Alleen het titelvak rond het nummer: tot 120 mm erboven en 150 mm links ervan. Zo valt
        // een deel van het sleutelplan erbuiten; wat er toch in valt, vangt `title_skip`.
        let near = RectPt::new(
            nr_bbox.x0 - 150.0 * crate::geom::PT_PER_MM,
            nr_bbox.y0 - 120.0 * crate::geom::PT_PER_MM,
            zone.x1,
            nr_bbox.y1 + 10.0 * crate::geom::PT_PER_MM,
        );
        let skip = |t: &str| {
            self.title_skip
                .as_ref()
                .is_some_and(|re| re.is_match(t.trim()))
        };
        runs.iter()
            .filter(|r| zone.contains(&r.bbox) && near.contains(&r.bbox) && r.bbox != *nr_bbox)
            .filter(|r| r.text.chars().filter(|c| c.is_alphabetic()).count() >= 4)
            .filter(|r| !date.is_match(&r.text) && !skip(&r.text))
            .max_by(|a, b| {
                a.bbox
                    .height()
                    .total_cmp(&b.bbox.height())
                    .then(b.bbox.y0.total_cmp(&a.bbox.y0))
            })
            .map(|r| undouble(r.text.trim()).to_string())
            .unwrap_or_default()
    }
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

    #[test]
    fn title_is_not_a_compass_word_a_field_name_or_an_address() {
        let id = Identifier::new(&Config::default().identify).unwrap();
        let tall = |t: &str, y: f64, h: f64| TextRun {
            text: t.into(),
            bbox: RectPt::new(2900.0, y, 3000.0, y + h),
        };
        let runs = vec![
            tall("noordwest", 2000.0, 46.0),
            tall("noordwets", 2010.0, 46.0),
            tall("west", 2020.0, 26.0),
            tall("Tekeningnr:", 2280.0, 40.0),
            tall(
                "Voorbeeldlaan 14, 1234 AB Voorbeeldstad, tel.:0101234567",
                2100.0,
                40.0,
            ),
            tall("Tekening ongeschikt voor uitvoeringsfase", 2120.0, 40.0),
            tall("doorsnede A-A Noord", 2109.0, 35.0),
            run("TO-301", 3111.0, 2305.0),
        ];
        let r = id.identify(&runs, (3370.0, 2384.0), "x");
        assert_eq!(r.title, "doorsnede A-A Noord");
    }

    const A1: (f64, f64) = (2384.0, 1692.0);
    const A3: (f64, f64) = (1191.0, 842.0);

    fn at(t: &str, x: f64, y: f64, x1: f64, y1: f64) -> TextRun {
        TextRun {
            text: t.into(),
            bbox: RectPt::new(x, y, x1, y1),
        }
    }

    #[test]
    fn kba_number_beside_its_field() {
        let runs = vec![
            at("Tekeningnr:", 1362.0, 1112.0, 1417.5, 1121.8),
            at("OV_401", 1432.9, 1112.5, 1501.8, 1130.4),
        ];
        let r = ident().identify(&runs, (1692.0, 1191.0), "1234-OVK_400 detailboekje");
        assert_eq!(r.number.as_deref(), Some("OV_401"));
        assert_eq!(r.method, IdMethod::TitleBlock);
    }

    #[test]
    fn kba_number_under_its_field_and_in_one_run() {
        let runs = vec![
            at("BLAD", 1038.3, 778.6, 1056.9, 783.9),
            at("D004", 1038.8, 788.0, 1071.8, 798.3),
        ];
        let r = ident().identify(&runs, A3, "1234-OVK_400 detailboekje");
        assert_eq!(r.number.as_deref(), Some("D004"));
        let runs = vec![at("Tekening: OVK_101", 1000.0, 788.0, 1100.0, 798.0)];
        assert_eq!(
            ident().identify(&runs, A3, "x").number.as_deref(),
            Some("OVK_101")
        );
    }

    #[test]
    fn kba_title_left_of_the_zone_and_none_in_a_stacked_block() {
        let runs = vec![
            at("detailoverzicht", 1152.2, 932.6, 1294.2, 948.2),
            at("XX00-000 Voorbeeldproject", 1151.8, 974.0, 1300.8, 987.8),
            at("Kolthof Bouwadvies", 1579.5, 928.4, 1648.0, 934.6),
            at("Tekeningnr:", 1362.0, 1112.0, 1417.5, 1121.8),
            at("OV_401", 1432.9, 1112.5, 1501.8, 1130.4),
        ];
        let r = ident().identify(&runs, (1692.0, 1191.0), "x");
        assert_eq!(r.title, "detailoverzicht");
        let runs = vec![
            at("isolatie volgens opgave", 1000.0, 640.0, 1100.0, 665.0),
            at("BLAD", 1038.3, 778.6, 1056.9, 783.9),
            at("D004", 1038.8, 788.0, 1071.8, 798.3),
            at("A. Tekenaar", 903.2, 788.0, 949.9, 796.9),
        ];
        assert_eq!(ident().identify(&runs, A3, "x").title, "");
    }

    #[test]
    fn booklet_number_outside_the_field_is_not_the_sheet_number() {
        // het boekjesnummer staat onder een andere veldnaam, het bladnummer onder BLAD
        let runs = vec![
            at("KENMERK", 1038.4, 755.9, 1070.8, 761.2),
            at("OVK_400", 1038.2, 765.0, 1096.3, 775.0),
            at("BLAD", 1038.3, 778.6, 1056.9, 783.9),
            at("D004", 1038.8, 788.0, 1071.8, 798.3),
        ];
        let r = ident().identify(&runs, A3, "1234-OVK_400 detailboekje");
        assert_eq!(r.number.as_deref(), Some("D004"));
        // zonder veld is OVK_400 geen nummer, ook niet uit de bestandsnaam
        let runs = vec![at("OVK_400", 1038.2, 788.0, 1096.3, 798.0)];
        let r = ident().identify(&runs, A3, "1234-OVK_400 detailboekje");
        assert_eq!(r.number, None);
    }

    #[test]
    fn double_printed_text_is_read_once() {
        let runs = vec![
            at("Tekeningnr:Tekeningnr:", 3040.1, 2305.4, 3095.6, 2315.2),
            at("OVK_099OVK_099", 3111.0, 2306.0, 3195.7, 2323.8),
        ];
        let r = ident().identify(&runs, (3370.0, 2384.0), "x");
        assert_eq!(r.number.as_deref(), Some("OVK_099"));
    }

    #[test]
    fn to_number_in_the_field_keeps_its_old_route() {
        let runs = vec![
            at("Tekeningnr:", 3040.0, 2305.0, 3095.0, 2315.0),
            at("TO-204n", 3111.0, 2305.0, 3190.0, 2323.0),
        ];
        let r = ident().identify(&runs, (3370.0, 2384.0), "x");
        assert_eq!(r.number.as_deref(), Some("TO-204n"));
    }

    #[test]
    fn empty_field_pattern_switches_kba_numbers_off() {
        let mut cfg = Config::default().identify;
        cfg.field_regex.clear();
        let id = Identifier::new(&cfg).unwrap();
        let runs = vec![
            at("BLAD", 1038.3, 778.6, 1056.9, 783.9),
            at("D004", 1038.8, 788.0, 1071.8, 798.3),
        ];
        assert_eq!(id.identify(&runs, A3, "x").number, None);
    }

    fn ident() -> Identifier {
        Identifier::new(&Config::default().identify).unwrap()
    }

    /// Titelhoek van een verkoopblad: los label en los nummer, geen tekeningnummer.
    fn sales_runs() -> Vec<TextRun> {
        vec![
            run("Z.09.09", 1430.0, 1337.0),
            run("Verkooptekening", 1929.0, 1428.0),
            run("Bouwnummer:", 1928.0, 1459.0),
            run("Z.09.09_A1_09", 2076.0, 1459.0),
        ]
    }

    #[test]
    fn building_number_in_title_block_wins_over_file_name() {
        for stem in ["Z.09.09_A1_09 Basis", "1234-TO-100 Z.09.09_A1_09 Basis"] {
            let r = ident().identify(&sales_runs(), A1, stem);
            assert_eq!(r.number.as_deref(), Some("Z.09.09_A1_09"), "{stem}");
            assert_eq!(r.method, IdMethod::TitleBlock);
            assert_eq!(r.variant, None);
        }
        // ook als de bestandsnaam een heel ander nummer draagt
        let r = ident().identify(&sales_runs(), A1, "1234-TO-100 iets anders");
        assert_eq!(r.number.as_deref(), Some("Z.09.09_A1_09"));
    }

    #[test]
    fn building_number_with_label_in_one_run() {
        let runs = vec![run("Bouwnummer: Z.09.09_A1_09", 1928.0, 1459.0)];
        let r = ident().identify(&runs, A1, "x");
        assert_eq!(r.number.as_deref(), Some("Z.09.09_A1_09"));
    }

    #[test]
    fn drawing_number_in_title_block_wins_over_building_number() {
        let mut runs = sales_runs();
        runs.push(run("TO-100", 2076.0, 1600.0));
        let r = ident().identify(&runs, A1, "x");
        assert_eq!(r.number.as_deref(), Some("TO-100"));
        assert_eq!(r.method, IdMethod::TitleBlock);
    }

    #[test]
    fn building_number_outside_title_block_does_not_count() {
        let runs = vec![run("Z.09.09_A1_09", 100.0, 100.0)];
        let r = ident().identify(&runs, A1, "1234-TO-100 plattegrond");
        assert_eq!(r.number.as_deref(), Some("TO-100"));
        assert_eq!(r.method, IdMethod::FileName);
    }

    #[test]
    fn building_number_from_file_name() {
        for stem in ["Z.09.09_A1_09 Basis", "1234-TO-100 Z.09.09_A1_09 Basis"] {
            let r = ident().identify(&[], A1, stem);
            assert_eq!(r.number.as_deref(), Some("Z.09.09_A1_09"), "{stem}");
            assert_eq!(r.method, IdMethod::FileName);
        }
    }

    #[test]
    fn variants_stay_apart_but_basis_and_old_do_not() {
        let v = |stem: &str| ident().identify(&sales_runs(), A1, stem).variant;
        assert_eq!(
            v("Z.09.09_A1_09_OptieA_zwart").as_deref(),
            Some("OptieA_zwart")
        );
        assert_eq!(v("Z.09.09_A1_09_zwart").as_deref(), Some("zwart"));
        assert_eq!(v("Z.09.09_A1_09 option 1").as_deref(), Some("option 1"));
        assert_eq!(v("Z.09.09_A1_09 Basis"), None);
        assert_eq!(v("1234-TO-100 Z.09.09_A1_09 Basis_oud"), None);
    }

    #[test]
    fn empty_pattern_switches_building_number_off() {
        let mut cfg = Config::default().identify;
        cfg.building_regex.clear();
        cfg.building_file_regex.clear();
        let id = Identifier::new(&cfg).unwrap();
        let r = id.identify(&sales_runs(), A1, "1234-TO-100 Z.09.09_A1_09 Basis");
        assert_eq!(r.number.as_deref(), Some("TO-100"));
        assert_eq!(r.method, IdMethod::FileName);
    }
}
