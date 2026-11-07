//! Registratie: vind de transformatie die blad B (weergavepunten) op blad A legt.
//!
//! Aanpak (grof -> fijn, op projectieprofielen):
//! 1. Grof (25 dpi): per rotatiekandidaat B renderen in het kader van A, dan schaal en
//!    verschuiving zoeken op de rij- en kolomprofielen van de inktdichtheid (1D, goedkoop).
//! 2. Fijn (100 dpi): opnieuw renderen met de grove transformatie en schaal/verschuiving
//!    verfijnen, met subpixelinterpolatie.
//! 3. Kwaliteit: overlap van de inkt (A in B-buurt en B in A-buurt) op 100 dpi.
//!
//! Een mislukte registratie (lage overlap) wordt als fout teruggegeven, nooit stil geslikt.

use crate::geom::{Affine, PageGeom, Rot90, MM_PER_PT};
use crate::pdf::{self, Gray};
use anyhow::Result;
use pdfium_render::prelude::PdfPage;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegParams {
    pub coarse_dpi: f64,
    pub fine_dpi: f64,
    /// Maximale schaalafwijking die gezocht wordt (0.03 = 3 %).
    pub max_scale_dev: f64,
    /// Maximale verschuiving in mm.
    pub max_shift_mm: f64,
    /// Ook 180 graden proberen bij gelijke orientatie (kost een render extra).
    pub try_180: bool,
    /// Onder deze overlapscore geldt de registratie als mislukt.
    pub min_score: f64,
    /// Identiteit wint als haar overlap hooguit zoveel lager is dan de gevonden transformatie.
    pub identity_margin: f64,
}

