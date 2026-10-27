//! Genereert beeldmateriaal voor de HTML-mockup uit echte uitgiftes (alleen lokaal gebruiken).
//!
//! mockup_assets <A.pdf> <B.pdf> <opmerkingen.pdf> <opmerkingen.json> <uitmap> <held-blad>
use anyhow::Result;
use serde_json::{json, Value};
use sheetdiff_core::config::Config;
use sheetdiff_core::geom::{Affine, RectPt, MM_PER_PT, PT_PER_MM};
use sheetdiff_core::identify::Identifier;
use sheetdiff_core::overlay::{self, OverlayColors, ViewMode};
use sheetdiff_core::pdf::{self, Doc};
use sheetdiff_core::register;
use std::collections::BTreeMap;
use std::path::PathBuf;

fn ids(doc: &Doc, ident: &Identifier) -> Result<BTreeMap<String, usize>> {
    let mut m = BTreeMap::new();
    for p in 0..doc.page_count() {
        let page = doc.page(p)?;
        let g = pdf::page_geom(&page)?;
        let runs = pdf::text_runs(&page, &g)?;
        if let Some(n) = ident.identify(&runs, g.display_size(), "").number {
            m.entry(n).or_insert(p);
        }
    }
    Ok(m)
}

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let (fa, fb, fc, fj, out, hero) = (
        PathBuf::from(&a[1]),
        PathBuf::from(&a[2]),
        PathBuf::from(&a[3]),
        PathBuf::from(&a[4]),
        PathBuf::from(&a[5]),
        a[6].clone(),
    );
    std::fs::create_dir_all(&out)?;
    let cfg = Config::default();
    let ident = Identifier::new(&cfg.identify)?;
    let (da, db, dc) = (Doc::open(&fa)?, Doc::open(&fb)?, Doc::open(&fc)?);
    let (ia, ib, ic) = (ids(&da, &ident)?, ids(&db, &ident)?, ids(&dc, &ident)?);
    let colors = OverlayColors::default();
    let mut manifest = json!({"sheets": [], "comments": [], "hero": hero});

    // thumbnails per blad
    for (nr, &pa) in &ia {
        let Some(&pb) = ib.get(nr) else { continue };
        let (pga, pgb) = (da.page(pa)?, db.page(pb)?);
        let (ga, gb) = (pdf::page_geom(&pga)?, pdf::page_geom(&pgb)?);
        let reg = register::register(&pga, &ga, &pgb, &gb, &cfg.register)?;
        let (w, h) = ga.display_size();
        let dpi = if *nr == hero { 72.0 } else { 18.0 };
        let s = dpi / 72.0;
        let (pw, ph) = ((w * s).round() as usize, (h * s).round() as usize);
        let to_px = Affine::scale(s);
        let modes: Vec<(&str, ViewMode)> = if *nr == hero {
            vec![
                ("diff", ViewMode::Diff),
                ("a", ViewMode::A),
                ("b", ViewMode::B),
            ]
        } else {
            vec![("diff", ViewMode::Diff)]
        };
        for (name, mode) in modes {
            let (rgba, _, _) = overlay::render_tile(
                Some(&pga),
                Some(&pgb),
                &reg.transform,
                &to_px,
                pw,
                ph,
                &cfg.compare.diff,
                mode,
                &colors,
                0.55,
                0.5,
            )?;
            overlay::save_png(&out.join(format!("{nr}_{name}.png")), rgba, pw, ph)?;
        }
        manifest["sheets"].as_array_mut().unwrap().push(json!({
            "nr": nr, "w_mm": w * MM_PER_PT, "h_mm": h * MM_PER_PT, "reg": reg.note, "score": reg.score
        }));
        eprintln!("blad {nr}: {}", reg.note);
    }

    // opmerkingen: tekstannotaties met aangehechte vormen
    let annots: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(&fj)?)?;
    let mut regs: BTreeMap<usize, (String, usize, register::Registration)> = BTreeMap::new();
    let mut n = 0;
    for (ci, t) in annots.iter().enumerate() {
        let content = t["content"].as_str().unwrap_or("");
        if content.trim().is_empty() || t["author"] == "Jochem" {
            continue;
        }
        let cpage = t["page"].as_u64().unwrap() as usize;
        let r = |v: &Value| {
            let a = v["rect"].as_array().unwrap();
            RectPt::new(
                a[0].as_f64().unwrap(),
                a[1].as_f64().unwrap(),
                a[2].as_f64().unwrap(),
                a[3].as_f64().unwrap(),
            )
        };
        let tr = r(t);
        // vormen zonder tekst binnen 40 mm, en Jochems afvinking
        let mut zone = tr;
        let mut done_by_jochem = false;
        for (k, s) in annots.iter().enumerate() {
            if k == ci || s["page"] != t["page"] {
                continue;
            }
            let sr = r(s);
            if s["author"] == "Jochem" {
                if sr.intersects(&tr.expand(5.0 * PT_PER_MM)) {
                    done_by_jochem = true;
                }
                continue;
            }
            if s["content"].as_str().unwrap_or("").trim().is_empty()
                && sr.distance(&tr) < 40.0 * PT_PER_MM
                && sr.area() < 2.0e6
            {
                zone = zone.union(&sr);
            }
        }
        // welk blad is deze opmerkingenpagina?
        let Some(nr) = ic.iter().find(|(_, &p)| p == cpage).map(|(k, _)| k.clone()) else {
            eprintln!("opmerking {ci} op pagina {cpage}: geen tekeningnummer (voorblad?)");
            manifest["comments"].as_array_mut().unwrap().push(json!({
                "id": ci, "page": cpage, "sheet": null, "text": content, "subject": t["subject"],
                "author": t["author"], "date": t["date"], "proposal": "ONDUIDELIJK",
                "reason": "algemene opmerking zonder plek op een blad", "jochem": done_by_jochem
            }));
            continue;
        };
        let (Some(&pa), Some(&pb)) = (ia.get(&nr), ib.get(&nr)) else {
            continue;
        };
        let (pga, pgb, pgc) = (da.page(pa)?, db.page(pb)?, dc.page(cpage)?);
        let (ga, gb, gc) = (
            pdf::page_geom(&pga)?,
            pdf::page_geom(&pgb)?,
            pdf::page_geom(&pgc)?,
        );
        let regc = &regs
            .entry(cpage)
            .or_insert_with(|| {
                let r = register::register(&pga, &ga, &pgc, &gc, &cfg.register).unwrap();
                eprintln!("opm-pagina {cpage} -> {nr}: {}", r.note);
                (nr.clone(), pa, r)
            })
            .2
            .clone();
        let regb = register::register(&pga, &ga, &pgb, &gb, &cfg.register)?;
        let zone_a = zone.transform(&regc.transform);
        let crop = zone_a.expand(20.0 * PT_PER_MM);
        let dpi = 110.0;
        let s = dpi / 72.0;
        let (w, h) = (
            (crop.width() * s).round() as usize,
            (crop.height() * s).round() as usize,
        );
        let to_px = Affine::translate(-crop.x0, -crop.y0).then(&Affine::scale(s));
        // opmerkingen-pdf met annotaties in A-kader
        let c_rgba = pdf::render_rgba(&pgc, &regc.transform.then(&to_px), w, h, true)?;
        overlay::save_png(&out.join(format!("opm{ci}_c.png")), c_rgba.data, w, h)?;
        for (name, mode) in [
            ("a", ViewMode::A),
            ("b", ViewMode::B),
            ("diff", ViewMode::Diff),
        ] {
            let (rgba, _, _) = overlay::render_tile(
                Some(&pga),
                Some(&pgb),
                &regb.transform,
                &to_px,
                w,
                h,
                &cfg.compare.diff,
                mode,
                &colors,
                0.55,
                0.5,
            )?;
            overlay::save_png(&out.join(format!("opm{ci}_{name}.png")), rgba, w, h)?;
        }
        // verschil binnen de zone (+ marge) op analyse-dpi
        let z = zone_a.expand(cfg.comments.zone_margin_mm * PT_PER_MM);
        let s3 = 300.0 / 72.0;
        let (zw, zh) = (
            (z.width() * s3).round() as usize,
            (z.height() * s3).round() as usize,
        );
        let (_, rem, add) = overlay::render_tile(
            Some(&pga),
            Some(&pgb),
            &regb.transform,
            &Affine::translate(-z.x0, -z.y0).then(&Affine::scale(s3)),
            zw,
            zh,
            &cfg.compare.diff,
            ViewMode::Diff,
            &colors,
            0.5,
            0.5,
        )?;
        let total = rem + add;
        let (proposal, reason) = if total > 400 {
            (
                "WAARSCHIJNLIJK VERWERKT",
                format!("{total} verschilpixels A->B in de opmerkingszone"),
            )
        } else if total > 40 {
            (
                "ONDUIDELIJK",
                format!("klein verschil ({total} px) in de zone"),
            )
        } else {
            (
                "NIET VERWERKT",
                "geen verschil A->B in de opmerkingszone".to_string(),
            )
        };
        let zr = |r: &RectPt| {
            json!([
                (r.x0 - crop.x0) * s,
                (r.y0 - crop.y0) * s,
                (r.x1 - crop.x0) * s,
                (r.y1 - crop.y0) * s
            ])
        };
        manifest["comments"].as_array_mut().unwrap().push(json!({
            "id": ci, "page": cpage, "sheet": nr, "text": content, "subject": t["subject"], "author": t["author"],
            "date": t["date"], "proposal": proposal, "reason": reason, "removed": rem, "added": add,
            "zone_px": zr(&zone_a), "img_w": w, "img_h": h, "jochem": done_by_jochem,
            "reg_c": regc.note, "zone_mm": [zone_a.x0 * MM_PER_PT, zone_a.y0 * MM_PER_PT]
        }));
        n += 1;
        eprintln!(
            "opmerking {ci} ({nr}): {proposal} - {reason} - '{}'",
            content.replace('\n', " ")
        );
    }
    std::fs::write(
        out.join("manifest.json"),
        serde_json::to_string_pretty(&manifest)?,
    )?;
    eprintln!("{n} opmerkingen met plek");
    Ok(())
}
