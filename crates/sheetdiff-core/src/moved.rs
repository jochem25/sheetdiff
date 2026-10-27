//! Verplaatste onderdelen herkennen. Een blad kan uit delen bestaan die onderling verschoven
//! zijn: een viewport die een paar mm is opgeschoven terwijl kader en titelhoek vast staan.
//! De globale uitlijning legt dan het grootste deel goed en de rest verschijnt als dubbel
//! (weg + erbij). Per verschilregio toetsen we of een tweede transformatie (bijv. die van het
//! kader) het verschil daar wel verklaart; zo ja, dan is die regio "verplaatst", geen
//! inhoudelijke wijziging.

use crate::geom::{Affine, RectPt, MM_PER_PT};
use crate::pdf::{self, Gray};
use crate::raster::{self, DiffParams};
use anyhow::Result;
use pdfium_render::prelude::PdfPage;

/// Uitkomst per regio: verschuiving in mm (B t.o.v. A) als de alternatieve transformatie het
/// verschil verklaart.
pub type MovedMm = Option<(f64, f64)>;

fn count_in(d: &[u8], w: usize, h: usize, r: &RectPt, s: f64) -> u64 {
    let x0 = ((r.x0 * s).floor().max(0.0) as usize).min(w);
    let x1 = ((r.x1 * s).ceil().max(0.0) as usize).min(w);
    let y0 = ((r.y0 * s).floor().max(0.0) as usize).min(h);
    let y1 = ((r.y1 * s).ceil().max(0.0) as usize).min(h);
    let mut n = 0;
    for y in y0..y1 {
        n += d[y * w + x0..y * w + x1]
            .iter()
            .filter(|&&c| c != 0)
            .count() as u64;
    }
    n
}

/// Toetst per regio (A-weergavepunten) of `alt` het verschil beter verklaart dan `main`.
/// Werkt op `dpi` (100 is genoeg: het gaat om verschuivingen van tienden mm en meer).
#[allow(clippy::too_many_arguments)]
pub fn classify(
    page_a: &PdfPage,
    size_a: (f64, f64),
    page_b: &PdfPage,
    main: &Affine,
    alt: &Affine,
    regions: &[RectPt],
    params: &DiffParams,
    dpi: f64,
) -> Result<Vec<MovedMm>> {
    if regions.is_empty() || main == alt {
        return Ok(vec![None; regions.len()]);
    }
    let s = dpi / 72.0;
    let (w, h) = (
        (size_a.0 * s).round() as usize,
        (size_a.1 * s).round() as usize,
    );
    let to_px = Affine::scale(s);
    let ga: Gray = pdf::render_gray(page_a, &to_px, w, h)?;
    let gm = pdf::render_gray(page_b, &main.then(&to_px), w, h)?;
    let gl = pdf::render_gray(page_b, &alt.then(&to_px), w, h)?;
    let dm = raster::diff(&ga, &gm, params);
    let dl = raster::diff(&ga, &gl, params);
    let mut out = Vec::with_capacity(regions.len());
    for r in regions {
        let rr = r.expand(1.0);
        let nm = count_in(&dm.class, w, h, &rr, s);
        let nl = count_in(&dl.class, w, h, &rr, s);
        if nm >= 4 && (nl as f64) < 0.2 * nm as f64 {
            // verschuiving: waar landt het midden van de regio onder beide transformaties
            let (cx, cy) = r.center();
            let inv_m = main.inverse();
            let moved = inv_m.map(|im| {
                // B-punt dat onder `main` op (cx,cy) valt, en waar `alt` dat B-punt legt
                let (bx, by) = im.apply(cx, cy);
                let (ax, ay) = alt.apply(bx, by);
                ((ax - cx) * MM_PER_PT, (ay - cy) * MM_PER_PT)
            });
            out.push(moved);
        } else {
            out.push(None);
        }
    }
    Ok(out)
}
