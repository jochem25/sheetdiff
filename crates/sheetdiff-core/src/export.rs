//! Export: opmerkingenlijst (xlsx/csv), verschillenlijst (xlsx) en PDF-kopieen met
//! annotaties (opmerkingen met status op B, of revisiewolken). Annotaties worden als
//! incrementele update toegevoegd: de oorspronkelijke inhoud blijft byte-voor-byte staan en de
//! markeringen zijn in Bluebeam/Acrobat te verwijderen. Het bronbestand wordt nooit
//! overschreven.

use crate::comments::{CommentsResult, Proposal, ReviewState, Status};
use crate::compare::{IssueDiff, Verdict};
use crate::geom::{PageGeom, RectPt, PT_PER_MM};
use crate::issue::Issue;
use crate::textdiff::TextChangeKind;
use anyhow::{bail, Context, Result};
use lopdf::{dictionary, Object, StringFormat};
use rust_xlsxwriter::{Color, Format, FormatAlign, Workbook};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn status_color(s: Status, p: Proposal) -> [f32; 3] {
    match s {
        Status::Done => [0.05, 0.41, 0.38],          // petrol
        Status::NotDone => [0.65, 0.20, 0.17],       // rood
        Status::Question => [0.56, 0.42, 0.12],      // oker
        Status::NotApplicable => [0.42, 0.47, 0.46], // grijs
        Status::Open => match p {
            Proposal::LikelyDone => [0.06, 0.63, 0.55],
            Proposal::NotDone => [0.86, 0.12, 0.12],
            _ => [0.56, 0.42, 0.12],
        },
    }
}

pub fn comments_csv(r: &CommentsResult, st: &ReviewState) -> String {
    let f = |s: &str| {
        // Excel voert een cel die met = + - @ begint uit als formule: neutraliseren
        let s = if s.starts_with(['=', '+', '-', '@']) {
            format!("'{s}")
        } else {
            s.to_string()
        };
        let s = s.as_str();
        if s.contains([';', '"', '\n', '\r']) {
            format!("\"{}\"", s.replace('"', "\"\""))
        } else {
            s.to_string()
        }
    };
    let mut o = String::from("\u{feff}nr;blad;soort;tekst aannemer;auteur;voorstel;reden;status;notitie;tekstwijzigingen;eigen markering\n");
    for c in &r.comments {
        let e = st.entry(&c.id);
        o.push_str(&format!(
            "{};{};{:?};{};{};{};{};{};{};{};{}\n",
            c.number,
            f(c.sheet.as_deref().unwrap_or("algemeen")),
            c.kind,
            f(&c.text),
            f(&c.author),
            c.proposal.label(),
            f(&c.reason),
            e.status.label(),
            f(&e.note),
            f(&c.text_changes.join(" | ")),
            f(&c.own_marks.join(" | ")),
        ));
    }
    o
}

