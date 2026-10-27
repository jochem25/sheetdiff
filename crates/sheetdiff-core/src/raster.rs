//! Rasterbewerkingen op grijsbeelden: minimumfilter, pixeldiff met anti-aliasingtolerantie,
//! verkleinen en verbonden componenten.

use crate::pdf::Gray;
use rayon::prelude::*;

/// Diff-klasse per pixel (bitvlaggen).
pub const REMOVED: u8 = 1; // inkt in A die in B (binnen de straal) ontbreekt
pub const ADDED: u8 = 2; // inkt in B die in A (binnen de straal) ontbreekt

/// Minimumfilter met een vierkant venster van (2r+1)^2 (grijs: donkerste waarde in de buurt).
/// Dat is een dilatatie van de inkt.
#[allow(clippy::needless_range_loop)] // indexen lopen over twee buffers tegelijk
pub fn min_filter(src: &Gray, r: usize) -> Gray {
    if r == 0 {
        return src.clone();
    }
    let (w, h) = (src.w, src.h);
    // horizontaal
    let mut tmp = vec![255u8; w * h];
    tmp.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let s = &src.data[y * w..(y + 1) * w];
        for x in 0..w {
            let lo = x.saturating_sub(r);
            let hi = (x + r).min(w - 1);
            let mut m = 255u8;
            for &v in &s[lo..=hi] {
                m = m.min(v);
            }
            row[x] = m;
        }
    });
    // verticaal
    let mut out = vec![255u8; w * h];
    out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let lo = y.saturating_sub(r);
        let hi = (y + r).min(h - 1);
        row.copy_from_slice(&tmp[lo * w..(lo + 1) * w]);
        for yy in lo + 1..=hi {
            let t = &tmp[yy * w..(yy + 1) * w];
            for x in 0..w {
                row[x] = row[x].min(t[x]);
            }
        }
    });
    Gray { w, h, data: out }
}

/// Instellingen voor de pixeldiff.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DiffParams {
    /// Een pixel telt als verschil als hij minstens zoveel grijswaarden donkerder is dan het
    /// donkerste punt in de buurt van de andere kant (kleurtolerantie, 0-255).
    pub tolerance: u8,
    /// Buurtstraal in pixels (minimale lijnafstand): een lijn die minder dan dit verschuift
    /// telt niet als verschil. Vangt anti-aliasing en subpixelverschuiving op.
    pub radius_px: usize,
}

impl Default for DiffParams {
    fn default() -> Self {
        Self {
            tolerance: 64,
            radius_px: 1,
        }
    }
}

/// Resultaat van een pixeldiff op een (deel)beeld.
pub struct DiffMap {
    pub w: usize,
    pub h: usize,
    /// Per pixel 0 / REMOVED / ADDED / beide.
    pub class: Vec<u8>,
    pub removed: u64,
    pub added: u64,
}

/// Pixeldiff tussen twee even grote, uitgelijnde grijsbeelden.
///
/// removed(p) = A(p) < min_r(B)(p) - tol  -> A heeft hier inkt die in B nergens in de buurt is
/// added(p)   = B(p) < min_r(A)(p) - tol  -> omgekeerd
#[allow(clippy::needless_range_loop)]
pub fn diff(a: &Gray, b: &Gray, p: &DiffParams) -> DiffMap {
    assert_eq!((a.w, a.h), (b.w, b.h), "diff vereist gelijke afmetingen");
    let amin = min_filter(a, p.radius_px);
    let bmin = min_filter(b, p.radius_px);
    let tol = p.tolerance as i16;
    let w = a.w;
    let mut class = vec![0u8; a.w * a.h];
    let (removed, added) = class
        .par_chunks_mut(w)
        .enumerate()
        .map(|(y, row)| {
            let mut r = 0u64;
            let mut ad = 0u64;
            let o = y * w;
            for x in 0..w {
                let i = o + x;
                let mut c = 0u8;
                if (a.data[i] as i16) < bmin.data[i] as i16 - tol {
                    c |= REMOVED;
                    r += 1;
                }
                if (b.data[i] as i16) < amin.data[i] as i16 - tol {
                    c |= ADDED;
                    ad += 1;
                }
                row[x] = c;
            }
            (r, ad)
        })
        .reduce(|| (0, 0), |x, y| (x.0 + y.0, x.1 + y.1));
    DiffMap {
        w: a.w,
        h: a.h,
        class,
        removed,
        added,
    }
}

