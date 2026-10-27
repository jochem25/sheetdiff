//! Geometrie: rechthoeken en affiene transformaties in weergavecoordinaten.
//!
//! Conventie in de hele crate: **weergavepunten** = PDF-punten (1/72 inch) na toepassen van
//! /Rotate, oorsprong linksboven van de CropBox, y naar beneden. Pixels = punten * dpi / 72.

use serde::{Deserialize, Serialize};

pub const MM_PER_PT: f64 = 25.4 / 72.0;
pub const PT_PER_MM: f64 = 72.0 / 25.4;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RectPt {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl RectPt {
    pub fn new(x0: f64, y0: f64, x1: f64, y1: f64) -> Self {
        Self {
            x0: x0.min(x1),
            y0: y0.min(y1),
            x1: x0.max(x1),
            y1: y0.max(y1),
        }
    }
    pub fn width(&self) -> f64 {
        self.x1 - self.x0
    }
    pub fn height(&self) -> f64 {
        self.y1 - self.y0
    }
    pub fn area(&self) -> f64 {
        self.width().max(0.0) * self.height().max(0.0)
    }
    pub fn center(&self) -> (f64, f64) {
        ((self.x0 + self.x1) / 2.0, (self.y0 + self.y1) / 2.0)
    }
    pub fn union(&self, o: &RectPt) -> RectPt {
        RectPt::new(
            self.x0.min(o.x0),
            self.y0.min(o.y0),
            self.x1.max(o.x1),
            self.y1.max(o.y1),
        )
    }
    pub fn intersects(&self, o: &RectPt) -> bool {
        self.x0 <= o.x1 && o.x0 <= self.x1 && self.y0 <= o.y1 && o.y0 <= self.y1
    }
    pub fn intersection_area(&self, o: &RectPt) -> f64 {
        let w = self.x1.min(o.x1) - self.x0.max(o.x0);
        let h = self.y1.min(o.y1) - self.y0.max(o.y0);
        if w <= 0.0 || h <= 0.0 {
            0.0
        } else {
            w * h
        }
    }
    pub fn expand(&self, d: f64) -> RectPt {
        RectPt::new(self.x0 - d, self.y0 - d, self.x1 + d, self.y1 + d)
    }
    pub fn contains(&self, o: &RectPt) -> bool {
        o.x0 >= self.x0 && o.x1 <= self.x1 && o.y0 >= self.y0 && o.y1 <= self.y1
    }
    /// Kortste afstand tussen twee rechthoeken (0 bij overlap).
    pub fn distance(&self, o: &RectPt) -> f64 {
        let dx = (o.x0 - self.x1).max(self.x0 - o.x1).max(0.0);
        let dy = (o.y0 - self.y1).max(self.y0 - o.y1).max(0.0);
        (dx * dx + dy * dy).sqrt()
    }
    pub fn transform(&self, t: &Affine) -> RectPt {
        let pts = [
            t.apply(self.x0, self.y0),
            t.apply(self.x1, self.y0),
            t.apply(self.x0, self.y1),
            t.apply(self.x1, self.y1),
        ];
        let mut r = RectPt::new(pts[0].0, pts[0].1, pts[0].0, pts[0].1);
        for p in &pts[1..] {
            r = r.union(&RectPt::new(p.0, p.1, p.0, p.1));
        }
        r
    }
}

/// Deel van `r` dat door de vereniging van `boxes` gedekt wordt, bemonsterd op een raster
/// van n x n punten. Overlappende boxen tellen zo maar een keer (een som van snijvlakken
/// telt dubbel waar boxen overlappen).
pub fn covered_fraction(r: &RectPt, boxes: &[RectPt], n: usize) -> f64 {
    if boxes.is_empty() || r.area() <= 0.0 {
        return 0.0;
    }
    let mut hit = 0usize;
    for j in 0..n {
        for i in 0..n {
            let x = r.x0 + (i as f64 + 0.5) / n as f64 * r.width();
            let y = r.y0 + (j as f64 + 0.5) / n as f64 * r.height();
            if boxes
                .iter()
                .any(|b| x >= b.x0 && x <= b.x1 && y >= b.y0 && y <= b.y1)
            {
                hit += 1;
            }
        }
    }
    hit as f64 / (n * n) as f64
}

/// Affiene transformatie: x' = a*x + c*y + e, y' = b*x + d*y + f.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Affine {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl Default for Affine {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Affine {
    pub const IDENTITY: Affine = Affine {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    pub fn translate(tx: f64, ty: f64) -> Self {
        Affine {
            e: tx,
            f: ty,
            ..Self::IDENTITY
        }
    }
    pub fn scale(s: f64) -> Self {
        Affine {
            a: s,
            d: s,
            ..Self::IDENTITY
        }
    }
    pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }
    /// `self.then(o)` = eerst self, daarna o.
    pub fn then(&self, o: &Affine) -> Affine {
        Affine {
            a: o.a * self.a + o.c * self.b,
            b: o.b * self.a + o.d * self.b,
            c: o.a * self.c + o.c * self.d,
            d: o.b * self.c + o.d * self.d,
            e: o.a * self.e + o.c * self.f + o.e,
            f: o.b * self.e + o.d * self.f + o.f,
        }
    }
    pub fn determinant(&self) -> f64 {
        self.a * self.d - self.b * self.c
    }
    pub fn inverse(&self) -> Option<Affine> {
        let det = self.determinant();
        if det.abs() < 1e-12 {
            return None;
        }
        let a = self.d / det;
        let b = -self.b / det;
        let c = -self.c / det;
        let d = self.a / det;
        let e = -(a * self.e + c * self.f);
        let f = -(b * self.e + d * self.f);
        Some(Affine { a, b, c, d, e, f })
    }
    /// Gemiddelde lineaire schaalfactor.
    pub fn scale_factor(&self) -> f64 {
        self.determinant().abs().sqrt()
    }
    /// Rotatie in graden (0/90/180/270 bij de transformaties die deze tool maakt).
    pub fn rotation_deg(&self) -> f64 {
        let r = self.b.atan2(self.a).to_degrees();
        if r < 0.0 {
            r + 360.0
        } else {
            r
        }
    }
}

