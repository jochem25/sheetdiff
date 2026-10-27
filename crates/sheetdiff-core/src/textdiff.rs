//! Tekstvergelijking: runs van A en B (B al in het kader van A) koppelen op inhoud en plek.
//! Uitkomst per run: gelijk, verplaatst, gewijzigd (oud -> nieuw op dezelfde plek),
//! toegevoegd, verwijderd, of alleen een printstempel.

use crate::geom::{Affine, RectPt, PT_PER_MM};
use crate::pdf::TextRun;
use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord)]
pub enum TextChangeKind {
    Changed,
    Added,
    Removed,
    Moved,
    Stamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TextChange {
    pub kind: TextChangeKind,
    pub old: Option<String>,
    pub new: Option<String>,
    /// Positie in A-weergavepunten (bij Added: plek van B-tekst in A-kader).
    pub bbox: RectPt,
    /// Bij Moved: verschuiving in mm.
    pub moved_mm: Option<f64>,
}

pub struct Normalizer {
    stamps: Vec<Regex>,
}

impl Normalizer {
    pub fn new(patterns: &[String]) -> anyhow::Result<Self> {
        Ok(Self {
            stamps: patterns
                .iter()
                .map(|p| Regex::new(p))
                .collect::<Result<_, _>>()?,
        })
    }
    /// Tekst genormaliseerd: NBSP -> spatie, witruimte samengevoegd.
    pub fn norm(&self, s: &str) -> String {
        s.replace('\u{a0}', " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }
    /// Zelfde, met printstempels vervangen door een vaste token.
    pub fn key(&self, s: &str) -> String {
        let mut t = self.norm(s);
        for re in &self.stamps {
            t = re.replace_all(&t, "\u{27e8}stempel\u{27e9}").into_owned();
        }
        t
    }
    pub fn has_stamp(&self, s: &str) -> bool {
        let t = self.norm(s);
        self.stamps.iter().any(|re| re.is_match(&t))
    }
}

/// Vergelijkt tekst. `b_to_a` legt B-weergavepunten op A.
pub fn diff_text(
    a: &[TextRun],
    b: &[TextRun],
    b_to_a: &Affine,
    norm: &Normalizer,
    pos_tol_mm: f64,
) -> Vec<TextChange> {
    diff_text_with(a, b, b_to_a, norm, pos_tol_mm, MAX_MOVE_MM)
}

/// Verder dan dit telt dezelfde tekst niet als "verplaatst" maar als weg + nieuw: een maat
/// "3250" die hier verdwijnt en ergens anders op het blad staat is een andere maat.
pub const MAX_MOVE_MM: f64 = 60.0;

pub fn diff_text_with(
    a: &[TextRun],
    b: &[TextRun],
    b_to_a: &Affine,
    norm: &Normalizer,
    pos_tol_mm: f64,
    max_move_mm: f64,
) -> Vec<TextChange> {
    let tol = pos_tol_mm * PT_PER_MM;
    let bt: Vec<(RectPt, String, String)> = b
        .iter()
        .map(|r| {
            (
                r.bbox.transform(b_to_a),
                norm.norm(&r.text),
                norm.key(&r.text),
            )
        })
        .collect();
    let at: Vec<(RectPt, String, String)> = a
        .iter()
        .map(|r| (r.bbox, norm.norm(&r.text), norm.key(&r.text)))
        .collect();
    let mut used_b = vec![false; bt.len()];
    let mut used_a = vec![false; at.len()];
    let mut out = Vec::new();

    // index B op sleutel
    let mut by_key: std::collections::HashMap<&str, Vec<usize>> = Default::default();
    for (j, (_, _, k)) in bt.iter().enumerate() {
        by_key.entry(k.as_str()).or_default().push(j);
    }
    let center_dist = |p: &RectPt, q: &RectPt| {
        let (a, b) = (p.center(), q.center());
        ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
    };

    // 1. zelfde sleutel, zelfde plek
    for (i, (ra, na, ka)) in at.iter().enumerate() {
        if let Some(cands) = by_key.get(ka.as_str()) {
            let best = cands
                .iter()
                .filter(|&&j| !used_b[j])
                .map(|&j| (j, center_dist(ra, &bt[j].0)))
                .filter(|&(_, d)| d <= tol)
                .min_by(|x, y| x.1.total_cmp(&y.1));
            if let Some((j, _)) = best {
                used_a[i] = true;
                used_b[j] = true;
                if *na != bt[j].1 {
                    // sleutel gelijk, tekst niet: alleen het stempel verschilt
                    out.push(TextChange {
                        kind: TextChangeKind::Stamp,
                        old: Some(na.clone()),
                        new: Some(bt[j].1.clone()),
                        bbox: *ra,
                        moved_mm: None,
                    });
                }
            }
        }
    }
    // 2. zelfde sleutel, andere plek -> verplaatst (dichtstbijzijnde)
    for (i, (ra, na, ka)) in at.iter().enumerate() {
        if used_a[i] {
            continue;
        }
        if let Some(cands) = by_key.get(ka.as_str()) {
            let best = cands
                .iter()
                .filter(|&&j| !used_b[j])
                .map(|&j| (j, center_dist(ra, &bt[j].0)))
                .min_by(|x, y| x.1.total_cmp(&y.1));
            if let Some((j, d)) = best {
                if d <= max_move_mm * PT_PER_MM {
                    used_a[i] = true;
                    used_b[j] = true;
                    out.push(TextChange {
                        kind: TextChangeKind::Moved,
                        old: Some(na.clone()),
                        new: Some(bt[j].1.clone()),
                        bbox: bt[j].0,
                        moved_mm: Some(d / PT_PER_MM),
                    });
                }
            }
        }
    }
    // 3. overgebleven: gewijzigd als er op dezelfde plek een andere tekst staat
    for (i, (ra, na, _)) in at.iter().enumerate() {
        if used_a[i] {
            continue;
        }
        let near = ra.expand(tol.max(ra.height() * 0.5));
        let best = bt
            .iter()
            .enumerate()
            .filter(|(j, (rb, _, _))| !used_b[*j] && near.intersects(rb))
            .map(|(j, (rb, _, _))| {
                (
                    j,
                    rb.intersection_area(ra) / ra.area().max(rb.area()).max(1e-6),
                )
            })
            .max_by(|x, y| x.1.total_cmp(&y.1));
        if let Some((j, _)) = best {
            used_a[i] = true;
            used_b[j] = true;
            out.push(TextChange {
                kind: TextChangeKind::Changed,
                old: Some(na.clone()),
                new: Some(bt[j].1.clone()),
                bbox: ra.union(&bt[j].0),
                moved_mm: None,
            });
        }
    }
    for (i, (ra, na, _)) in at.iter().enumerate() {
        if !used_a[i] {
            out.push(TextChange {
                kind: TextChangeKind::Removed,
                old: Some(na.clone()),
                new: None,
                bbox: *ra,
                moved_mm: None,
            });
        }
    }
    for (j, (rb, nb, _)) in bt.iter().enumerate() {
        if !used_b[j] {
            out.push(TextChange {
                kind: TextChangeKind::Added,
                old: None,
                new: Some(nb.clone()),
                bbox: *rb,
                moved_mm: None,
            });
        }
    }
    out.sort_by(|x, y| {
        x.kind
            .cmp(&y.kind)
            .then(x.bbox.y0.total_cmp(&y.bbox.y0))
            .then(x.bbox.x0.total_cmp(&y.bbox.x0))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::NormalizeCfg;

    fn run(t: &str, x: f64, y: f64) -> TextRun {
        TextRun {
            text: t.into(),
            bbox: RectPt::new(x, y, x + 30.0, y + 8.0),
        }
    }
    fn n() -> Normalizer {
        Normalizer::new(&NormalizeCfg::default().stamp_patterns).unwrap()
    }

    #[test]
    fn both_stamp_formats_are_neutralised() {
        let a = vec![
            run("24-6-2026 10:32:33", 10.0, 10.0),
            run("6/24/2026 10:32:33", 10.0, 50.0),
        ];
        let b = vec![
            run("31-8-2026 9:01:02", 10.0, 10.0),
            run("8/31/2026 9:01:02", 10.0, 50.0),
        ];
        let d = diff_text(&a, &b, &Affine::IDENTITY, &n(), 1.0);
        assert_eq!(d.len(), 2);
        assert!(d.iter().all(|c| c.kind == TextChangeKind::Stamp));
    }

    #[test]
    fn bare_date_change_is_a_real_change() {
        let a = vec![run("15-05-2026", 10.0, 10.0)];
        let b = vec![run("31-08-2026", 10.0, 10.0)];
        let d = diff_text(&a, &b, &Affine::IDENTITY, &n(), 1.0);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].kind, TextChangeKind::Changed);
    }

    #[test]
    fn changed_added_removed_moved() {
        let a = vec![
            run("62,4 m²", 100.0, 100.0),
            run("SL18", 300.0, 300.0),
            run("3250", 500.0, 500.0),
        ];
        let b = vec![
            run("61,8 m²", 100.5, 100.0),
            run("-87", 700.0, 100.0),
            run("3250", 560.0, 500.0),
        ];
        let d = diff_text(&a, &b, &Affine::IDENTITY, &n(), 1.0);
        let kinds: Vec<_> = d
            .iter()
            .map(|c| (c.kind, c.old.clone(), c.new.clone()))
            .collect();
        assert!(kinds.contains(&(
            TextChangeKind::Changed,
            Some("62,4 m²".into()),
            Some("61,8 m²".into())
        )));
        assert!(kinds.contains(&(TextChangeKind::Removed, Some("SL18".into()), None)));
        assert!(kinds.contains(&(TextChangeKind::Added, None, Some("-87".into()))));
        assert!(kinds.contains(&(
            TextChangeKind::Moved,
            Some("3250".into()),
            Some("3250".into())
        )));
    }
}