impl Default for RegParams {
    fn default() -> Self {
        Self {
            coarse_dpi: 25.0,
            fine_dpi: 100.0,
            max_scale_dev: 0.03,
            max_shift_mm: 40.0,
            try_180: false,
            min_score: 0.6,
            identity_margin: 0.002,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Registration {
    /// B-weergavepunten -> A-weergavepunten.
    pub transform: Affine,
    pub rotation: Rot90,
    pub scale: f64,
    /// Verschuiving in mm (na rotatie en schaal), voor weergave.
    pub dx_mm: f64,
    pub dy_mm: f64,
    /// Rotatie bovenop `rotation` om het midden van B, in graden; alleen handmatig.
    #[serde(default)]
    pub free_deg: f64,
    /// Overlap van de inkt, 0..1.
    pub score: f64,
    /// Deel van de inkt van A dat in B terugkomt, en andersom. Het gemiddelde is `score`.
    #[serde(default)]
    pub a_in_b: f64,
    #[serde(default)]
    pub b_in_a: f64,
    /// true als de transformatie exact identiteit is (zelfde formaat, geen verschuiving).
    pub identity: bool,
    pub ok: bool,
    pub note: String,
}

impl Registration {
    pub fn identity_with_score(score: f64) -> Self {
        Registration {
            transform: Affine::IDENTITY,
            rotation: Rot90::R0,
            free_deg: 0.0,
            scale: 1.0,
            dx_mm: 0.0,
            dy_mm: 0.0,
            score,
            a_in_b: score,
            b_in_a: score,
            identity: true,
            ok: true,
            note: "identiteit".into(),
        }
    }
}

/// Handmatige uitlijning van B op A: hele kwartslagen plus een vrije rest, schaal, verschuiving.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ManualAlign {
    /// Rotatie in graden met de klok mee, elke waarde.
    pub rotation_deg: f64,
    pub scale: f64,
    pub dx_mm: f64,
    pub dy_mm: f64,
}

impl ManualAlign {
    /// De waarden van een bestaande registratie, als startpunt om bij te stellen.
    pub fn of(r: &Registration) -> ManualAlign {
        ManualAlign {
            rotation_deg: r.rotation.degrees() as f64 + r.free_deg,
            scale: r.scale,
            dx_mm: r.dx_mm,
            dy_mm: r.dy_mm,
        }
    }

    /// Leest "rotatie,schaal,dx,dy" (graden, factor, mm, mm).
    pub fn parse(s: &str) -> Result<ManualAlign> {
        let v: Vec<f64> = s
            .split(',')
            .map(|p| p.trim().parse::<f64>())
            .collect::<std::result::Result<_, _>>()
            .map_err(|e| anyhow::anyhow!("uitlijning '{s}': {e}"))?;
        anyhow::ensure!(v.len() == 4, "uitlijning verwacht rotatie,schaal,dx,dy");
        anyhow::ensure!(v[1] > 0.0, "uitlijning: schaal moet groter zijn dan 0");
        Ok(ManualAlign {
            rotation_deg: v[0],
            scale: v[1],
            dx_mm: v[2],
            dy_mm: v[3],
        })
    }

    /// Bouwt de registratie: kwartslagen, vrije rest om het midden van het gedraaide blad, schaal,
    /// verschuiving. De overlap vult de vergelijking in; hier is hij nog niet bekend. Geen NaN:
    /// dat wordt `null` in JSON en de ouder kan de regel van een werker dan niet meer lezen.
    pub fn registration(&self, b_size: (f64, f64)) -> Registration {
        let quarters = (self.rotation_deg / 90.0).round();
        let rot = Rot90::from_degrees(quarters as i64 * 90);
        let free = self.rotation_deg - quarters * 90.0;
        let (w, h) = match rot {
            Rot90::R0 | Rot90::R180 => b_size,
            _ => (b_size.1, b_size.0),
        };
        let t = rot
            .affine(b_size.0, b_size.1)
            .then(&Affine::rotate_about(free, w / 2.0, h / 2.0))
            .then(&Affine::scale(self.scale))
            .then(&Affine::translate(
                self.dx_mm / MM_PER_PT,
                self.dy_mm / MM_PER_PT,
            ));
        Registration {
            transform: t,
            rotation: rot,
            free_deg: free,
            scale: self.scale,
            dx_mm: self.dx_mm,
            dy_mm: self.dy_mm,
            score: 0.0,
            a_in_b: 0.0,
            b_in_a: 0.0,
            identity: rot == Rot90::R0
                && free == 0.0
                && self.scale == 1.0
                && self.dx_mm == 0.0
                && self.dy_mm == 0.0,
            ok: true,
            note: format!(
                "handmatig: rotatie {:.2} gr, schaal {:.4}, verschuiving {:.2} / {:.2} mm",
                self.rotation_deg, self.scale, self.dx_mm, self.dy_mm
            ),
        }
    }
}

impl Registration {
    /// Meet de overlap van de inkt onder deze transformatie (handmatige uitlijning).
    pub fn measure(
        &mut self,
        page_a: &PdfPage,
        size_a: (f64, f64),
        page_b: &PdfPage,
        p: &RegParams,
    ) -> Result<()> {
        let ga = pdf::render_page_gray(page_a, p.fine_dpi, &Affine::IDENTITY, size_a)?;
        let gb = pdf::render_page_gray(page_b, p.fine_dpi, &self.transform, size_a)?;
        (self.a_in_b, self.b_in_a) = overlap_parts(&ga, &gb);
        self.score = (self.a_in_b + self.b_in_a) / 2.0;
        Ok(())
    }
}

/// Inktdichtheid per kolom en per rij (255 - grijs, opgeteld).
#[allow(clippy::needless_range_loop)]
fn profiles(g: &Gray) -> (Vec<f64>, Vec<f64>) {
    let mut cols = vec![0f64; g.w];
    let mut rows = vec![0f64; g.h];
    for y in 0..g.h {
        let row = &g.data[y * g.w..(y + 1) * g.w];
        let mut s = 0u64;
        for (x, &v) in row.iter().enumerate() {
            let ink = 255 - v as u64;
            cols[x] += ink as f64;
            s += ink;
        }
        rows[y] = s as f64;
    }
    (cols, rows)
}

fn sample_linear(p: &[f64], x: f64) -> f64 {
    if x < 0.0 || x > (p.len() - 1) as f64 {
        return 0.0;
    }
    let i = x.floor() as usize;
    let f = x - i as f64;
    if i + 1 >= p.len() {
        p[i]
    } else {
        p[i] * (1.0 - f) + p[i + 1] * f
    }
}

/// Genormaliseerde correlatie tussen pa(x) en pb((x - d)/s) over de lengte van pa.
fn ncc(pa: &[f64], pb: &[f64], s: f64, d: f64) -> f64 {
    let n = pa.len();
    let ma = pa.iter().sum::<f64>() / n as f64;
    let mut vb = Vec::with_capacity(n);
    for x in 0..n {
        vb.push(sample_linear(pb, (x as f64 - d) / s));
    }
    let mb = vb.iter().sum::<f64>() / n as f64;
    let (mut num, mut da, mut db) = (0.0, 0.0, 0.0);
    for x in 0..n {
        let a = pa[x] - ma;
        let b = vb[x] - mb;
        num += a * b;
        da += a * a;
        db += b * b;
    }
    if da <= 0.0 || db <= 0.0 {
        0.0
    } else {
        num / (da * db).sqrt()
    }
}

/// Beste verschuiving (geheel, daarna parabolisch verfijnd) voor gegeven schaal.
fn best_shift(pa: &[f64], pb: &[f64], s: f64, max_shift: i64) -> (f64, f64) {
    let mut best = (f64::MIN, 0i64);
    let mut scores = Vec::with_capacity((2 * max_shift + 1) as usize);
    for d in -max_shift..=max_shift {
        let c = ncc(pa, pb, s, d as f64);
        scores.push(c);
        if c > best.0 {
            best = (c, d);
        }
    }
    let i = (best.1 + max_shift) as usize;
    let mut d = best.1 as f64;
    if i > 0 && i + 1 < scores.len() {
        let (l, c, r) = (scores[i - 1], scores[i], scores[i + 1]);
        let den = l - 2.0 * c + r;
        if den.abs() > 1e-12 {
            let off = 0.5 * (l - r) / den;
            if off.abs() <= 1.0 {
                d += off;
            }
        }
    }
    (best.0, d)
}

/// Zoekt schaal (gedeeld voor x en y) en verschuiving per as.
fn search_scale_shift(
    a: (&[f64], &[f64]),
    b: (&[f64], &[f64]),
    scale_range: (f64, f64, f64),
    max_shift: i64,
) -> (f64, f64, f64, f64) {
    let (lo, hi, step) = scale_range;
    let mut best = (f64::MIN, 1.0, 0.0, 0.0);
    let n = ((hi - lo) / step).round() as i64;
    for k in 0..=n {
        let s = lo + k as f64 * step;
        let (cx, dx) = best_shift(a.0, b.0, s, max_shift);
        let (cy, dy) = best_shift(a.1, b.1, s, max_shift);
        let c = cx + cy;
        if c > best.0 {
            best = (c, s, dx, dy);
        }
    }
    best
}

/// Overlapscore van inkt: gemiddelde van (A-inkt met B-inkt binnen 1 px) en omgekeerd.
pub fn overlap_score(a: &Gray, b: &Gray) -> f64 {
    let (x, y) = overlap_parts(a, b);
    (x + y) / 2.0
}

/// Deel van de A-inkt met B-inkt binnen 1 px, en andersom.
pub fn overlap_parts(a: &Gray, b: &Gray) -> (f64, f64) {
    const INK: u8 = 160;
    let amin = crate::raster::min_filter(a, 1);
    let bmin = crate::raster::min_filter(b, 1);
    let (mut na, mut ha, mut nb, mut hb) = (0u64, 0u64, 0u64, 0u64);
    for i in 0..a.data.len() {
        if a.data[i] < INK {
            na += 1;
            if bmin.data[i] < INK {
                ha += 1;
            }
        }
        if b.data[i] < INK {
            nb += 1;
            if amin.data[i] < INK {
                hb += 1;
            }
        }
    }
    if na == 0 || nb == 0 {
        return (0.0, 0.0);
    }
    (ha as f64 / na as f64, hb as f64 / nb as f64)
}

/// Registreert pagina B op pagina A.
pub fn register(
    page_a: &PdfPage,
    geom_a: &PageGeom,
    page_b: &PdfPage,
    geom_b: &PageGeom,
    p: &RegParams,
) -> Result<Registration> {
    let size_a = geom_a.display_size();
    let size_b = geom_b.display_size();

    // --- grof ---
    let sc = p.coarse_dpi / 72.0;
    let ga = pdf::render_page_gray(page_a, p.coarse_dpi, &Affine::IDENTITY, size_a)?;
    let pa = profiles(&ga);
    let a_landscape = size_a.0 >= size_a.1;
    let b_landscape = size_b.0 >= size_b.1;
    let mut rots = if a_landscape == b_landscape {
        vec![Rot90::R0]
    } else {
        vec![Rot90::R90, Rot90::R270]
    };
    if p.try_180 {
        if a_landscape == b_landscape {
            rots.push(Rot90::R180);
        } else {
            // al beide kwartslagen
        }
    }
    let max_shift_c = (p.max_shift_mm / MM_PER_PT * sc).ceil() as i64;
    let mut best: Option<(f64, Affine, Rot90, f64)> = None;
    for rot in rots {
        let rb = rot.affine(size_b.0, size_b.1);
        let rsize = match rot {
            Rot90::R0 | Rot90::R180 => size_b,
            _ => (size_b.1, size_b.0),
        };
        let s0 = ((size_a.0 / rsize.0) + (size_a.1 / rsize.1)) / 2.0;
        let base = rb.then(&Affine::scale(s0));
        let gb = pdf::render_page_gray(page_b, p.coarse_dpi, &base, size_a)?;
        let pb = profiles(&gb);
        let (c, s, dx, dy) = search_scale_shift(
            (&pa.0, &pa.1),
            (&pb.0, &pb.1),
            (1.0 - p.max_scale_dev, 1.0 + p.max_scale_dev, 0.0005),
            max_shift_c,
        );
        // profielverschuiving in coarse-pixels -> punten
        let t = base
            .then(&Affine::scale(s))
            .then(&Affine::translate(dx / sc, dy / sc));
        tracing::debug!(?rot, c, s, dx, dy, "grove registratie");
        if best.as_ref().is_none_or(|b| c > b.0) {
            best = Some((c, t, rot, s0 * s));
        }
    }
    let (_, coarse_t, rot, _) = best.expect("minstens een rotatiekandidaat");

    // --- fijn ---
    let sf = p.fine_dpi / 72.0;
    let ga = pdf::render_page_gray(page_a, p.fine_dpi, &Affine::IDENTITY, size_a)?;
    let gb = pdf::render_page_gray(page_b, p.fine_dpi, &coarse_t, size_a)?;
    let pa = profiles(&ga);
    let pb = profiles(&gb);
    let max_shift_f = (2.0 * p.fine_dpi / p.coarse_dpi).ceil() as i64 + 2;
    let (_, s, dx, dy) = search_scale_shift(
        (&pa.0, &pa.1),
        (&pb.0, &pb.1),
        (0.998, 1.002, 0.0001),
        max_shift_f,
    );
    let fine_t = coarse_t
        .then(&Affine::scale(s))
        .then(&Affine::translate(dx / sf, dy / sf));

    // Exacte identiteit als alles binnen de meetonzekerheid op nul staat.
    let same_size = (size_a.0 - size_b.0).abs() < 0.5 && (size_a.1 - size_b.1).abs() < 0.5;
    let d_id = {
        let (x, y) = fine_t.apply(0.0, 0.0);
        let (x2, y2) = fine_t.apply(size_b.0, size_b.1);
        // max afwijking van de hoekpunten in fijne pixels
        (x.abs()
            .max(y.abs())
            .max((x2 - size_b.0).abs())
            .max((y2 - size_b.1).abs()))
            * sf
    };
    let mut is_identity = rot == Rot90::R0 && same_size && d_id < 0.35;
    let mut transform = if is_identity {
        Affine::IDENTITY
    } else {
        fine_t
    };

    // --- kwaliteit ---
    let gb = pdf::render_page_gray(page_b, p.fine_dpi, &transform, size_a)?;
    let mut parts = overlap_parts(&ga, &gb);
    let mut score = (parts.0 + parts.1) / 2.0;
    // Bij gelijk formaat wint identiteit als die (vrijwel) even goed past: veel gewijzigde
    // inhoud kan de profielen een fractie verschuiven zonder dat het blad verschoven is.
    if !is_identity && rot == Rot90::R0 && same_size {
        let gi = pdf::render_page_gray(page_b, p.fine_dpi, &Affine::IDENTITY, size_a)?;
        let pi = overlap_parts(&ga, &gi);
        let si = (pi.0 + pi.1) / 2.0;
        tracing::debug!(score, si, "identiteit vs gevonden transformatie");
        if si >= score - p.identity_margin {
            is_identity = true;
            transform = Affine::IDENTITY;
            score = si;
            parts = pi;
        }
    }
    let scale = transform.scale_factor();
    let (ox, oy) = {
        // verschuiving = waar de oorsprong van het geroteerde, geschaalde B terechtkomt
        let pre = rot.affine(size_b.0, size_b.1).then(&Affine::scale(scale));
        let (px, py) = pre.apply(0.0, 0.0);
        let (tx, ty) = transform.apply(0.0, 0.0);
        ((tx - px) * MM_PER_PT, (ty - py) * MM_PER_PT)
    };
    let ok = score >= p.min_score;
    let note = if !ok {
        format!(
            "UITLIJNING MISLUKT: overlap {:.1} % < {:.0} % ({:.0} % van A komt terug in B, {:.0} % van B in A). Vergelijking op dit blad is niet betrouwbaar. Is het toch hetzelfde blad, maar echt anders ingedeeld: lijn het handmatig uit.",
            score * 100.0,
            p.min_score * 100.0,
            parts.0 * 100.0,
            parts.1 * 100.0
        )
    } else if is_identity {
        "identiteit (zelfde formaat, geen verschuiving)".into()
    } else {
        format!(
            "rotatie {} gr, schaal {:.4}, verschuiving {:.2} / {:.2} mm",
            rot.degrees(),
            scale,
            ox,
            oy
        )
    };
    Ok(Registration {
        transform,
        rotation: rot,
        free_deg: 0.0,
        scale,
        dx_mm: ox,
        dy_mm: oy,
        score,
        a_in_b: parts.0,
        b_in_a: parts.1,
        identity: is_identity,
        ok,
        note,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_align_round_trips_and_stays_valid_json() {
        let m = ManualAlign {
            rotation_deg: 93.5,
            scale: 1.01,
            dx_mm: 2.0,
            dy_mm: -3.0,
        };
        let r = m.registration((1000.0, 700.0));
        assert_eq!(r.rotation, Rot90::R90);
        assert!((r.free_deg - 3.5).abs() < 1e-9);
        assert_eq!(ManualAlign::of(&r), m);
        let back: Registration = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(back.note, r.note);
    }

    #[test]
    fn free_rotation_turns_around_the_middle_of_the_sheet() {
        let m = ManualAlign {
            rotation_deg: 10.0,
            scale: 1.0,
            dx_mm: 0.0,
            dy_mm: 0.0,
        };
        let t = m.registration((200.0, 100.0)).transform;
        let (x, y) = t.apply(100.0, 50.0);
        assert!((x - 100.0).abs() < 1e-9 && (y - 50.0).abs() < 1e-9);
        // met de klok mee in weergave: rechts van het midden gaat omlaag (y groter)
        assert!(t.apply(200.0, 50.0).1 > 50.0);
    }

    #[test]
    fn nothing_turned_is_the_identity() {
        let m = ManualAlign::parse("0, 1, 0, 0").unwrap();
        assert!(m.registration((10.0, 10.0)).identity);
        assert!(ManualAlign::parse("1,2").is_err());
        assert!(ManualAlign::parse("0,0,0,0").is_err());
    }

    #[test]
    fn profile_shift_and_scale_recovered() {
        // synthetisch profiel met pieken (kaderlijnen, maatlijnen)
        let n = 1000;
        let mut a = vec![0f64; n];
        for (i, v) in a.iter_mut().enumerate() {
            if i % 97 == 0 || i % 211 == 0 || i == 13 || i == 987 {
                *v = 100.0;
            }
        }
        // b = a geschaald met 1/1.01 en verschoven: a(x) = b((x - 7)/1.01)
        let (s_true, d_true) = (1.01, 7.0);
        let mut b = vec![0f64; n];
        for (x, v) in b.iter_mut().enumerate() {
            *v = sample_linear(&a, x as f64 * s_true + d_true);
        }
        let (_, s, d, _) = search_scale_shift((&a, &a), (&b, &b), (0.98, 1.02, 0.0005), 20);
        assert!((s - s_true).abs() < 0.0011, "schaal {s}");
        assert!((d - d_true).abs() < 1.5, "verschuiving {d}");
    }
}
