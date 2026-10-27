//! Overlaybeelden: A en B over elkaar, per pixel gekleurd (verwijderd / toegevoegd / gelijk).
//! Wordt door de viewer per tegel gebruikt en door de CLI voor een PNG.

use crate::geom::Affine;
use crate::pdf::{self, Gray};
use crate::raster::{self, DiffParams, ADDED, REMOVED};
use anyhow::Result;
use pdfium_render::prelude::PdfPage;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OverlayColors {
    /// Inkt alleen in A (verwijderd).
    pub removed: [u8; 3],
    /// Inkt alleen in B (toegevoegd).
    pub added: [u8; 3],
    /// Gelijke inkt.
    pub same: [u8; 3],
}

impl Default for OverlayColors {
    fn default() -> Self {
        Self {
            removed: [220, 30, 30],
            added: [0, 170, 200],
            same: [60, 60, 60],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViewMode {
    /// Pixeldiff: gelijk grijs, verwijderd rood, toegevoegd cyaan.
    Diff,
    /// Alleen A.
    A,
    /// Alleen B (uitgelijnd op A).
    B,
    /// Klassieke overlay: A in de A-kleur, B in de B-kleur, met dekking.
    Blend,
}

/// Kleurt een paar uitgelijnde grijsbeelden.
///
/// `dim_same` 0..1: hoe sterk gelijke inkt wordt weergegeven (1 = volle `same`-kleur).
/// `mix` 0..1 bij Blend: 0 = alleen A, 1 = alleen B.
pub fn compose(
    a: &Gray,
    b: &Gray,
    class: Option<&[u8]>,
    mode: ViewMode,
    colors: &OverlayColors,
    dim_same: f32,
    mix: f32,
) -> Vec<u8> {
    let n = a.w * a.h;
    let mut out = vec![255u8; n * 4];
    let lerp = |ink: u8, c: [u8; 3], k: f32| -> [u8; 3] {
        // ink: 0 = vol, 255 = papier
        let t = (255 - ink) as f32 / 255.0 * k;
        [
            (255.0 - t * (255.0 - c[0] as f32)) as u8,
            (255.0 - t * (255.0 - c[1] as f32)) as u8,
            (255.0 - t * (255.0 - c[2] as f32)) as u8,
        ]
    };
    for i in 0..n {
        let (va, vb) = (a.data[i], b.data[i]);
        let px = match mode {
            ViewMode::A => [va, va, va],
            ViewMode::B => [vb, vb, vb],
            ViewMode::Blend => {
                let ca = lerp(va, colors.removed, 1.0 - mix);
                let cb = lerp(vb, colors.added, mix);
                // multiplicatief mengen: waar beide inkt hebben wordt het donker
                [
                    ((ca[0] as u16 * cb[0] as u16) / 255) as u8,
                    ((ca[1] as u16 * cb[1] as u16) / 255) as u8,
                    ((ca[2] as u16 * cb[2] as u16) / 255) as u8,
                ]
            }
            ViewMode::Diff => {
                let c = class.map(|c| c[i]).unwrap_or(0);
                if c & REMOVED != 0 && c & ADDED == 0 {
                    lerp(va, colors.removed, 1.0)
                } else if c & ADDED != 0 && c & REMOVED == 0 {
                    lerp(vb, colors.added, 1.0)
                } else if c != 0 {
                    // beide: kleur naar de donkerste kant
                    if va <= vb {
                        lerp(va, colors.removed, 1.0)
                    } else {
                        lerp(vb, colors.added, 1.0)
                    }
                } else {
                    lerp(va.min(vb), colors.same, dim_same)
                }
            }
        };
        out[i * 4..i * 4 + 4].copy_from_slice(&[px[0], px[1], px[2], 255]);
    }
    out
}

/// Rendert een tegel van A en B (B via `b_to_a`) in A-weergavepunten en kleurt hem.
/// `to_px` beeldt A-weergavepunten af op tegelpixels.
#[allow(clippy::too_many_arguments)]
pub fn render_tile(
    page_a: Option<&PdfPage>,
    page_b: Option<&PdfPage>,
    b_to_a: &Affine,
    to_px: &Affine,
    w: usize,
    h: usize,
    params: &DiffParams,
    mode: ViewMode,
    colors: &OverlayColors,
    dim_same: f32,
    mix: f32,
) -> Result<(Vec<u8>, u64, u64)> {
    // rand meerenderen zodat het minimumfilter aan de tegelrand klopt
    let m = params.radius_px + 1;
    let pad = Affine::translate(m as f64, m as f64);
    let (pw, ph) = (w + 2 * m, h + 2 * m);
    let blank = || Gray::new(pw, ph, 255);
    let ga = match page_a {
        Some(p) => pdf::render_gray(p, &to_px.then(&pad), pw, ph)?,
        None => blank(),
    };
    let gb = match page_b {
        Some(p) => pdf::render_gray(p, &b_to_a.then(to_px).then(&pad), pw, ph)?,
        None => blank(),
    };
    let d = raster::diff(&ga, &gb, params);
    let crop = |g: &Gray| -> Gray {
        let mut data = Vec::with_capacity(w * h);
        for y in m..m + h {
            data.extend_from_slice(&g.data[y * pw + m..y * pw + m + w]);
        }
        Gray { w, h, data }
    };
    let mut cls = Vec::with_capacity(w * h);
    let (mut rem, mut add) = (0u64, 0u64);
    for y in m..m + h {
        let row = &d.class[y * pw + m..y * pw + m + w];
        for &c in row {
            if c & REMOVED != 0 {
                rem += 1;
            }
            if c & ADDED != 0 {
                add += 1;
            }
        }
        cls.extend_from_slice(row);
    }
    let (ca, cb) = (crop(&ga), crop(&gb));
    Ok((
        compose(&ca, &cb, Some(&cls), mode, colors, dim_same, mix),
        rem,
        add,
    ))
}

pub fn save_png(path: &std::path::Path, rgba: Vec<u8>, w: usize, h: usize) -> Result<()> {
    let img = image::RgbaImage::from_raw(w as u32, h as u32, rgba)
        .ok_or_else(|| anyhow::anyhow!("buffer past niet bij {w}x{h}"))?;
    img.save(path)?;
    Ok(())
}