/// Grof raster: per blok van `k`x`k` pixels het aantal verschilpixels per klasse.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct CoarseMask {
    pub k: usize,
    pub w: usize,
    pub h: usize,
    /// aantal REMOVED-pixels per blok (verzadigd op u16::MAX)
    pub removed: Vec<u16>,
    pub added: Vec<u16>,
}

impl std::fmt::Debug for CoarseMask {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CoarseMask({}x{}, k={})", self.w, self.h, self.k)
    }
}

impl CoarseMask {
    pub fn new(full_w: usize, full_h: usize, k: usize) -> Self {
        let w = full_w.div_ceil(k);
        let h = full_h.div_ceil(k);
        Self {
            k,
            w,
            h,
            removed: vec![0; w * h],
            added: vec![0; w * h],
        }
    }
    /// Voegt een strook diff toe die op rij `y0` van het volle beeld begint.
    pub fn accumulate(&mut self, d: &DiffMap, y0: usize) {
        for y in 0..d.h {
            let cy = (y0 + y) / self.k;
            if cy >= self.h {
                break;
            }
            let row = &d.class[y * d.w..(y + 1) * d.w];
            for (x, &c) in row.iter().enumerate() {
                if c == 0 {
                    continue;
                }
                let ci = cy * self.w + x / self.k;
                if c & REMOVED != 0 {
                    self.removed[ci] = self.removed[ci].saturating_add(1);
                }
                if c & ADDED != 0 {
                    self.added[ci] = self.added[ci].saturating_add(1);
                }
            }
        }
    }
    pub fn total(&self, i: usize) -> u32 {
        self.removed[i] as u32 + self.added[i] as u32
    }
}

/// Een verschilregio in grove-rastercellen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellRegion {
    pub cx0: usize,
    pub cy0: usize,
    pub cx1: usize, // inclusief
    pub cy1: usize, // inclusief
    pub removed: u64,
    pub added: u64,
}

/// Verbonden componenten op het grove raster. Cellen met minstens `min_cell_px` verschilpixels
/// doen mee; cellen binnen `gap_cells` van elkaar worden samengevoegd (union-find op de
/// buurt, dus geen emmergrenzen).
pub fn regions(mask: &CoarseMask, min_cell_px: u32, gap_cells: usize) -> Vec<CellRegion> {
    let (w, h) = (mask.w, mask.h);
    let active: Vec<usize> = (0..w * h)
        .filter(|&i| mask.total(i) >= min_cell_px)
        .collect();
    if active.is_empty() {
        return Vec::new();
    }
    let mut idx_of = vec![usize::MAX; w * h];
    for (n, &i) in active.iter().enumerate() {
        idx_of[i] = n;
    }
    let mut parent: Vec<usize> = (0..active.len()).collect();
    fn find(p: &mut [usize], mut x: usize) -> usize {
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }
    let g = gap_cells as isize;
    for (n, &i) in active.iter().enumerate() {
        let (x, y) = ((i % w) as isize, (i / w) as isize);
        for dy in 0..=g {
            for dx in -g..=g {
                if dy == 0 && dx <= 0 {
                    continue;
                }
                let (nx, ny) = (x + dx, y + dy);
                if nx < 0 || ny < 0 || nx >= w as isize || ny >= h as isize {
                    continue;
                }
                let j = idx_of[ny as usize * w + nx as usize];
                if j != usize::MAX {
                    let (ra, rb) = (find(&mut parent, n), find(&mut parent, j));
                    if ra != rb {
                        parent[ra.max(rb)] = ra.min(rb);
                    }
                }
            }
        }
    }
    let mut map: std::collections::BTreeMap<usize, CellRegion> = Default::default();
    for (n, &i) in active.iter().enumerate() {
        let root = find(&mut parent, n);
        let (x, y) = (i % w, i / w);
        let e = map.entry(root).or_insert(CellRegion {
            cx0: x,
            cy0: y,
            cx1: x,
            cy1: y,
            removed: 0,
            added: 0,
        });
        e.cx0 = e.cx0.min(x);
        e.cy0 = e.cy0.min(y);
        e.cx1 = e.cx1.max(x);
        e.cy1 = e.cy1.max(y);
        e.removed += mask.removed[i] as u64;
        e.added += mask.added[i] as u64;
    }
    map.into_values().collect()
}

