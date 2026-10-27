//! Bladvergelijking: registratie -> pixeldiff in stroken -> regio's -> tekstdiff -> oordeel.

use crate::config::Config;
use crate::geom::{Affine, RectPt, MM_PER_PT, PT_PER_MM};
use crate::issue::{Issue, SheetRef};
use crate::pdf::{self, Doc};
use crate::raster::{self, CoarseMask};
use crate::register::{self, Registration};
use crate::textdiff::{self, Normalizer, TextChange, TextChangeKind};
use anyhow::Result;
use pdfium_render::prelude::PdfPage;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord, Hash)]
pub enum Verdict {
    /// Vergelijking kon niet betrouwbaar (uitlijning mislukt, bestand onleesbaar). Telt NOOIT
    /// als "geen verschillen".
    NotComparable,
    Changed,
    AddedInB,
    RemovedFromB,
    /// Alleen verschoven onderdelen (viewport, legenda, tekst op andere plek).
    MovedOnly,
    StampOnly,
    Identical,
}

impl Verdict {
    pub fn label(&self) -> &'static str {
        match self {
            Verdict::NotComparable => "NIET VERGELIJKBAAR",
            Verdict::Changed => "gewijzigd",
            Verdict::AddedInB => "nieuw in B",
            Verdict::RemovedFromB => "vervallen in B",
            Verdict::MovedOnly => "alleen verplaatst",
            Verdict::StampOnly => "alleen printstempel",
            Verdict::Identical => "gelijk",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Region {
    /// In A-weergavepunten.
    pub bbox: RectPt,
    pub removed_px: u64,
    pub added_px: u64,
    /// Regio valt (vrijwel) geheel op een printstempel.
    pub stamp: bool,
    /// Regio verdwijnt als B met de kadertransformatie wordt gelegd: dit deel is verschoven
    /// (mm), geen inhoudelijke wijziging.
    #[serde(default)]
    pub moved_mm: Option<(f64, f64)>,
    /// Ook door de vectorvergelijking gevonden.
    #[serde(default)]
    pub by_vector: bool,
    /// Indices in `SheetDiff::text` van tekstwijzigingen in deze regio.
    pub text: Vec<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SheetDiff {
    pub key: String,
    pub title: String,
    pub verdict: Verdict,
    pub reasons: Vec<String>,
    pub a: Option<SheetLoc>,
    pub b: Option<SheetLoc>,
    pub registration: Option<Registration>,
    pub dpi: f64,
    pub removed_px: u64,
    pub added_px: u64,
    /// Aandeel verschilpixels t.o.v. het blad, in procent.
    pub pixel_pct: f64,
    pub regions: Vec<Region>,
    pub text: Vec<TextChange>,
    pub seconds: f64,
    /// Vectorvergelijking (paden): samenvatting en clusters in A-weergavepunten.
    #[serde(default)]
    pub vector: Option<VectorSummary>,
    #[serde(skip)]
    pub mask: Option<CoarseMask>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VectorSummary {
    pub paths_a: usize,
    pub paths_b: usize,
    pub removed_segs: usize,
    pub added_segs: usize,
    pub style_paths: usize,
    pub clusters: Vec<VectorCluster>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorCluster {
    pub kind: crate::vector::VChangeKind,
    pub bbox: RectPt,
    pub segs: usize,
    /// Totale lengte van de segmenten in mm.
    #[serde(default)]
    pub length_mm: f64,
    pub note: String,
    /// Valt in een pixelregio (anders: alleen door vector gezien).
    pub in_region: bool,
}

/// Voegt vectorwijzigingen van dezelfde soort samen tot clusters (union-find op afstand).
#[allow(clippy::needless_range_loop)]
pub fn cluster_vchanges(ch: &[crate::vector::VChange], gap: f64) -> Vec<VectorCluster> {
    let n = ch.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(p: &mut [usize], mut x: usize) -> usize {
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }
    // sorteren op x0 zodat we vroeg kunnen stoppen
    let mut idx: Vec<usize> = (0..n).collect();
    idx.sort_by(|&a, &b| ch[a].bbox.x0.total_cmp(&ch[b].bbox.x0));
    for (ii, &i) in idx.iter().enumerate() {
        for &j in &idx[ii + 1..] {
            if ch[j].bbox.x0 > ch[i].bbox.x1 + gap {
                break;
            }
            if ch[i].kind == ch[j].kind && ch[i].bbox.distance(&ch[j].bbox) <= gap {
                let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                if a != b {
                    parent[a.max(b)] = a.min(b);
                }
            }
        }
    }
    let mut map: std::collections::BTreeMap<usize, VectorCluster> = Default::default();
    for i in 0..n {
        let r = find(&mut parent, i);
        let e = map.entry(r).or_insert(VectorCluster {
            kind: ch[i].kind,
            bbox: ch[i].bbox,
            segs: 0,
            length_mm: 0.0,
            note: ch[i].note.clone(),
            in_region: false,
        });
        e.bbox = e.bbox.union(&ch[i].bbox);
        e.segs += ch[i].segs.len().max(1);
        e.length_mm += ch[i]
            .segs
            .iter()
            .map(|s| ((s.x1 - s.x0) as f64).hypot((s.y1 - s.y0) as f64) / PT_PER_MM)
            .sum::<f64>();
    }
    map.into_values().collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SheetLoc {
    pub file: std::path::PathBuf,
    pub page: usize,
    pub booklets: Vec<String>,
}

impl From<&SheetRef> for SheetLoc {
    fn from(s: &SheetRef) -> Self {
        SheetLoc {
            file: s.file.clone(),
            page: s.page,
            booklets: s.booklets.clone(),
        }
    }
}

/// Pixeldiff van pagina B (via `reg`) tegen pagina A, in stroken, met grof verschilraster.
pub fn pixel_diff(
    page_a: &PdfPage,
    size_a: (f64, f64),
    page_b: &PdfPage,
    b_to_a: &Affine,
    cfg: &Config,
) -> Result<CoarseMask> {
    let c = &cfg.compare;
    let s = c.dpi / 72.0;
    let w = ((size_a.0 * s).round() as usize).max(1);
    let h = ((size_a.1 * s).round() as usize).max(1);
    let m = c.diff.radius_px + 1; // overlap tussen stroken
    let mut mask = CoarseMask::new(w, h, c.coarse_block_px);
    let mut y0 = 0usize;
    while y0 < h {
        let inner = c.strip_px.min(h - y0);
        let top = y0.saturating_sub(m);
        let bottom = (y0 + inner + m).min(h);
        let sh = bottom - top;
        let to_px_a = Affine::scale(s).then(&Affine::translate(0.0, -(top as f64)));
        let to_px_b = b_to_a.then(&to_px_a);
        let ga = pdf::render_gray(page_a, &to_px_a, w, sh)?;
        let gb = pdf::render_gray(page_b, &to_px_b, w, sh)?;
        let d = raster::diff(&ga, &gb, &c.diff);
        // alleen de binnenrijen meetellen
        let skip = y0 - top;
        let inner_map = raster::DiffMap {
            w: d.w,
            h: inner,
            class: d.class[skip * d.w..(skip + inner) * d.w].to_vec(),
            removed: 0,
            added: 0,
        };
        mask.accumulate(&inner_map, y0);
        y0 += inner;
    }
    Ok(mask)
}

fn mask_regions(mask: &CoarseMask, cfg: &Config) -> Vec<Region> {
    let c = &cfg.compare;
    let px_per_mm = c.dpi / 25.4;
    let gap_cells = ((c.merge_gap_mm * px_per_mm) / mask.k as f64).ceil() as usize;
    let pt_per_px = 72.0 / c.dpi;
    raster::regions(mask, c.min_block_px, gap_cells)
        .into_iter()
        .filter(|r| r.removed + r.added >= c.min_region_px)
        .map(|r| Region {
            bbox: RectPt::new(
                (r.cx0 * mask.k) as f64 * pt_per_px,
                (r.cy0 * mask.k) as f64 * pt_per_px,
                ((r.cx1 + 1) * mask.k) as f64 * pt_per_px,
                ((r.cy1 + 1) * mask.k) as f64 * pt_per_px,
            ),
            removed_px: r.removed,
            added_px: r.added,
            stamp: false,
            moved_mm: None,
            by_vector: false,
            text: Vec::new(),
        })
        .collect()
}

/// Vergelijkt een bladpaar.
pub fn compare_sheet(
    a: &SheetRef,
    b: &SheetRef,
    cfg: &Config,
    manual: Option<&Registration>,
) -> Result<SheetDiff> {
    let t0 = Instant::now();
    let doc_a = Doc::open(&a.file)?;
    let doc_b = Doc::open(&b.file)?;
    let page_a = doc_a.page(a.page)?;
    let page_b = doc_b.page(b.page)?;
    let size_a = a.geom.display_size();
    let mut reasons = Vec::new();

    let reg = match manual {
        Some(r) => r.clone(),
        None => register::register(&page_a, &a.geom, &page_b, &b.geom, &cfg.register)?,
    };
    let mut out = SheetDiff {
        key: a.key.clone(),
        title: if a.title.is_empty() {
            b.title.clone()
        } else {
            a.title.clone()
        },
        verdict: Verdict::NotComparable,
        reasons: Vec::new(),
        a: Some(a.into()),
        b: Some(b.into()),
        registration: Some(reg.clone()),
        dpi: cfg.compare.dpi,
        removed_px: 0,
        added_px: 0,
        pixel_pct: 0.0,
        regions: Vec::new(),
        text: Vec::new(),
        seconds: 0.0,
        vector: None,
        mask: None,
    };
    if !reg.ok {
        reasons.push(reg.note.clone());
        out.reasons = reasons;
        out.seconds = t0.elapsed().as_secs_f64();
        return Ok(out);
    }
    if !reg.identity {
        reasons.push(format!("B uitgelijnd op A: {}", reg.note));
    }

    // pixels
    let mask = pixel_diff(&page_a, size_a, &page_b, &reg.transform, cfg)?;
    let mut regions = mask_regions(&mask, cfg);

    // verplaatste delen: viewport verschoven binnen een vast kader
    let same_frame = (size_a.0 - b.geom.display_size().0).abs() < 0.5
        && (size_a.1 - b.geom.display_size().1).abs() < 0.5
        && reg.rotation == crate::geom::Rot90::R0;
    if !reg.identity && same_frame && !regions.is_empty() {
        let rects: Vec<RectPt> = regions.iter().map(|r| r.bbox).collect();
        let moved = crate::moved::classify(
            &page_a,
            size_a,
            &page_b,
            &reg.transform,
            &Affine::IDENTITY,
            &rects,
            &cfg.compare.diff,
            100.0,
        )?;
        for (r, m) in regions.iter_mut().zip(moved) {
            r.moved_mm = m;
        }
        let n = regions.iter().filter(|r| r.moved_mm.is_some()).count();
        if n > 0 {
            reasons.push(format!(
                "{n} regio('s) verschoven t.o.v. de tekening (kader/legenda/viewport), apart gemeld als verplaatst"
            ));
        }
    }

    // annotaties worden niet vergeleken: luid melden als ze er zijn
    let markup = |p: &pdfium_render::prelude::PdfPage| {
        use pdfium_render::prelude::PdfPageAnnotationType as T;
        p.annotations()
            .iter()
            .filter(|a| {
                !matches!(
                    a.annotation_type(),
                    T::Link | T::Widget | T::Popup | T::XfaWidget
                )
            })
            .count()
    };
    // hyperlinks (bijv. Revit-viewverwijzingen) tellen niet: die tekenen niets
    let (na, nb) = (markup(&page_a), markup(&page_b));
    if na + nb > 0 {
        reasons.push(format!(
            "blad bevat annotaties (A {na}, B {nb}); die zijn NIET in de vergelijking meegenomen"
        ));
    }

    // tekst; een leesfout is iets anders dan een lege tekstlaag
    let mut text_failed = false;
    let mut read = |p: &pdfium_render::prelude::PdfPage, g: &crate::geom::PageGeom, side: &str| {
        match pdf::text_runs(p, g) {
            Ok(r) => r,
            Err(e) => {
                text_failed = true;
                reasons.push(format!("tekstlaag {side} onleesbaar: {e:#}"));
                Vec::new()
            }
        }
    };
    let runs_a = read(&page_a, &a.geom, "A");
    let runs_b = read(&page_b, &b.geom, "B");
    if !text_failed && (runs_a.is_empty() || runs_b.is_empty()) {
        reasons.push("geen tekstlaag op A of B: alleen pixelvergelijking".into());
    }
    let norm = Normalizer::new(&cfg.normalize.stamp_patterns)?;
    // Tekst in verschoven delen (kader, legenda) vergelijken onder de kadertransformatie,
    // de rest onder de hoofdtransformatie. Anders koppelt de nabijheidsstap kadertekst aan
    // verkeerde buren.
    let moved_rects: Vec<RectPt> = regions
        .iter()
        .filter(|r| r.moved_mm.is_some())
        .map(|r| r.bbox.expand(PT_PER_MM))
        .collect();
    let in_moved = |b: &RectPt| moved_rects.iter().any(|m| m.intersects(b));
    let text = if moved_rects.is_empty() {
        textdiff::diff_text(
            &runs_a,
            &runs_b,
            &reg.transform,
            &norm,
            cfg.compare.text_pos_tol_mm,
        )
    } else {
        let (ma, ra): (Vec<pdf::TextRun>, Vec<pdf::TextRun>) =
            runs_a.iter().cloned().partition(|r| in_moved(&r.bbox));
        let (mb, rb): (Vec<pdf::TextRun>, Vec<pdf::TextRun>) =
            runs_b.iter().cloned().partition(|r| in_moved(&r.bbox));
        let mut t =
            textdiff::diff_text(&ra, &rb, &reg.transform, &norm, cfg.compare.text_pos_tol_mm);
        t.extend(textdiff::diff_text(
            &ma,
            &mb,
            &Affine::IDENTITY,
            &norm,
            cfg.compare.text_pos_tol_mm,
        ));
        t.sort_by(|x, y| {
            x.kind
                .cmp(&y.kind)
                .then(x.bbox.y0.total_cmp(&y.bbox.y0))
                .then(x.bbox.x0.total_cmp(&y.bbox.x0))
        });
        t
    };

    // vector (paden): derde methode naast raster en tekst
    let vec_summary = {
        let (va, sa) = crate::vector::extract(&page_a, &a.geom)?;
        let (vb, sb) = crate::vector::extract(&page_b, &b.geom)?;
        if sa.paths == 0 && sb.paths == 0 {
            reasons.push("geen vectorlaag (scan of gerasterde PDF): alleen raster en tekst".into());
        }
        let ch =
            crate::vector::compare(&va, &vb, &reg.transform, &crate::vector::VParams::default());
        let mut clusters = cluster_vchanges(&ch, cfg.compare.merge_gap_mm * PT_PER_MM);
        // verplaatste delen: vectorclusters in een verplaatste regio tellen niet
        clusters.retain(|c| !moved_rects.iter().any(|m| m.contains(&c.bbox)));
        VectorSummary {
            paths_a: sa.paths,
            paths_b: sb.paths,
            removed_segs: ch
                .iter()
                .filter(|c| c.kind == crate::vector::VChangeKind::Removed)
                .map(|c| c.segs.len())
                .sum(),
            added_segs: ch
                .iter()
                .filter(|c| c.kind == crate::vector::VChangeKind::Added)
                .map(|c| c.segs.len())
                .sum(),
            style_paths: ch
                .iter()
                .filter(|c| c.kind == crate::vector::VChangeKind::Style)
                .count(),
            clusters,
        }
    };

    // stempelzones: tekstruns met een printstempel, in A-kader
    let margin = 1.5 * PT_PER_MM;
    let mut stamp_boxes: Vec<RectPt> = runs_a
        .iter()
        .filter(|r| norm.has_stamp(&r.text))
        .map(|r| r.bbox.expand(margin))
        .collect();
    stamp_boxes.extend(
        runs_b
            .iter()
            .filter(|r| norm.has_stamp(&r.text))
            .map(|r| r.bbox.transform(&reg.transform).expand(margin)),
    );
    for r in regions.iter_mut() {
        r.stamp = crate::geom::covered_fraction(&r.bbox, &stamp_boxes, 24) >= 0.8;
        for (i, t) in text.iter().enumerate() {
            if t.bbox.intersects(&r.bbox) {
                r.text.push(i);
            }
        }
    }
    let real_text = text
        .iter()
        .filter(|t| t.kind != TextChangeKind::Stamp)
        .count();
    let stamp_text = text.len() - real_text;
    let mut vec_summary = vec_summary;
    for c in vec_summary.clusters.iter_mut() {
        c.in_region = regions
            .iter()
            .any(|r| r.bbox.expand(PT_PER_MM).intersects(&c.bbox));
    }
    for r in regions.iter_mut() {
        r.by_vector = vec_summary
            .clusters
            .iter()
            .any(|c| r.bbox.expand(PT_PER_MM).intersects(&c.bbox));
    }
    // alleen door vector gezien (bijv. lijndikte of kleur), niet op een printstempel
    let (vector_only, invisible) = vec_summary
        .clusters
        .iter()
        .filter(|c| !c.in_region && !stamp_boxes.iter().any(|s| s.contains(&c.bbox)))
        .fold((0usize, 0usize), |(v, i), c| {
            if c.kind == crate::vector::VChangeKind::Style
                || c.length_mm >= cfg.compare.min_vector_len_mm
            {
                (v + 1, i)
            } else {
                (v, i + 1)
            }
        });
    if invisible > 0 {
        reasons.push(format!(
            "{invisible} kleine vectorwijziging(en) zonder zichtbaar pixelverschil (afgedekt of < {} mm); niet meegeteld",
            cfg.compare.min_vector_len_mm
        ));
    }
    if vector_only > 0 {
        reasons.push(format!(
            "{vector_only} vectorwijziging(en) zonder pixelverschil (bijv. lijndikte of kleur)"
        ));
    }
    let real_regions = regions
        .iter()
        .filter(|r| !r.stamp && r.moved_mm.is_none())
        .count()
        + vector_only;

    out.removed_px = regions.iter().map(|r| r.removed_px).sum();
    out.added_px = regions.iter().map(|r| r.added_px).sum();
    let total_px = (mask.w * mask.k * mask.h * mask.k) as f64;
    out.pixel_pct = 100.0 * (out.removed_px + out.added_px) as f64 / total_px.max(1.0);
    let moved_regions = regions.iter().filter(|r| r.moved_mm.is_some()).count();
    let moved_text = text
        .iter()
        .filter(|t| t.kind == TextChangeKind::Moved)
        .count();
    let content_text = real_text - moved_text;
    out.verdict = if content_text == 0 && real_regions == 0 {
        if moved_regions > 0 || moved_text > 0 {
            Verdict::MovedOnly
        } else if stamp_text == 0 && regions.is_empty() {
            Verdict::Identical
        } else {
            Verdict::StampOnly
        }
    } else {
        Verdict::Changed
    };
    if text_failed
        && matches!(
            out.verdict,
            Verdict::Identical | Verdict::StampOnly | Verdict::MovedOnly
        )
    {
        out.verdict = Verdict::NotComparable;
        reasons.push("tekstlaag kon niet gelezen worden: 'gelijk' is niet vast te stellen".into());
    }
    if real_text > 0 && real_regions == 0 {
        reasons.push(
            "tekst gewijzigd zonder zichtbaar pixelverschil (bijv. verborgen of overlappende tekst)"
                .into(),
        );
    }
    out.vector = Some(vec_summary);
    out.regions = regions;
    out.text = text;
    out.reasons = reasons;
    out.mask = Some(mask);
    out.seconds = t0.elapsed().as_secs_f64();
    Ok(out)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueDiff {
    pub a_root: std::path::PathBuf,
    pub b_root: std::path::PathBuf,
    pub sheets: Vec<SheetDiff>,
    /// Boekjessamenstelling: (boekje, alleen in A, alleen in B).
    pub booklet_changes: Vec<(String, Vec<String>, Vec<String>)>,
    pub warnings: Vec<String>,
    pub config: Config,
}

impl IssueDiff {
    pub fn count(&self, v: Verdict) -> usize {
        self.sheets.iter().filter(|s| s.verdict == v).count()
    }
    /// Exitcode volgens plan: 0 geen verschillen, 1 verschillen, 2 niet vergelijkbaar.
    pub fn exit_code(&self) -> i32 {
        // nul vergeleken bladen is nooit "geen verschillen"
        if self.count(Verdict::NotComparable) > 0 || self.sheets.is_empty() {
            2
        } else if self
            .sheets
            .iter()
            .any(|s| !matches!(s.verdict, Verdict::Identical | Verdict::StampOnly))
        {
            1
        } else {
            0
        }
    }
}

/// Voortgangsmelding.
pub enum Progress<'a> {
    Start {
        total: usize,
    },
    Sheet {
        done: usize,
        total: usize,
        diff: &'a SheetDiff,
    },
}

/// Normaliseert een boekjesnaam, zodat "TO+_wijzA_gevels" en "PVG_TO+_gevels" op elkaar
/// passen: neemt het deel na de laatste '_'-groep die in beide voorkomt is te fragiel, dus
/// vergelijk op het laatste woord(en) na het eerste '_'-blok met een letter-cijfercode.
fn booklet_tail(name: &str) -> String {
    name.rsplit_once('_')
        .map(|(_, t)| t.to_lowercase())
        .unwrap_or_else(|| name.to_lowercase())
}

/// Waar de bladvergelijkingen draaien.
pub enum Runner<'a> {
    /// In dit proces, een blad tegelijk (PDFium is per proces geserialiseerd).
    InProcess,
    /// Over werkerprocessen van `exe` (`exe worker <job>`), zie [`crate::parallel`].
    Processes { exe: &'a Path, workers: usize },
}

pub fn compare_issues(
    a: &Issue,
    b: &Issue,
    cfg: &Config,
    progress: impl FnMut(Progress),
) -> Result<IssueDiff> {
    compare_issues_with(a, b, cfg, Runner::InProcess, progress)
}

pub fn compare_issues_with(
    a: &Issue,
    b: &Issue,
    cfg: &Config,
    runner: Runner,
    mut progress: impl FnMut(Progress),
) -> Result<IssueDiff> {
    let mut sheets = Vec::new();
    let keys: std::collections::BTreeSet<&String> =
        a.sheets.keys().chain(b.sheets.keys()).collect();
    let total = keys.len();
    progress(Progress::Start { total });
    let mut warnings: Vec<String> = a
        .warnings
        .iter()
        .map(|w| format!("A: {w}"))
        .chain(b.warnings.iter().map(|w| format!("B: {w}")))
        .collect();
    let mut done = 0usize;
    let mut finish =
        |d: SheetDiff, sheets: &mut Vec<SheetDiff>, progress: &mut dyn FnMut(Progress)| {
            done += 1;
            progress(Progress::Sheet {
                done,
                total,
                diff: &d,
            });
            let mut d = d;
            d.mask = None; // geheugen: grove maskers niet voor de hele set bewaren
            sheets.push(d);
        };
    if total == 0 {
        warnings.push("GEEN BLADEN gevonden in A en B (alles tekeninglijst, of de nummer-regex past niet): niets vergeleken".into());
    }
    let mut pairs = Vec::new();
    for k in keys {
        match (a.sheets.get(k), b.sheets.get(k)) {
            (Some(sa), Some(sb)) => pairs.push((sa.clone(), sb.clone())),
            (Some(sa), None) => finish(
                only(k, sa, Verdict::RemovedFromB, cfg, true),
                &mut sheets,
                &mut progress,
            ),
            (None, Some(sb)) => finish(
                only(k, sb, Verdict::AddedInB, cfg, false),
                &mut sheets,
                &mut progress,
            ),
            (None, None) => unreachable!("sleutel komt uit A of B"),
        }
    }
    match runner {
        Runner::InProcess => {
            for (sa, sb) in &pairs {
                let d = match compare_sheet(sa, sb, cfg, None) {
                    Ok(d) => d,
                    Err(e) => SheetDiff {
                        key: sa.key.clone(),
                        title: sa.title.clone(),
                        verdict: Verdict::NotComparable,
                        reasons: vec![format!("fout: {e:#}")],
                        a: Some(sa.into()),
                        b: Some(sb.into()),
                        registration: None,
                        dpi: cfg.compare.dpi,
                        removed_px: 0,
                        added_px: 0,
                        pixel_pct: 0.0,
                        regions: vec![],
                        text: vec![],
                        seconds: 0.0,
                        vector: None,
                        mask: None,
                    },
                };
                finish(d, &mut sheets, &mut progress);
            }
        }
        Runner::Processes { exe, workers } => {
            crate::parallel::compare_pairs(pairs, cfg, exe, workers, |d| {
                finish(d, &mut sheets, &mut progress)
            })?;
        }
    }
    sheets.sort_by(|x, y| x.verdict.cmp(&y.verdict).then(x.key.cmp(&y.key)));

    // boekjes vergelijken op naamstaart
    let mut booklet_changes = Vec::new();
    let b_by_tail: std::collections::BTreeMap<String, &Vec<String>> = b
        .booklets
        .iter()
        .map(|(n, v)| (booklet_tail(n), v))
        .collect();
    for (name, keys_a) in &a.booklets {
        let tail = booklet_tail(name);
        match b_by_tail.get(&tail) {
            Some(keys_b) => {
                let only_a: Vec<String> = keys_a
                    .iter()
                    .filter(|k| !keys_b.contains(k))
                    .cloned()
                    .collect();
                let only_b: Vec<String> = keys_b
                    .iter()
                    .filter(|k| !keys_a.contains(k))
                    .cloned()
                    .collect();
                if !only_a.is_empty() || !only_b.is_empty() {
                    booklet_changes.push((name.clone(), only_a, only_b));
                }
            }
            None => warnings.push(format!("boekje '{name}' heeft geen tegenhanger in B")),
        }
    }
    Ok(IssueDiff {
        a_root: a.root.clone(),
        b_root: b.root.clone(),
        sheets,
        booklet_changes,
        warnings,
        config: cfg.clone(),
    })
}

fn only(k: &str, s: &SheetRef, v: Verdict, cfg: &Config, in_a: bool) -> SheetDiff {
    SheetDiff {
        key: k.to_string(),
        title: s.title.clone(),
        verdict: v,
        reasons: vec![],
        a: if in_a { Some(s.into()) } else { None },
        b: if in_a { None } else { Some(s.into()) },
        registration: None,
        dpi: cfg.compare.dpi,
        removed_px: 0,
        added_px: 0,
        pixel_pct: 0.0,
        regions: vec![],
        text: vec![],
        seconds: 0.0,
        vector: None,
        mask: None,
    }
}

/// Handig voor de CLI: vergelijk twee losse pagina's.
pub fn compare_pages(
    file_a: &Path,
    page_a: usize,
    file_b: &Path,
    page_b: usize,
    cfg: &Config,
) -> Result<SheetDiff> {
    let mk = |f: &Path, p: usize| -> Result<SheetRef> {
        let d = Doc::open(f)?;
        let geom = d.geom(p)?;
        Ok(SheetRef {
            key: format!(
                "{}#{}",
                f.file_stem().unwrap_or_default().to_string_lossy(),
                p + 1
            ),
            number: None,
            title: String::new(),
            file: f.to_path_buf(),
            page: p,
            geom,
            method: crate::identify::IdMethod::None,
            text_runs: 0,
            booklets: vec![],
            signature: 0,
        })
    };
    compare_sheet(&mk(file_a, page_a)?, &mk(file_b, page_b)?, cfg, None)
}

pub fn mm(pt: f64) -> f64 {
    pt * MM_PER_PT
}
