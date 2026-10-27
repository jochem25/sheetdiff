//! Verkenning: render een pagina en teken de tekstbboxen erop, om de coordinaten te controleren.
use sheetdiff_core::geom::Affine;
use sheetdiff_core::pdf::{self, Doc};
use std::time::Instant;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let path = std::path::PathBuf::from(&args[1]);
    let pidx: usize = args.get(2).map(|s| s.parse().unwrap()).unwrap_or(0);
    let dpi: f64 = args.get(3).map(|s| s.parse().unwrap()).unwrap_or(30.0);
    let out = args.get(4).cloned().unwrap_or("probe.png".into());
    let t = Instant::now();
    let doc = Doc::open(&path)?;
    println!("open {:?} pagina's {}", t.elapsed(), doc.page_count());
    let page = doc.page(pidx)?;
    let g = pdf::page_geom(&page)?;
    let (w, h) = g.display_size();
    println!(
        "geom {:?} display {:.1}x{:.1} pt; pdfium w/h {} {}",
        g,
        w,
        h,
        page.width().value,
        page.height().value
    );
    let t = Instant::now();
    let img = pdf::render_page_gray(&page, dpi, &Affine::IDENTITY, (w, h))?;
    println!("render {}x{} in {:?}", img.w, img.h, t.elapsed());
    let t = Instant::now();
    let runs = pdf::text_runs(&page, &g)?;
    println!("{} tekstruns in {:?}", runs.len(), t.elapsed());
    let mut rgb = image::RgbImage::from_fn(img.w as u32, img.h as u32, |x, y| {
        let v = img.get(x as usize, y as usize);
        image::Rgb([v, v, v])
    });
    let s = dpi / 72.0;
    for r in &runs {
        let (x0, y0, x1, y1) = (
            (r.bbox.x0 * s) as i64,
            (r.bbox.y0 * s) as i64,
            (r.bbox.x1 * s) as i64,
            (r.bbox.y1 * s) as i64,
        );
        for x in x0..=x1 {
            for y in [y0, y1] {
                if x >= 0 && y >= 0 && (x as u32) < rgb.width() && (y as u32) < rgb.height() {
                    rgb.put_pixel(x as u32, y as u32, image::Rgb([255, 0, 0]));
                }
            }
        }
        for y in y0..=y1 {
            for x in [x0, x1] {
                if x >= 0 && y >= 0 && (x as u32) < rgb.width() && (y as u32) < rgb.height() {
                    rgb.put_pixel(x as u32, y as u32, image::Rgb([255, 0, 0]));
                }
            }
        }
    }
    for r in runs.iter().rev().take(15) {
        println!("  {:?}", r);
    }
    rgb.save(&out)?;
    Ok(())
}
