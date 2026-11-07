//! Beeldlaag voor de viewer: A, B uitgelijnd en de pixelklasse van een uitsnede. Staat in core
//! zodat een werkerproces de laag van een buurblad vooraf kan maken.

use crate::geom::{Affine, RectPt};
use crate::pdf::{self, Doc, Gray};
use crate::raster::{self, DiffParams};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SheetSrc {
    pub a: Option<(PathBuf, usize)>,
    pub b: Option<(PathBuf, usize)>,
    pub b_to_a: Affine,
    /// Afmeting van het blad in A-weergavepunten.
    pub size_pt: (f64, f64),
}

impl SheetSrc {
    pub fn files(&self) -> impl Iterator<Item = &PathBuf> {
        self.a.iter().chain(self.b.iter()).map(|x| &x.0)
    }
}

#[derive(Clone)]
pub struct Layer {
    pub rect_pt: RectPt,
    pub dpi: f64,
    pub w: usize,
    pub h: usize,
    pub a: Gray,
    pub b: Gray,
    pub class: Vec<u8>,
}

impl Layer {
    /// Binair formaat voor de overdracht van werker naar app.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(64 + self.w * self.h * 3);
        for f in [
            self.rect_pt.x0,
            self.rect_pt.y0,
            self.rect_pt.x1,
            self.rect_pt.y1,
            self.dpi,
        ] {
            v.extend_from_slice(&f.to_le_bytes());
        }
        v.extend_from_slice(&(self.w as u64).to_le_bytes());
        v.extend_from_slice(&(self.h as u64).to_le_bytes());
        v.extend_from_slice(&self.a.data);
        v.extend_from_slice(&self.b.data);
        v.extend_from_slice(&self.class);
        v
    }

    /// Omgekeerde van [`Layer::to_bytes`]; `None` als de lengte niet klopt.
    pub fn from_bytes(s: &[u8]) -> Option<Layer> {
        let word = |i: usize| -> Option<[u8; 8]> { s.get(i * 8..i * 8 + 8)?.try_into().ok() };
        let f = |i: usize| word(i).map(f64::from_le_bytes);
        let (w, h) = (
            u64::from_le_bytes(word(5)?) as usize,
            u64::from_le_bytes(word(6)?) as usize,
        );
        let n = w.checked_mul(h)?;
        const HEAD: usize = 7 * 8;
        if s.len() != HEAD + n.checked_mul(3)? {
            return None;
        }
        let part = |i: usize| s[HEAD + i * n..HEAD + (i + 1) * n].to_vec();
        Some(Layer {
            rect_pt: RectPt::new(f(0)?, f(1)?, f(2)?, f(3)?),
            dpi: f(4)?,
            w,
            h,
            a: Gray {
                w,
                h,
                data: part(0),
            },
            b: Gray {
                w,
                h,
                data: part(1),
            },
            class: part(2),
        })
    }
}

/// Basis-dpi: hoogstens 100, en de langste zijde hoogstens 4096 px (textuurgrens, geheugen).
pub fn base_dpi(size_pt: (f64, f64)) -> f64 {
    (4096.0 * 72.0 / size_pt.0.max(size_pt.1)).min(100.0)
}

/// De basislaag: het hele blad op [`base_dpi`].
pub fn render_base(
    docs: &HashMap<PathBuf, Doc>,
    src: &SheetSrc,
    params: &DiffParams,
) -> anyhow::Result<Layer> {
    let rect = RectPt::new(0.0, 0.0, src.size_pt.0, src.size_pt.1);
    render_layer(docs, src, rect, base_dpi(src.size_pt), params)
}

pub fn render_layer(
    docs: &HashMap<PathBuf, Doc>,
    src: &SheetSrc,
    rect: RectPt,
    dpi: f64,
    params: &DiffParams,
) -> anyhow::Result<Layer> {
    let s = dpi / 72.0;
    let w = ((rect.width() * s).round() as usize).max(1);
    let h = ((rect.height() * s).round() as usize).max(1);
    let m = params.radius_px + 1;
    let to_px = Affine::translate(-rect.x0, -rect.y0)
        .then(&Affine::scale(s))
        .then(&Affine::translate(m as f64, m as f64));
    let (pw, ph) = (w + 2 * m, h + 2 * m);
    // Een kant die er hoort te zijn maar niet te laden is, is een FOUT, geen wit blad:
    // anders toont de viewer "alles erbij" of "alles weg" (review 09-10 #1).
    let page =
        |p: &Option<(PathBuf, usize)>, side: &str| -> anyhow::Result<Option<pdf::PdfPage<'_>>> {
            match p {
                None => Ok(None),
                Some((f, i)) => {
                    let d = docs
                        .get(f)
                        .ok_or_else(|| anyhow::anyhow!("{side} niet te openen: {}", f.display()))?;
                    Ok(Some(d.page(*i)?))
                }
            }
        };
    let ga = match page(&src.a, "A")? {
        Some(p) => pdf::render_gray(&p, &to_px, pw, ph)?,
        None => Gray::new(pw, ph, 255),
    };
    let gb = match page(&src.b, "B")? {
        Some(p) => pdf::render_gray(&p, &src.b_to_a.then(&to_px), pw, ph)?,
        None => Gray::new(pw, ph, 255),
    };
    let d = raster::diff(&ga, &gb, params);
    let crop = |data: &[u8]| -> Vec<u8> {
        let mut v = Vec::with_capacity(w * h);
        for y in m..m + h {
            v.extend_from_slice(&data[y * pw + m..y * pw + m + w]);
        }
        v
    };
    Ok(Layer {
        rect_pt: rect,
        dpi,
        w,
        h,
        a: Gray {
            w,
            h,
            data: crop(&ga.data),
        },
        b: Gray {
            w,
            h,
            data: crop(&gb.data),
        },
        class: crop(&d.class),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_survives_the_binary_format() {
        let g = |v: u8| Gray {
            w: 3,
            h: 2,
            data: vec![v; 6],
        };
        let l = Layer {
            rect_pt: RectPt::new(1.0, 2.0, 3.0, 4.0),
            dpi: 90.0,
            w: 3,
            h: 2,
            a: g(1),
            b: g(2),
            class: vec![3; 6],
        };
        let bytes = l.to_bytes();
        let r = Layer::from_bytes(&bytes).unwrap();
        assert_eq!((r.w, r.h, r.dpi, r.rect_pt), (3, 2, 90.0, l.rect_pt));
        assert_eq!((r.a.data, r.b.data, r.class), (l.a.data, l.b.data, l.class));
        assert!(Layer::from_bytes(&bytes[..bytes.len() - 1]).is_none());
    }
}
