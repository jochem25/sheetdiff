//! PDFium-wrapper: openen, paginageometrie, renderen (ook stroken/tegels) en tekst met bbox.
//!
//! PDFium is niet thread-safe; pdfium-render (feature `thread_safe`) zet er een mutex omheen.
//! Binnen een proces renderen dus nooit twee pagina's tegelijk. Parallelliseren doe je over
//! processen (zie `sheetdiff-cli worker`), niet over threads.

use crate::geom::{Affine, PageGeom, RectPt, Rot90};
use anyhow::{anyhow, Context, Result};
use pdfium_render::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Doorgegeven zodat afnemers geen eigen pdfium-render-afhankelijkheid nodig hebben.
pub use pdfium_render::prelude::PdfPage;

static PDFIUM: OnceLock<std::result::Result<Pdfium, String>> = OnceLock::new();

/// Waar pdfium.dll gezocht wordt, in volgorde.
pub fn pdfium_search_paths() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(p) = std::env::var("SHEETDIFF_PDFIUM") {
        v.push(PathBuf::from(p));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            v.push(dir.to_path_buf());
        }
    }
    // ontwikkelomgeving: vendor/pdfium/bin in de workspace
    v.push(PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../vendor/pdfium/bin"
    )));
    v
}

/// Gedeelde PDFium-instantie. Geeft een duidelijke fout als de DLL ontbreekt.
pub fn pdfium() -> Result<&'static Pdfium> {
    let r = PDFIUM.get_or_init(|| {
        let mut errors = Vec::new();
        for dir in pdfium_search_paths() {
            let lib = if dir.extension().is_some_and(|e| e == "dll") {
                dir.clone()
            } else {
                Pdfium::pdfium_platform_library_name_at_path(&dir)
            };
            if !lib.exists() {
                continue;
            }
            match Pdfium::bind_to_library(&lib) {
                Ok(b) => return Ok(Pdfium::new(b)),
                Err(e) => errors.push(format!("{}: {e}", lib.display())),
            }
        }
        Err(format!(
            "pdfium.dll niet gevonden of niet te laden. Zet pdfium.dll naast de exe of wijs hem aan \
             met SHEETDIFF_PDFIUM. Gezocht: {:?}. Fouten: {:?}",
            pdfium_search_paths(),
            errors
        ))
    });
    r.as_ref().map_err(|e| anyhow!(e.clone()))
}

pub struct Doc {
    pub path: PathBuf,
    pub inner: PdfDocument<'static>,
}

impl Doc {
    pub fn open(path: &Path) -> Result<Doc> {
        let p = pdfium()?;
        let inner = p
            .load_pdf_from_file(path, None)
            .map_err(|e| anyhow!("PDF niet te openen: {}: {e}", path.display()))?;
        Ok(Doc {
            path: path.to_path_buf(),
            inner,
        })
    }

    pub fn page_count(&self) -> usize {
        self.inner.pages().len() as usize
    }

    pub fn page(&self, idx: usize) -> Result<PdfPage<'_>> {
        self.inner.pages().get(idx as i32).map_err(|e| {
            anyhow!(
                "pagina {idx} niet te laden uit {}: {e}",
                self.path.display()
            )
        })
    }

    pub fn geom(&self, idx: usize) -> Result<PageGeom> {
        let page = self.page(idx)?;
        page_geom(&page)
    }
}