/// Verkleint een grijsbeeld met factor `k` (minimum per blok, zodat dunne lijnen blijven).
pub fn downsample_min(src: &Gray, k: usize) -> Gray {
    let w = src.w.div_ceil(k);
    let h = src.h.div_ceil(k);
    let mut out = vec![255u8; w * h];
    out.par_chunks_mut(w).enumerate().for_each(|(cy, row)| {
        for yy in cy * k..((cy + 1) * k).min(src.h) {
            let s = &src.data[yy * src.w..(yy + 1) * src.w];
            for (x, &v) in s.iter().enumerate() {
                let c = &mut row[x / k];
                if v < *c {
                    *c = v;
                }
            }
        }
    });
    Gray { w, h, data: out }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas(w: usize, h: usize) -> Gray {
        Gray::new(w, h, 255)
    }
    fn hline(g: &mut Gray, y: usize, x0: usize, x1: usize, v: u8) {
        for x in x0..x1 {
            g.data[y * g.w + x] = v;
        }
    }

    #[test]
    fn identical_gives_no_diff() {
        let mut a = canvas(50, 50);
        hline(&mut a, 10, 5, 45, 0);
        let d = diff(&a, &a.clone(), &DiffParams::default());
        assert_eq!((d.removed, d.added), (0, 0));
    }

    #[test]
    fn one_pixel_shift_is_tolerated_but_two_is_not() {
        let mut a = canvas(50, 50);
        hline(&mut a, 10, 5, 45, 0);
        let mut b1 = canvas(50, 50);
        hline(&mut b1, 11, 5, 45, 0);
        let d = diff(&a, &b1, &DiffParams::default());
        assert_eq!((d.removed, d.added), (0, 0));
        let mut b3 = canvas(50, 50);
        hline(&mut b3, 13, 5, 45, 0);
        let d = diff(&a, &b3, &DiffParams::default());
        assert_eq!((d.removed, d.added), (40, 40));
    }

    #[test]
    fn antialias_grey_below_tolerance_is_ignored() {
        let mut a = canvas(20, 20);
        hline(&mut a, 5, 2, 18, 200);
        let b = canvas(20, 20);
        // 255 - 200 = 55 < 64
        let d = diff(&a, &b, &DiffParams::default());
        assert_eq!(d.removed, 0);
    }

    #[test]
    fn added_line_and_regions() {
        let a = canvas(100, 100);
        let mut b = canvas(100, 100);
        hline(&mut b, 20, 10, 30, 0);
        hline(&mut b, 80, 70, 90, 0);
        let d = diff(&a, &b, &DiffParams::default());
        assert_eq!(d.added, 40);
        let mut m = CoarseMask::new(100, 100, 4);
        m.accumulate(&d, 0);
        let r = regions(&m, 1, 1);
        assert_eq!(r.len(), 2);
        assert!(r.iter().all(|r| r.added == 20 && r.removed == 0));
        // met een grote samenvoegafstand worden het er een
        let r = regions(&m, 1, 20);
        assert_eq!(r.len(), 1);
    }
}
