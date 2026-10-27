//! Vectorlaag: paden uit de PDF lezen (ook binnen Form XObjects) en geometrisch vergelijken.
//!
//! Vergelijken op dekking in plaats van op segmentgelijkheid: twee prints van dezelfde
//! tekening knippen lijnen verschillend op en tekenen in andere volgorde. Elk segment wordt
//! daarom bemonsterd (elke `step_mm`) en een monsterpunt telt als gedekt als er binnen
//! `tol_mm` een segment van de andere kant ligt. Een segment met te weinig dekking is
//! verwijderd (A) of toegevoegd (B). Daarnaast: stijlverschil (lijndikte/kleur) bij gelijke
//! geometrie.

use crate::geom::{Affine, PageGeom, RectPt, PT_PER_MM};
use anyhow::{anyhow, Result};
use pdfium_render::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Seg {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VPath {
    /// Segmenten in weergavepunten (bogen afgevlakt).
    pub segs: Vec<Seg>,
    pub stroked: bool,
    pub filled: bool,
    /// Lijndikte in punten (na transformatie).
    pub width: f32,
    pub stroke: [u8; 3],
    pub fill: [u8; 3],
    pub bbox: RectPt,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct VectorStats {
    pub paths: usize,
    pub segments: usize,
    pub forms: usize,
    pub images: usize,
    pub texts: usize,
    pub shadings: usize,
}

fn to_affine(m: &PdfMatrix) -> Affine {
    Affine {
        a: m.a() as f64,
        b: m.b() as f64,
        c: m.c() as f64,
        d: m.d() as f64,
        e: m.e() as f64,
        f: m.f() as f64,
    }
}

fn color(c: Result<PdfColor, PdfiumError>) -> [u8; 3] {
    c.map(|c| [c.red(), c.green(), c.blue()])
        .unwrap_or([0, 0, 0])
}

/// Leest alle paden van een pagina in weergavepunten.
pub fn extract(page: &PdfPage, geom: &PageGeom) -> Result<(Vec<VPath>, VectorStats)> {
    let u2d = geom.user_to_display();
    let mut out = Vec::new();
    let mut st = VectorStats::default();
    let objs = page.objects();
    for obj in objs.iter() {
        walk(&obj, &Affine::IDENTITY, &u2d, &mut out, &mut st)?;
    }
    Ok((out, st))
}

fn walk(
    obj: &PdfPageObject,
    parent: &Affine,
    u2d: &Affine,
    out: &mut Vec<VPath>,
    st: &mut VectorStats,
) -> Result<()> {
    match obj {
        PdfPageObject::Path(p) => {
            let m = to_affine(&p.matrix().map_err(|e| anyhow!("{e}"))?);
            let t = m.then(parent).then(u2d);
            let mut segs = Vec::new();
            let mut cur: Option<(f64, f64)> = None;
            let mut start: Option<(f64, f64)> = None;
            let mut bez: Vec<(f64, f64)> = Vec::new();
            for s in p.segments().raw().iter() {
                let (x, y) = t.apply(s.x().value as f64, s.y().value as f64);
                match s.segment_type() {
                    PdfPathSegmentType::MoveTo => {
                        cur = Some((x, y));
                        start = Some((x, y));
                        bez.clear();
                    }
                    PdfPathSegmentType::LineTo => {
                        if let Some(c) = cur {
                            push(&mut segs, c, (x, y));
                        }
                        cur = Some((x, y));
                    }
                    PdfPathSegmentType::BezierTo => {
                        bez.push((x, y));
                        if bez.len() == 3 {
                            if let Some(c) = cur {
                                flatten(&mut segs, c, bez[0], bez[1], bez[2]);
                            }
                            cur = Some(bez[2]);
                            bez.clear();
                        }
                    }
                    PdfPathSegmentType::Unknown => {}
                }
                if s.is_close() {
                    if let (Some(c), Some(s0)) = (cur, start) {
                        push(&mut segs, c, s0);
                        cur = Some(s0);
                    }
                }
            }
            st.paths += 1;
            st.segments += segs.len();
            if segs.is_empty() {
                return Ok(());
            }
            let stroked = p.is_stroked().unwrap_or(true);
            let filled = !matches!(p.fill_mode(), Ok(PdfPathFillMode::None));
            let w = p.stroke_width().map(|w| w.value as f64).unwrap_or(0.0) * t.scale_factor();
            let mut bb = RectPt::new(
                segs[0].x0 as f64,
                segs[0].y0 as f64,
                segs[0].x0 as f64,
                segs[0].y0 as f64,
            );
            for s in &segs {
                bb = bb.union(&RectPt::new(
                    s.x0 as f64,
                    s.y0 as f64,
                    s.x1 as f64,
                    s.y1 as f64,
                ));
            }
            out.push(VPath {
                segs,
                stroked,
                filled,
                width: w as f32,
                stroke: color(p.stroke_color()),
                fill: color(p.fill_color()),
                bbox: bb,
            });
        }
        PdfPageObject::XObjectForm(f) => {
            st.forms += 1;
            let m = to_affine(&f.matrix().map_err(|e| anyhow!("{e}"))?);
            let t = m.then(parent);
            for child in f.iter() {
                walk(&child, &t, u2d, out, st)?;
            }
        }
        PdfPageObject::Image(_) => st.images += 1,
        PdfPageObject::Text(_) => st.texts += 1,
        PdfPageObject::Shading(_) => st.shadings += 1,
        _ => {}
    }
    Ok(())
}

fn push(v: &mut Vec<Seg>, a: (f64, f64), b: (f64, f64)) {
    if (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6 {
        return;
    }
    v.push(Seg {
        x0: a.0 as f32,
        y0: a.1 as f32,
        x1: b.0 as f32,
        y1: b.1 as f32,
    });
}

fn flatten(v: &mut Vec<Seg>, p0: (f64, f64), p1: (f64, f64), p2: (f64, f64), p3: (f64, f64)) {
    // aantal stappen naar lengte van het controlepolygoon (~1 mm per stap, 2..16)
    let len = |a: (f64, f64), b: (f64, f64)| ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
    let l = len(p0, p1) + len(p1, p2) + len(p2, p3);
    let n = ((l / PT_PER_MM).ceil() as usize).clamp(2, 16);
    let mut prev = p0;
    for i in 1..=n {
        let t = i as f64 / n as f64;
        let u = 1.0 - t;
        let x =
            u * u * u * p0.0 + 3.0 * u * u * t * p1.0 + 3.0 * u * t * t * p2.0 + t * t * t * p3.0;
        let y =
            u * u * u * p0.1 + 3.0 * u * u * t * p1.1 + 3.0 * u * t * t * p2.1 + t * t * t * p3.1;
        push(v, prev, (x, y));
        prev = (x, y);
    }
}

/// Ruimtelijke index over segmenten (uniform raster). Bij een zoekvraag worden alle cellen
/// geraakt die binnen de tolerantie liggen, dus geen emmergrens-fouten.
pub struct SegIndex<'a> {
    segs: Vec<(&'a Seg, usize)>,
    cell: f32,
    x0: f32,
    y0: f32,
    nx: usize,
    ny: usize,
    cells: Vec<Vec<u32>>,
}

impl<'a> SegIndex<'a> {
    pub fn new(paths: &'a [VPath], cell: f32) -> Self {
        let segs: Vec<(&Seg, usize)> = paths
            .iter()
            .enumerate()
            .flat_map(|(pi, p)| p.segs.iter().map(move |s| (s, pi)))
            .collect();
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for (s, _) in &segs {
            x0 = x0.min(s.x0.min(s.x1));
            y0 = y0.min(s.y0.min(s.y1));
            x1 = x1.max(s.x0.max(s.x1));
            y1 = y1.max(s.y0.max(s.y1));
        }
        if segs.is_empty() {
            (x0, y0, x1, y1) = (0.0, 0.0, 1.0, 1.0);
        }
        let nx = (((x1 - x0) / cell).ceil() as usize).max(1);
        let ny = (((y1 - y0) / cell).ceil() as usize).max(1);
        let mut cells = vec![Vec::new(); nx * ny];
        for (i, (s, _)) in segs.iter().enumerate() {
            let cx0 = (((s.x0.min(s.x1) - x0) / cell) as usize).min(nx - 1);
            let cx1 = (((s.x0.max(s.x1) - x0) / cell) as usize).min(nx - 1);
            let cy0 = (((s.y0.min(s.y1) - y0) / cell) as usize).min(ny - 1);
            let cy1 = (((s.y0.max(s.y1) - y0) / cell) as usize).min(ny - 1);
            // lange diagonalen: alleen cellen langs de lijn zou zuiniger zijn; voor tekeningen
            // (veel horizontaal/verticaal) is de bbox-vulling acceptabel, met een grens.
            if (cx1 - cx0 + 1) * (cy1 - cy0 + 1) > 4096 {
                let n =
                    ((s.x1 - s.x0).abs().max((s.y1 - s.y0).abs()) / (cell * 0.5)).ceil() as usize;
                for k in 0..=n {
                    let t = k as f32 / n.max(1) as f32;
                    let x = s.x0 + (s.x1 - s.x0) * t;
                    let y = s.y0 + (s.y1 - s.y0) * t;
                    let cx = (((x - x0) / cell) as usize).min(nx - 1);
                    let cy = (((y - y0) / cell) as usize).min(ny - 1);
                    let c = &mut cells[cy * nx + cx];
                    if c.last() != Some(&(i as u32)) {
                        c.push(i as u32);
                    }
                }
                continue;
            }
            for cy in cy0..=cy1 {
                for cx in cx0..=cx1 {
                    cells[cy * nx + cx].push(i as u32);
                }
            }
        }
        Self {
            segs,
            cell,
            x0,
            y0,
            nx,
            ny,
            cells,
        }
    }

    /// Dichtstbijzijnde segment binnen `tol` van (x, y): (afstand, padindex).
    pub fn near(&self, x: f32, y: f32, tol: f32) -> Option<(f32, usize)> {
        let cx0 = ((x - tol - self.x0) / self.cell).floor() as isize;
        let cx1 = ((x + tol - self.x0) / self.cell).floor() as isize;
        let cy0 = ((y - tol - self.y0) / self.cell).floor() as isize;
        let cy1 = ((y + tol - self.y0) / self.cell).floor() as isize;
        let mut best: Option<(f32, usize)> = None;
        for cy in cy0.max(0)..=cy1.min(self.ny as isize - 1) {
            for cx in cx0.max(0)..=cx1.min(self.nx as isize - 1) {
                for &i in &self.cells[cy as usize * self.nx + cx as usize] {
                    let (s, pi) = self.segs[i as usize];
                    let d = dist_point_seg(x, y, s);
                    if d <= tol && best.is_none_or(|b| d < b.0) {
                        best = Some((d, pi));
                    }
                }
            }
        }
        best
    }
}

impl SegIndex<'_> {
    /// Alle paden met een segment binnen `tol` van (x, y).
    pub fn near_all(&self, x: f32, y: f32, tol: f32, out: &mut Vec<usize>) {
        out.clear();
        let cx0 = ((x - tol - self.x0) / self.cell).floor() as isize;
        let cx1 = ((x + tol - self.x0) / self.cell).floor() as isize;
        let cy0 = ((y - tol - self.y0) / self.cell).floor() as isize;
        let cy1 = ((y + tol - self.y0) / self.cell).floor() as isize;
        for cy in cy0.max(0)..=cy1.min(self.ny as isize - 1) {
            for cx in cx0.max(0)..=cx1.min(self.nx as isize - 1) {
                for &i in &self.cells[cy as usize * self.nx + cx as usize] {
                    let (s, pi) = self.segs[i as usize];
                    if dist_point_seg(x, y, s) <= tol && !out.contains(&pi) {
                        out.push(pi);
                    }
                }
            }
        }
    }
}

pub fn dist_point_seg(px: f32, py: f32, s: &Seg) -> f32 {
    let (dx, dy) = (s.x1 - s.x0, s.y1 - s.y0);
    let l2 = dx * dx + dy * dy;
    let t = if l2 <= 0.0 {
        0.0
    } else {
        (((px - s.x0) * dx + (py - s.y0) * dy) / l2).clamp(0.0, 1.0)
    };
    let (qx, qy) = (s.x0 + t * dx, s.y0 + t * dy);
    ((px - qx).powi(2) + (py - qy).powi(2)).sqrt()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VChangeKind {
    Removed,
    Added,
    /// Zelfde geometrie, andere lijndikte of kleur.
    Style,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VChange {
    pub kind: VChangeKind,
    /// In A-weergavepunten.
    pub bbox: RectPt,
    /// Ongedekte segmenten (A-kader), voor tekenen als vectoroverlay.
    pub segs: Vec<Seg>,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VParams {
    pub tol_mm: f64,
    pub step_mm: f64,
    /// Segment telt als gedekt bij minstens deze fractie gedekte monsters.
    pub min_cover: f64,
    /// Lijndikteverschil dat als stijlwijziging telt (mm).
    pub width_tol_mm: f64,
}

impl Default for VParams {
    fn default() -> Self {
        Self {
            tol_mm: 0.15,
            step_mm: 0.5,
            min_cover: 0.8,
            width_tol_mm: 0.05,
        }
    }
}

/// (padindex, ongedekte segmenten)
type Uncovered = Vec<(usize, Vec<Seg>)>;
/// (padindex bron, padindex dekkend pad met andere stijl)
type StyleMismatch = Vec<(usize, usize)>;

/// Ongedekte segmenten van `src` t.o.v. `other` (beide al in hetzelfde kader), plus paden
/// waarvan de geometrie gedekt is maar geen enkel dekkend pad dezelfde stijl heeft.
fn uncovered(
    src: &[VPath],
    idx: &SegIndex,
    other: &[VPath],
    p: &VParams,
) -> (Uncovered, StyleMismatch) {
    use rayon::prelude::*;
    let tol = (p.tol_mm * PT_PER_MM) as f32;
    let step = (p.step_mm * PT_PER_MM) as f32;
    let wtol = (p.width_tol_mm * PT_PER_MM) as f32;
    let per: Vec<(usize, Vec<Seg>, Option<usize>)> = src
        .par_iter()
        .enumerate()
        .map(|(pi, path)| {
            let mut bad = Vec::new();
            for s in &path.segs {
                let len = ((s.x1 - s.x0).powi(2) + (s.y1 - s.y0).powi(2)).sqrt();
                let n = ((len / step).ceil() as usize).max(1);
                let mut hit = 0usize;
                for k in 0..=n {
                    let t = k as f32 / n as f32;
                    let (x, y) = (s.x0 + (s.x1 - s.x0) * t, s.y0 + (s.y1 - s.y0) * t);
                    if idx.near(x, y, tol).is_some() {
                        hit += 1;
                    }
                }
                if (hit as f64) < p.min_cover * (n + 1) as f64 {
                    bad.push(*s);
                }
            }
            let mut style = None;
            if bad.is_empty() && path.stroked {
                // stijl: midden van het langste segment; zijn er dekkende paden met dezelfde stijl?
                let s = path
                    .segs
                    .iter()
                    .max_by(|a, b| {
                        let la = (a.x1 - a.x0).hypot(a.y1 - a.y0);
                        let lb = (b.x1 - b.x0).hypot(b.y1 - b.y0);
                        la.total_cmp(&lb)
                    })
                    .expect("niet leeg");
                let mut cand = Vec::new();
                idx.near_all((s.x0 + s.x1) / 2.0, (s.y0 + s.y1) / 2.0, tol, &mut cand);
                let stroked: Vec<usize> = cand.into_iter().filter(|&o| other[o].stroked).collect();
                let same = stroked.iter().any(|&o| {
                    (other[o].width - path.width).abs() <= wtol && other[o].stroke == path.stroke
                });
                if !stroked.is_empty() && !same {
                    style = Some(stroked[0]);
                }
            }
            (pi, bad, style)
        })
        .collect();
    let mut res = Vec::new();
    let mut style = Vec::new();
    for (pi, bad, st) in per {
        if !bad.is_empty() {
            res.push((pi, bad));
        }
        if let Some(o) = st {
            style.push((pi, o));
        }
    }
    (res, style)
}

/// Vergelijkt paden. `b` wordt eerst met `b_to_a` in het A-kader gelegd.
pub fn compare(a: &[VPath], b: &[VPath], b_to_a: &Affine, p: &VParams) -> Vec<VChange> {
    let bt: Vec<VPath> = b
        .iter()
        .map(|vp| {
            let segs: Vec<Seg> = vp
                .segs
                .iter()
                .map(|s| {
                    let (x0, y0) = b_to_a.apply(s.x0 as f64, s.y0 as f64);
                    let (x1, y1) = b_to_a.apply(s.x1 as f64, s.y1 as f64);
                    Seg {
                        x0: x0 as f32,
                        y0: y0 as f32,
                        x1: x1 as f32,
                        y1: y1 as f32,
                    }
                })
                .collect();
            VPath {
                segs,
                bbox: vp.bbox.transform(b_to_a),
                width: (vp.width as f64 * b_to_a.scale_factor()) as f32,
                ..vp.clone()
            }
        })
        .collect();
    let cell = (2.0 * PT_PER_MM) as f32;
    let ia = SegIndex::new(a, cell);
    let ib = SegIndex::new(&bt, cell);
    let (rem, style) = uncovered(a, &ib, &bt, p);
    let (add, _) = uncovered(&bt, &ia, a, p);
    let mut out = Vec::new();
    let bb = |segs: &[Seg]| {
        let mut r = RectPt::new(
            segs[0].x0 as f64,
            segs[0].y0 as f64,
            segs[0].x1 as f64,
            segs[0].y1 as f64,
        );
        for s in segs {
            r = r.union(&RectPt::new(
                s.x0 as f64,
                s.y0 as f64,
                s.x1 as f64,
                s.y1 as f64,
            ));
        }
        r
    };
    for (_, segs) in rem {
        out.push(VChange {
            kind: VChangeKind::Removed,
            bbox: bb(&segs),
            segs,
            note: String::new(),
        });
    }
    for (_, segs) in add {
        out.push(VChange {
            kind: VChangeKind::Added,
            bbox: bb(&segs),
            segs,
            note: String::new(),
        });
    }
    for (pa, pb) in style {
        let (x, y) = (&a[pa], &bt[pb]);
        out.push(VChange {
            kind: VChangeKind::Style,
            bbox: x.bbox,
            segs: x.segs.clone(),
            note: format!(
                "lijndikte {:.2} -> {:.2} mm, kleur {:?} -> {:?}",
                x.width as f64 / PT_PER_MM,
                y.width as f64 / PT_PER_MM,
                x.stroke,
                y.stroke
            ),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(x0: f32, y0: f32, x1: f32, y1: f32) -> VPath {
        VPath {
            segs: vec![Seg { x0, y0, x1, y1 }],
            stroked: true,
            filled: false,
            width: 0.5,
            stroke: [0, 0, 0],
            fill: [0, 0, 0],
            bbox: RectPt::new(x0 as f64, y0 as f64, x1 as f64, y1 as f64),
        }
    }

    #[test]
    fn differently_split_lines_are_equal() {
        let a = vec![line(0.0, 0.0, 300.0, 0.0)];
        let b = vec![line(0.0, 0.0, 120.0, 0.0), line(120.0, 0.0, 300.0, 0.0)];
        assert!(compare(&a, &b, &Affine::IDENTITY, &VParams::default()).is_empty());
    }

    #[test]
    fn extended_line_and_new_line() {
        let a = vec![line(0.0, 0.0, 100.0, 0.0)];
        let b = vec![line(0.0, 0.0, 150.0, 0.0), line(0.0, 50.0, 100.0, 50.0)];
        let d = compare(&a, &b, &Affine::IDENTITY, &VParams::default());
        assert_eq!(d.iter().filter(|c| c.kind == VChangeKind::Added).count(), 2);
        assert_eq!(
            d.iter().filter(|c| c.kind == VChangeKind::Removed).count(),
            0
        );
    }

    #[test]
    fn width_change_is_style() {
        let a = vec![line(0.0, 0.0, 100.0, 0.0)];
        let mut l = line(0.0, 0.0, 100.0, 0.0);
        l.width = 2.0;
        let d = compare(&a, &[l], &Affine::IDENTITY, &VParams::default());
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].kind, VChangeKind::Style);
    }
}