/// Rotatie over een veelvoud van 90 graden (met de klok mee in weergave).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Rot90 {
    #[default]
    R0,
    R90,
    R180,
    R270,
}

impl Rot90 {
    pub const ALL: [Rot90; 4] = [Rot90::R0, Rot90::R90, Rot90::R180, Rot90::R270];

    pub fn from_degrees(deg: i64) -> Rot90 {
        match deg.rem_euclid(360) {
            90 => Rot90::R90,
            180 => Rot90::R180,
            270 => Rot90::R270,
            _ => Rot90::R0,
        }
    }
    pub fn degrees(self) -> i64 {
        match self {
            Rot90::R0 => 0,
            Rot90::R90 => 90,
            Rot90::R180 => 180,
            Rot90::R270 => 270,
        }
    }
    /// Transformatie die een pagina van `w` x `h` met de klok mee draait; resultaat ligt weer
    /// met linksboven op (0,0).
    pub fn affine(self, w: f64, h: f64) -> Affine {
        match self {
            Rot90::R0 => Affine::IDENTITY,
            // (x,y) -> (h - y, x)
            Rot90::R90 => Affine {
                a: 0.0,
                b: 1.0,
                c: -1.0,
                d: 0.0,
                e: h,
                f: 0.0,
            },
            Rot90::R180 => Affine {
                a: -1.0,
                b: 0.0,
                c: 0.0,
                d: -1.0,
                e: w,
                f: h,
            },
            // (x,y) -> (y, w - x)
            Rot90::R270 => Affine {
                a: 0.0,
                b: -1.0,
                c: 1.0,
                d: 0.0,
                e: 0.0,
                f: w,
            },
        }
    }
}

/// Geometrie van een pagina: hoe PDF-gebruikersruimte naar weergavepunten gaat.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PageGeom {
    /// CropBox in gebruikersruimte [x0, y0, x1, y1].
    pub crop: [f64; 4],
    /// /Rotate van de pagina.
    pub rotate: Rot90,
}

impl PageGeom {
    pub fn unrotated_size(&self) -> (f64, f64) {
        (
            (self.crop[2] - self.crop[0]).abs(),
            (self.crop[3] - self.crop[1]).abs(),
        )
    }
    /// Afmeting in weergavepunten (na rotatie).
    pub fn display_size(&self) -> (f64, f64) {
        let (w, h) = self.unrotated_size();
        match self.rotate {
            Rot90::R0 | Rot90::R180 => (w, h),
            _ => (h, w),
        }
    }
    /// Gebruikersruimte (y omhoog) -> weergavepunten.
    pub fn user_to_display(&self) -> Affine {
        let x0 = self.crop[0].min(self.crop[2]);
        let y1 = self.crop[1].max(self.crop[3]);
        // eerst: (x,y) -> (x - x0, y1 - y)
        let flip = Affine {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: -1.0,
            e: -x0,
            f: y1,
        };
        let (w, h) = self.unrotated_size();
        flip.then(&self.rotate.affine(w, h))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9
    }

    #[test]
    fn affine_inverse_roundtrip() {
        let t = Affine::scale(1.01)
            .then(&Rot90::R90.affine(100.0, 50.0))
            .then(&Affine::translate(3.0, -2.0));
        let inv = t.inverse().unwrap();
        let p = t.apply(12.0, 34.0);
        assert!(close(inv.apply(p.0, p.1), (12.0, 34.0)));
    }

    #[test]
    fn rot90_corners() {
        // pagina 100 breed, 50 hoog; na 90 graden rechtsom is hij 50 breed, 100 hoog
        let r = Rot90::R90.affine(100.0, 50.0);
        // linksonder (0,50) -> linksboven (0,0)
        assert!(close(r.apply(0.0, 50.0), (0.0, 0.0)));
        // linksboven (0,0) -> rechtsboven (50,0)
        assert!(close(r.apply(0.0, 0.0), (50.0, 0.0)));
        let r270 = Rot90::R270.affine(100.0, 50.0);
        assert!(close(r270.apply(100.0, 0.0), (0.0, 0.0)));
    }

    #[test]
    fn user_to_display_unrotated() {
        let g = PageGeom {
            crop: [0.0, 0.0, 1190.6, 841.9],
            rotate: Rot90::R0,
        };
        // pymupdf-meting: Rect y=140.8762 (gebruiker) -> 701.0 (weergave)
        let (_, y) = g.user_to_display().apply(820.0, 140.8762);
        assert!((y - 701.0238).abs() < 1e-3);
    }

    #[test]
    fn overlapping_boxes_count_once() {
        // regio half op het stempel; A en B hebben hun stempel op dezelfde plek
        let r = RectPt::new(0.0, 0.0, 100.0, 10.0);
        let stamp = RectPt::new(0.0, 0.0, 50.0, 10.0);
        let f = covered_fraction(&r, &[stamp, stamp], 24);
        assert!((f - 0.5).abs() < 0.05, "{f}");
    }

    #[test]
    fn rect_distance() {
        let a = RectPt::new(0.0, 0.0, 10.0, 10.0);
        let b = RectPt::new(13.0, 14.0, 20.0, 20.0);
        assert!((a.distance(&b) - 5.0).abs() < 1e-9);
        assert_eq!(a.distance(&a), 0.0);
    }
}
