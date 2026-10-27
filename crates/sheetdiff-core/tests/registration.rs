//! Registratietests met bekende verschuiving, schaal en rotatie. De PDF's worden in de test
//! zelf met PDFium gemaakt: een asymmetrische "tekening" (kader, titelhoek, raster, cirkels).

use pdfium_render::prelude::*;
use sheetdiff_core::geom::{Affine, Rot90, PT_PER_MM};
use sheetdiff_core::pdf::{self, Doc};
use sheetdiff_core::register::{register, RegParams};

const W_MM: f64 = 841.0;
const H_MM: f64 = 594.0;

/// Tekent in weergave-mm (x rechts, y omlaag), afgebeeld met `t` (mm -> weergave-pt van een
/// ongedraaide pagina), en zet daarna eventueel /Rotate.
fn make_pdf(path: &std::path::Path, t: &Affine, rotate: Option<PdfPageRenderRotation>) {
    let p = pdf::pdfium().expect("pdfium.dll");
    let mut doc = p.create_new_pdf().unwrap();
    let (wpt, hpt) = (W_MM * PT_PER_MM, H_MM * PT_PER_MM);
    let mut page = doc
        .pages_mut()
        .create_page_at_end(PdfPagePaperSize::Custom(
            PdfPoints::new(wpt as f32),
            PdfPoints::new(hpt as f32),
        ))
        .unwrap();
    let to_user = |x: f64, y: f64| {
        let (dx, dy) = t.apply(x * PT_PER_MM, y * PT_PER_MM);
        (PdfPoints::new(dx as f32), PdfPoints::new((hpt - dy) as f32))
    };
    let line = |page: &mut PdfPage, x0: f64, y0: f64, x1: f64, y1: f64, w: f32| {
        let (a, b) = to_user(x0, y0);
        let (c, d) = to_user(x1, y1);
        page.objects_mut()
            .create_path_object_line(a, b, c, d, PdfColor::BLACK, PdfPoints::new(w))
            .unwrap();
    };
    // kader
    for (x0, y0, x1, y1) in [
        (10.0, 10.0, 831.0, 10.0),
        (831.0, 10.0, 831.0, 584.0),
        (831.0, 584.0, 10.0, 584.0),
        (10.0, 584.0, 10.0, 10.0),
    ] {
        line(&mut page, x0, y0, x1, y1, 1.4);
    }
    // titelhoek rechtsonder
    for (x0, y0, x1, y1) in [
        (651.0, 524.0, 831.0, 524.0),
        (651.0, 524.0, 651.0, 584.0),
        (651.0, 554.0, 831.0, 554.0),
        (741.0, 554.0, 741.0, 584.0),
    ] {
        line(&mut page, x0, y0, x1, y1, 0.7);
    }
    // onregelmatig raster
    let xs = [60.0, 133.0, 171.0, 260.0, 302.0, 397.0, 455.0, 520.0, 611.0];
    let ys = [70.0, 118.0, 207.0, 260.0, 344.0, 431.0];
    for &x in &xs {
        line(&mut page, x, 50.0, x, 470.0, 0.35);
    }
    for &y in &ys {
        line(&mut page, 40.0, y, 630.0, y, 0.35);
    }
    // diagonalen en een "gebouw"
    line(&mut page, 133.0, 118.0, 302.0, 344.0, 0.5);
    line(&mut page, 455.0, 70.0, 611.0, 431.0, 0.5);
    for i in 0..12 {
        let y = 150.0 + i as f64 * 9.5;
        line(&mut page, 330.0, y, 430.0 - i as f64 * 3.0, y, 0.25);
    }
    if let Some(r) = rotate {
        page.set_rotation(r);
    }
    doc.save_to_file(path).unwrap();
}

fn check(
    scale: f64,
    dx_mm: f64,
    dy_mm: f64,
    rot: Option<PdfPageRenderRotation>,
    rot_expect: Rot90,
) {
    let dir = std::env::temp_dir().join(format!("sheetdiff-reg-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let fa = dir.join(format!("a_{scale}_{dx_mm}_{dy_mm}_{rot_expect:?}.pdf"));
    let fb = dir.join(format!("b_{scale}_{dx_mm}_{dy_mm}_{rot_expect:?}.pdf"));
    make_pdf(&fa, &Affine::IDENTITY, None);
    let tb = Affine::scale(scale).then(&Affine::translate(dx_mm * PT_PER_MM, dy_mm * PT_PER_MM));
    make_pdf(&fb, &tb, rot);
    let (da, db) = (Doc::open(&fa).unwrap(), Doc::open(&fb).unwrap());
    let (pa, pb) = (da.page(0).unwrap(), db.page(0).unwrap());
    let (ga, gb) = (pdf::page_geom(&pa).unwrap(), pdf::page_geom(&pb).unwrap());
    let reg = register(&pa, &ga, &pb, &gb, &RegParams::default()).unwrap();
    assert!(reg.ok, "{}", reg.note);
    assert_eq!(reg.rotation, rot_expect, "{}", reg.note);
    // controlepunten: waar ligt een A-punt op B (weergave), en komt het terug?
    let (wu, hu) = gb.unrotated_size();
    let b_disp = tb.then(&gb.rotate.affine(wu, hu));
    let worst_of = |t: &Affine| {
        let mut worst: f64 = 0.0;
        for (x, y) in [
            (60.0, 70.0),
            (611.0, 431.0),
            (831.0, 584.0),
            (10.0, 10.0),
            (397.0, 260.0),
        ] {
            let p = (x * PT_PER_MM, y * PT_PER_MM);
            let q = b_disp.apply(p.0, p.1);
            let back = t.apply(q.0, q.1);
            let err = ((back.0 - p.0).powi(2) + (back.1 - p.1).powi(2)).sqrt() / PT_PER_MM;
            worst = worst.max(err);
        }
        worst
    };
    let worst = worst_of(&reg.transform);
    assert!(worst < 0.15, "fout {worst:.3} mm, {}", reg.note);
    // negatieve controle: zonder uitlijning moet het bij een verschoven blad wel misgaan,
    // anders meet deze test niets
    let trivial = scale == 1.0 && dx_mm == 0.0 && dy_mm == 0.0 && rot.is_none();
    if !trivial {
        assert!(
            worst_of(&Affine::IDENTITY) > 1.0,
            "controle niet onderscheidend"
        );
    } else {
        assert!(reg.identity, "gelijk blad moet exact identiteit geven");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn identity_is_exact() {
    check(1.0, 0.0, 0.0, None, Rot90::R0);
}

#[test]
fn shift_only() {
    check(1.0, 3.2, -1.7, None, Rot90::R0);
}

#[test]
fn shift_and_plotter_scale() {
    check(0.985, 4.0, 2.5, None, Rot90::R0);
}

#[test]
fn rotated_page_like_bluebeam() {
    check(
        1.0,
        0.0,
        0.0,
        Some(PdfPageRenderRotation::Degrees90),
        Rot90::R270,
    );
}

#[test]
fn rotated_scaled_and_shifted() {
    check(
        1.01,
        -2.0,
        3.0,
        Some(PdfPageRenderRotation::Degrees270),
        Rot90::R90,
    );
}