pub fn page_geom(page: &PdfPage) -> Result<PageGeom> {
    let crop = page
        .boundaries()
        .crop()
        .or_else(|_| page.boundaries().media())
        .map_err(|e| anyhow!("geen crop/mediabox: {e}"))?
        .bounds;
    let rotate = match page.rotation().unwrap_or(PdfPageRenderRotation::None) {
        PdfPageRenderRotation::None => Rot90::R0,
        PdfPageRenderRotation::Degrees90 => Rot90::R90,
        PdfPageRenderRotation::Degrees180 => Rot90::R180,
        PdfPageRenderRotation::Degrees270 => Rot90::R270,
    };
    let g = PageGeom {
        crop: [
            crop.left().value as f64,
            crop.bottom().value as f64,
            crop.right().value as f64,
            crop.top().value as f64,
        ],
        rotate,
    };
    // PDFium rendert op zijn eigen paginamaat; wijkt onze geometrie daarvan af (bijv. CropBox
    // buiten MediaBox), dan kloppen coordinaten niet. Luid weigeren in plaats van verschuiven.
    let (w, h) = g.display_size();
    let (pw, ph) = (page.width().value as f64, page.height().value as f64);
    if (w - pw).abs() > 1.0 || (h - ph).abs() > 1.0 {
        anyhow::bail!(
            "paginageometrie onbetrouwbaar: CropBox geeft {w:.1}x{h:.1} pt, PDFium {pw:.1}x{ph:.1} pt"
        );
    }
    Ok(g)
}

/// Grijswaardenbeeld, 8 bit, rij voor rij. 255 = wit (papier), 0 = zwart.
#[derive(Clone)]
pub struct Gray {
    pub w: usize,
    pub h: usize,
    pub data: Vec<u8>,
}

impl Gray {
    pub fn new(w: usize, h: usize, fill: u8) -> Gray {
        Gray {
            w,
            h,
            data: vec![fill; w * h],
        }
    }
    #[inline]
    pub fn get(&self, x: usize, y: usize) -> u8 {
        self.data[y * self.w + x]
    }
    pub fn to_image(&self) -> image::GrayImage {
        image::GrayImage::from_raw(self.w as u32, self.h as u32, self.data.clone())
            .expect("afmetingen kloppen per constructie")
    }
}

/// Kleurenbeeld RGBA8.
#[derive(Clone)]
pub struct Rgba {
    pub w: usize,
    pub h: usize,
    pub data: Vec<u8>,
}

/// Rec. 601 luma in gehele getallen.
#[inline]
pub fn luma(r: u8, g: u8, b: u8) -> u8 {
    ((r as u32 * 299 + g as u32 * 587 + b as u32 * 114 + 500) / 1000) as u8
}

/// Rendert een uitsnede van een pagina in een eigen, wit gevulde buffer.
///
/// `to_px` beeldt weergavepunten van DEZE pagina af op pixels van het doelbeeld (bijv.
/// schaal dpi/72, eventueel samengesteld met een registratietransformatie en een
/// tegeloffset). Het doelbeeld is `w` x `h` pixels. Er wordt niet geresampled: PDFium
/// rendert de vectoren direct in het doelraster, dus ook een uitgelijnd blad blijft scherp.
///
/// PDFium past de matrix van `FPDF_RenderPageBitmapWithMatrix` toe op weergavepunten
/// (na /Rotate, oorsprong linksboven). pdfium-render vermenigvuldigt hem met een schaal
/// uit de doelmaat; die is hier 1 omdat er geen doelmaat gezet wordt.
pub fn render_region(
    page: &PdfPage,
    to_px: &Affine,
    w: usize,
    h: usize,
    with_annotations: bool,
) -> Result<(Vec<u8>, usize)> {
    pdfium()?;
    let pf = PdfBitmapFormat::BGRA;
    let need = PdfBitmap::bytes_required_for_size_and_format(w as i32, h as i32, pf);
    let stride = need / h.max(1);
    let mut buf = vec![255u8; need];
    {
        let mut bitmap = PdfBitmap::from_bytes(w as i32, h as i32, pf, &mut buf)
            .map_err(|e| anyhow!("bitmap {w}x{h} niet aan te maken: {e}"))?;
        let m = to_px;
        // pdfium-render zet standaard FPDF_REVERSE_BYTE_ORDER: de buffer is dan RGBA, niet BGRA.
        let cfg = PdfRenderConfig::new()
            .set_reverse_byte_order(true)
            .clear_before_rendering(false)
            .render_annotations(with_annotations)
            .render_form_data(false)
            .set_path_smoothing(true)
            .set_text_smoothing(true)
            .set_image_smoothing(true)
            .reset_matrix(PdfMatrix::new(
                m.a as f32, m.b as f32, m.c as f32, m.d as f32, m.e as f32, m.f as f32,
            ))
            .map_err(|e| anyhow!("ongeldige transformatie: {e}"))?
            .clip(0, 0, w as i32, h as i32);
        page.render_into_bitmap_with_config(&mut bitmap, &cfg)
            .map_err(|e| anyhow!("renderen mislukt: {e}"))?;
    }
    Ok((buf, stride))
}