pub fn comments_xlsx(r: &CommentsResult, st: &ReviewState, path: &Path) -> Result<()> {
    let mut wb = Workbook::new();
    let ws = wb.add_worksheet().set_name("Opmerkingen")?;
    let head = Format::new()
        .set_bold()
        .set_background_color(Color::RGB(0xE7F2EF))
        .set_font_color(Color::RGB(0x0D6862));
    let wrap = Format::new().set_text_wrap().set_align(FormatAlign::Top);
    let cols = [
        ("Nr", 5.0),
        ("Blad", 10.0),
        ("Tekst aannemer", 50.0),
        ("Auteur", 14.0),
        ("Voorstel", 20.0),
        ("Reden", 34.0),
        ("Status Jochem", 14.0),
        ("Notitie", 36.0),
        ("Tekstwijzigingen A->B", 30.0),
        ("Eigen markering in PDF", 24.0),
    ];
    for (i, (h, w)) in cols.iter().enumerate() {
        ws.write_with_format(0, i as u16, *h, &head)?;
        ws.set_column_width(i as u16, *w)?;
    }
    for (row, c) in r.comments.iter().enumerate() {
        let e = st.entry(&c.id);
        let rr = (row + 1) as u32;
        ws.write_number_with_format(rr, 0, c.number as f64, &wrap)?;
        ws.write_with_format(rr, 1, c.sheet.as_deref().unwrap_or("algemeen"), &wrap)?;
        ws.write_with_format(rr, 2, &c.text, &wrap)?;
        ws.write_with_format(rr, 3, &c.author, &wrap)?;
        ws.write_with_format(rr, 4, c.proposal.label(), &wrap)?;
        ws.write_with_format(rr, 5, &c.reason, &wrap)?;
        ws.write_with_format(rr, 6, e.status.label(), &wrap)?;
        ws.write_with_format(rr, 7, &e.note, &wrap)?;
        ws.write_with_format(rr, 8, c.text_changes.join("\n"), &wrap)?;
        ws.write_with_format(rr, 9, c.own_marks.join("\n"), &wrap)?;
    }
    ws.set_freeze_panes(1, 0)?;
    ws.autofilter(0, 0, r.comments.len() as u32, (cols.len() - 1) as u16)?;
    wb.save(path)
        .with_context(|| format!("xlsx niet te schrijven: {}", path.display()))?;
    Ok(())
}

pub fn diff_xlsx(d: &IssueDiff, path: &Path) -> Result<()> {
    let mut wb = Workbook::new();
    let ws = wb.add_worksheet().set_name("Bladen")?;
    let head = Format::new()
        .set_bold()
        .set_background_color(Color::RGB(0xE7F2EF))
        .set_font_color(Color::RGB(0x0D6862));
    let hdr = [
        "Blad",
        "Titel",
        "Oordeel",
        "Pixels %",
        "Regio's",
        "Gewijzigd",
        "Toegevoegd",
        "Verwijderd",
        "Verplaatst",
        "Uitlijning",
        "Boekjes",
        "Redenen",
    ];
    for (i, h) in hdr.iter().enumerate() {
        ws.write_with_format(0, i as u16, *h, &head)?;
    }
    for (row, s) in d.sheets.iter().enumerate() {
        let r = (row + 1) as u32;
        let n = |k| s.text.iter().filter(|t| t.kind == k).count() as f64;
        ws.write(r, 0, &s.key)?;
        ws.write(r, 1, &s.title)?;
        ws.write(r, 2, s.verdict.label())?;
        ws.write_number(r, 3, s.pixel_pct)?;
        ws.write_number(r, 4, s.regions.iter().filter(|x| !x.stamp).count() as f64)?;
        ws.write_number(r, 5, n(TextChangeKind::Changed))?;
        ws.write_number(r, 6, n(TextChangeKind::Added))?;
        ws.write_number(r, 7, n(TextChangeKind::Removed))?;
        ws.write_number(r, 8, n(TextChangeKind::Moved))?;
        ws.write(
            r,
            9,
            s.registration
                .as_ref()
                .map(|x| x.note.as_str())
                .unwrap_or(""),
        )?;
        ws.write(
            r,
            10,
            s.b.as_ref()
                .or(s.a.as_ref())
                .map(|l| l.booklets.join(", "))
                .unwrap_or_default(),
        )?;
        ws.write(r, 11, s.reasons.join(" | "))?;
    }
    let wt = wb.add_worksheet().set_name("Tekst")?;
    for (i, h) in ["Blad", "Soort", "Oud", "Nieuw", "x mm", "y mm"]
        .iter()
        .enumerate()
    {
        wt.write_with_format(0, i as u16, *h, &head)?;
    }
    let mut r = 1u32;
    for s in d.sheets.iter().filter(|s| s.verdict == Verdict::Changed) {
        for t in s.text.iter().filter(|t| t.kind != TextChangeKind::Stamp) {
            wt.write(r, 0, &s.key)?;
            wt.write(r, 1, format!("{:?}", t.kind))?;
            wt.write(r, 2, t.old.as_deref().unwrap_or(""))?;
            wt.write(r, 3, t.new.as_deref().unwrap_or(""))?;
            wt.write_number(r, 4, (t.bbox.x0 / PT_PER_MM).round())?;
            wt.write_number(r, 5, (t.bbox.y0 / PT_PER_MM).round())?;
            r += 1;
        }
    }
    wb.save(path)?;
    Ok(())
}

/// Vrije bestandsnaam in `dir`: `<stem><suffix>.pdf`, of met (2), (3) ... als die al bestaat.
/// Een export overschrijft nooit iets, ook geen eerdere export of een invoerbestand.
pub fn free_name(dir: &Path, stem: &str, suffix: &str) -> PathBuf {
    let first = dir.join(format!("{stem}{suffix}.pdf"));
    if !first.exists() {
        return first;
    }
    (2..10_000)
        .map(|n| dir.join(format!("{stem}{suffix} ({n}).pdf")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}

/// Een annotatie die op een pagina komt (rechthoek in weergavepunten van die pagina).
#[derive(Debug, Clone)]
pub struct NewAnnot {
    pub page: usize,
    pub rect: RectPt,
    pub color: [f32; 3],
    pub contents: String,
    /// Kort label boven de rechthoek (FreeText); leeg = geen label.
    pub label: String,
    pub cloudy: bool,
    pub nm: String,
}

fn pdf_text(s: &str) -> Object {
    // UTF-16BE met BOM: werkt voor alle tekens
    let mut b = vec![0xFE, 0xFF];
    for u in s.encode_utf16() {
        b.extend_from_slice(&u.to_be_bytes());
    }
    Object::String(b, StringFormat::Hexadecimal)
}

/// Schrijft een kopie van `src` met de annotaties als incrementele update.
pub fn annotate_copy(
    src: &Path,
    out: &Path,
    geoms: &[PageGeom],
    annots: &[NewAnnot],
) -> Result<()> {
    if src.canonicalize().ok() == out.canonicalize().ok() || src == out || out.exists() {
        bail!(
            "uitvoer bestaat al of is de bron; er wordt nooit overschreven: {}",
            out.display()
        );
    }
    let bytes = std::fs::read(src)?;
    let prev = lopdf::Document::load_mem(&bytes)
        .with_context(|| format!("PDF niet te lezen: {}", src.display()))?;
    let pages = prev.get_pages();
    let mut inc = lopdf::IncrementalDocument::create_from(bytes, prev);
    let mut by_page: BTreeMap<usize, Vec<&NewAnnot>> = BTreeMap::new();
    for a in annots {
        by_page.entry(a.page).or_default().push(a);
    }
    for (p, list) in by_page {
        let Some(&page_id) = pages.get(&((p + 1) as u32)) else {
            bail!("pagina {} bestaat niet in {}", p + 1, src.display());
        };
        let geom = geoms
            .get(p)
            .with_context(|| format!("geen geometrie voor pagina {}", p + 1))?;
        let d2u = geom
            .user_to_display()
            .inverse()
            .context("paginageometrie niet inverteerbaar")?;
        let mut new_refs = Vec::new();
        for a in list {
            let r = a.rect.transform(&d2u);
            let rect = vec![r.x0.into(), r.y0.into(), r.x1.into(), r.y1.into()];
            let color: Vec<Object> = a.color.iter().map(|&c| Object::Real(c)).collect();
            let mut sq = dictionary! {
                "Type" => "Annot",
                "Subtype" => "Square",
                "Rect" => rect,
                "C" => color.clone(),
                "BS" => dictionary! { "W" => 2.0 },
                "F" => 4,
                "T" => pdf_text("sheetdiff"),
                "Subj" => pdf_text("sheetdiff"),
                "NM" => pdf_text(&a.nm),
                "Contents" => pdf_text(&a.contents),
                "P" => page_id,
            };
            if a.cloudy {
                sq.set("BE", dictionary! { "S" => "C", "I" => 1.0 });
            }
            // appearance stream, zodat elke viewer de rechthoek toont (ook zonder eigen
            // generator); Acrobat/Bluebeam tekenen bij bewerken hun eigen wolk.
            let (w, h) = (r.width(), r.height());
            let ap = format!(
                "q {} {} {} RG 2 w 1 1 {:.2} {:.2} re S Q",
                a.color[0],
                a.color[1],
                a.color[2],
                (w - 2.0).max(0.1),
                (h - 2.0).max(0.1)
            );
            let ap_id = inc.new_document.add_object(lopdf::Stream::new(
                dictionary! {
                    "Type" => "XObject",
                    "Subtype" => "Form",
                    "BBox" => vec![0.into(), 0.into(), w.into(), h.into()],
                },
                ap.into_bytes(),
            ));
            // BBox [0 0 w h] wordt door de viewer op /Rect gepast (PDF-spec, algoritme 8.1).
            sq.set("AP", dictionary! { "N" => ap_id });
            let id = inc.new_document.add_object(sq);
            new_refs.push(Object::Reference(id));
            if !a.label.is_empty() && geom.rotate == crate::geom::Rot90::R0 {
                // label boven de rechthoek, in weergaveruimte, daarna naar gebruikersruimte
                let lab = RectPt::new(
                    a.rect.x0,
                    a.rect.y0 - 14.0,
                    a.rect.x0 + 9.0 * 0.5 * a.label.chars().count() as f64 + 10.0,
                    a.rect.y0 - 1.0,
                )
                .transform(&d2u);
                let (lw, lh) = (lab.width(), lab.height());
                let latin: String = a
                    .label
                    .chars()
                    .map(|c| {
                        if (c as u32) < 128 && c != '(' && c != ')' && c != '\\' {
                            c
                        } else {
                            '?'
                        }
                    })
                    .collect();
                let ap = format!(
                    "q 1 1 1 rg 0 0 {lw:.2} {lh:.2} re f BT /Helv 9 Tf {} {} {} rg 3 3.5 Td ({latin}) Tj ET Q",
                    a.color[0], a.color[1], a.color[2]
                );
                let ap_id = inc.new_document.add_object(lopdf::Stream::new(
                    dictionary! {
                        "Type" => "XObject",
                        "Subtype" => "Form",
                        "BBox" => vec![0.into(), 0.into(), lw.into(), lh.into()],
                        "Resources" => dictionary! { "Font" => dictionary! { "Helv" => dictionary! {
                            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding" } } },
                    },
                    ap.into_bytes(),
                ));
                let ft = dictionary! {
                    "AP" => dictionary! { "N" => ap_id },
                    "Type" => "Annot",
                    "Subtype" => "FreeText",
                    "Rect" => vec![lab.x0.into(), lab.y0.into(), lab.x1.into(), lab.y1.into()],
                    "DA" => Object::string_literal(format!("/Helv 9 Tf {} {} {} rg", a.color[0], a.color[1], a.color[2])),
                    "C" => vec![Object::Real(1.0), Object::Real(1.0), Object::Real(1.0)],
                    "F" => 4,
                    "T" => pdf_text("sheetdiff"),
                    "Subj" => pdf_text("sheetdiff"),
                    "NM" => pdf_text(&format!("{}-label", a.nm)),
                    "Contents" => pdf_text(&a.label),
                    "P" => page_id,
                };
                let id = inc.new_document.add_object(ft);
                new_refs.push(Object::Reference(id));
            }
        }
        // /Annots van de pagina uitbreiden (inline array of verwijzing naar een array)
        inc.opt_clone_object_to_new_document(page_id)?;
        let annots_obj = inc
            .get_prev_documents()
            .get_dictionary(page_id)?
            .get(b"Annots")
            .ok()
            .cloned();
        match annots_obj {
            Some(Object::Reference(arr_id)) => {
                inc.opt_clone_object_to_new_document(arr_id)?;
                if let Ok(Object::Array(a)) = inc.new_document.get_object_mut(arr_id) {
                    a.extend(new_refs);
                }
            }
            Some(Object::Array(mut a)) => {
                a.extend(new_refs);
                inc.new_document
                    .get_dictionary_mut(page_id)?
                    .set("Annots", Object::Array(a));
            }
            _ => {
                inc.new_document
                    .get_dictionary_mut(page_id)?
                    .set("Annots", Object::Array(new_refs));
            }
        }
    }
    inc.save(out)
        .with_context(|| format!("PDF niet te schrijven: {}", out.display()))?;
    Ok(())
}

/// PDF-kopie(en) van B met de opmerkingen en hun status. Een bestand per B-bestand met
/// opmerkingen: `<naam>_opmerkingen.pdf` in `out_dir`.
/// Opmerkingen die geen plek op B hebben (algemeen, blad niet in B, niet uit te lijnen) en
/// dus niet in de PDF-export staan; de aanroeper meldt ze.
pub fn comments_without_place(r: &CommentsResult) -> Vec<usize> {
    r.comments
        .iter()
        // akkoordvinkjes en stempels horen bewust geen plek te hebben
        .filter(|c| c.zone_b.is_none() && c.kind != crate::comments::CommentKind::Info)
        .map(|c| c.number)
        .collect()
}

pub fn comments_on_b(
    r: &CommentsResult,
    st: &ReviewState,
    b: &Issue,
    out_dir: &Path,
) -> Result<Vec<PathBuf>> {
    let mut per_file: BTreeMap<PathBuf, (Vec<NewAnnot>, BTreeMap<usize, PageGeom>)> =
        BTreeMap::new();
    for c in &r.comments {
        let (Some(nr), Some(zb)) = (&c.sheet, c.zone_b) else {
            continue;
        };
        let Some(sb) = b.sheets.get(nr) else { continue };
        let e = st.entry(&c.id);
        let status = if e.status == Status::Open {
            "open".to_string()
        } else {
            e.status.label().to_string()
        };
        let entry = per_file.entry(sb.file.clone()).or_default();
        entry.1.insert(sb.page, sb.geom);
        entry.0.push(NewAnnot {
            page: sb.page,
            rect: zb.expand(3.0 * PT_PER_MM),
            color: status_color(e.status, c.proposal),
            contents: format!(
                "Opmerking #{} ({}): {}\nVoorstel: {} - {}\nStatus: {}{}",
                c.number,
                c.author,
                c.text,
                c.proposal.label(),
                c.reason,
                status,
                if e.note.is_empty() {
                    String::new()
                } else {
                    format!("\nNotitie: {}", e.note)
                }
            ),
            label: format!("#{} {}", c.number, status),
            cloudy: true,
            nm: format!("sheetdiff-{}", c.id),
        });
    }
    std::fs::create_dir_all(out_dir)?;
    let mut out = Vec::new();
    for (file, (annots, geoms_map)) in per_file {
        let n = geoms_map.keys().max().copied().unwrap_or(0) + 1;
        let mut geoms = vec![
            PageGeom {
                crop: [0.0, 0.0, 1.0, 1.0],
                rotate: Default::default()
            };
            n
        ];
        for (p, g) in geoms_map {
            geoms[p] = g;
        }
        let stem = file.file_stem().unwrap_or_default().to_string_lossy();
        let dst = free_name(out_dir, &stem, "_opmerkingen");
        annotate_copy(&file, &dst, &geoms, &annots)?;
        out.push(dst);
    }
    Ok(out)
}

/// Revisiewolken om de verschilregio's (niet-stempel) op een kopie van B.
pub fn clouds_on_b(d: &IssueDiff, b: &Issue, out_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut per_file: BTreeMap<PathBuf, (Vec<NewAnnot>, BTreeMap<usize, PageGeom>)> =
        BTreeMap::new();
    for s in d.sheets.iter().filter(|s| s.verdict == Verdict::Changed) {
        let Some(sb) = b.sheets.get(&s.key) else {
            continue;
        };
        let Some(reg) = &s.registration else { continue };
        let Some(inv) = reg.transform.inverse() else {
            continue;
        };
        let entry = per_file.entry(sb.file.clone()).or_default();
        entry.1.insert(sb.page, sb.geom);
        for (i, r) in s.regions.iter().filter(|r| !r.stamp).enumerate() {
            let texts: Vec<String> = r
                .text
                .iter()
                .filter_map(|&k| s.text.get(k))
                .filter(|t| t.kind != TextChangeKind::Stamp)
                .map(|t| match (&t.old, &t.new) {
                    (Some(o), Some(n)) => format!("{o} -> {n}"),
                    (Some(o), None) => format!("weg: {o}"),
                    (None, Some(n)) => format!("nieuw: {n}"),
                    _ => String::new(),
                })
                .collect();
            entry.0.push(NewAnnot {
                page: sb.page,
                rect: r.bbox.transform(&inv).expand(5.0 * PT_PER_MM),
                color: [1.0, 0.0, 0.0],
                contents: format!(
                    "W-{}-{:02}: {} px weg, {} px erbij{}",
                    s.key,
                    i + 1,
                    r.removed_px,
                    r.added_px,
                    if texts.is_empty() {
                        String::new()
                    } else {
                        format!("\n{}", texts.join("\n"))
                    }
                ),
                label: String::new(),
                cloudy: true,
                nm: format!("sheetdiff-{}-{}", s.key, i + 1),
            });
        }
    }
    // tekstwijzigingen die in geen enkele regio vallen (bijv. alleen tekst gewijzigd)
    for s in d.sheets.iter().filter(|s| s.verdict == Verdict::Changed) {
        let Some(sb) = b.sheets.get(&s.key) else {
            continue;
        };
        let Some(inv) = s.registration.as_ref().and_then(|r| r.transform.inverse()) else {
            continue;
        };
        let entry = per_file.entry(sb.file.clone()).or_default();
        entry.1.insert(sb.page, sb.geom);
        for (k, t) in s.text.iter().enumerate() {
            let in_region = s.regions.iter().any(|r| !r.stamp && r.text.contains(&k));
            if in_region || matches!(t.kind, TextChangeKind::Stamp | TextChangeKind::Moved) {
                continue;
            }
            entry.0.push(NewAnnot {
                page: sb.page,
                rect: t.bbox.transform(&inv).expand(3.0 * PT_PER_MM),
                color: [1.0, 0.0, 0.0],
                contents: format!(
                    "tekst: {} -> {}",
                    t.old.as_deref().unwrap_or("-"),
                    t.new.as_deref().unwrap_or("-")
                ),
                label: String::new(),
                cloudy: true,
                nm: format!("sheetdiff-{}-t{}", s.key, k),
            });
        }
    }
    per_file.retain(|_, (a, _)| !a.is_empty());
    std::fs::create_dir_all(out_dir)?;
    let mut out = Vec::new();
    for (file, (annots, geoms_map)) in per_file {
        let n = geoms_map.keys().max().copied().unwrap_or(0) + 1;
        let mut geoms = vec![
            PageGeom {
                crop: [0.0, 0.0, 1.0, 1.0],
                rotate: Default::default()
            };
            n
        ];
        for (p, g) in geoms_map {
            geoms[p] = g;
        }
        let stem = file.file_stem().unwrap_or_default().to_string_lossy();
        let dst = free_name(out_dir, &stem, "_wijzigingen");
        annotate_copy(&file, &dst, &geoms, &annots)?;
        out.push(dst);
    }
    Ok(out)
}