/// Rendert een uitsnede grijs.
pub fn render_gray(page: &PdfPage, to_px: &Affine, w: usize, h: usize) -> Result<Gray> {
    // Bewust via BGRA: PDFium 157.0.8086 breekt het proces af (exit 3) bij renderen naar
    // een FPDFBitmap_Gray-bitmap met matrix. Gemeten 09-10-2026.
    let (buf, stride) = render_region(page, to_px, w, h, false)?;
    let mut data = Vec::with_capacity(w * h);
    for row in buf.chunks_exact(stride).take(h) {
        for px in row[..w * 4].as_chunks::<4>().0 {
            data.push(luma(px[0], px[1], px[2]));
        }
    }
    Ok(Gray { w, h, data })
}

/// Rendert een uitsnede in kleur (RGBA8, voor de viewer).
pub fn render_rgba(
    page: &PdfPage,
    to_px: &Affine,
    w: usize,
    h: usize,
    with_annotations: bool,
) -> Result<Rgba> {
    let (buf, stride) = render_region(page, to_px, w, h, with_annotations)?;
    let mut data = Vec::with_capacity(w * h * 4);
    for row in buf.chunks_exact(stride).take(h) {
        for px in row[..w * 4].as_chunks::<4>().0 {
            data.extend_from_slice(&[px[0], px[1], px[2], 255]);
        }
    }
    Ok(Rgba { w, h, data })
}

/// Rendert een hele pagina grijs op `dpi`, eventueel in weergaveruimte van een andere pagina
/// via `align` (weergavepunten deze pagina -> weergavepunten doelpagina) en met doelformaat.
pub fn render_page_gray(
    page: &PdfPage,
    dpi: f64,
    align: &Affine,
    target_size_pt: (f64, f64),
) -> Result<Gray> {
    let s = dpi / 72.0;
    let w = (target_size_pt.0 * s).round().max(1.0) as usize;
    let h = (target_size_pt.1 * s).round().max(1.0) as usize;
    let to_px = align.then(&Affine::scale(s));
    render_gray(page, &to_px, w, h)
}

/// Tekstrun met bbox in weergavepunten.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TextRun {
    pub text: String,
    pub bbox: RectPt,
}

/// Tekstruns van een pagina (PDFium-segmenten: aaneengesloten tekst op een regel).
pub fn text_runs(page: &PdfPage, geom: &PageGeom) -> Result<Vec<TextRun>> {
    let text = page
        .text()
        .map_err(|e| anyhow!("tekstlaag niet te lezen: {e}"))?;
    let u2d = geom.user_to_display();
    let mut out = Vec::new();
    for seg in text.segments().iter() {
        let t = seg.text();
        let t = t.trim();
        if t.is_empty() {
            continue;
        }
        let b = seg.bounds();
        let r = RectPt::new(
            b.left().value as f64,
            b.bottom().value as f64,
            b.right().value as f64,
            b.top().value as f64,
        )
        .transform(&u2d);
        out.push(TextRun {
            text: t.to_string(),
            bbox: r,
        });
    }
    Ok(out)
}

/// Bestand bestaat en is leesbaar; voor duidelijke foutmeldingen voor het openen.
pub fn check_file(path: &Path) -> Result<()> {
    std::fs::metadata(path)
        .with_context(|| format!("bestand niet gevonden: {}", path.display()))?;
    Ok(())
}
