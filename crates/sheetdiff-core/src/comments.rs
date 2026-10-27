//! Opmerkingen van de aannemer: uitlezen, groeperen, uitlijnen op A en B, en per opmerking een
//! VOORSTEL of hij in B verwerkt is. Het voorstel is geen oordeel; de status zet de gebruiker
//! (opgeslagen in een json naast de PDF's).
//!
//! Twee vormen:
//! (a) PDF-annotaties (FreeText, Text, Polygon/wolk, Square, Circle, Ink, PolyLine/pijl, Line,
//!     Stamp) - gelezen met lopdf, zodat ook Bluebeam-velden (Subj) beschikbaar zijn;
//! (b) rood ingetekend in de inhoud zelf - gevonden als rode inkt in de opmerkingen-PDF die op
//!     dezelfde plek in A niet staat.

use crate::config::Config;
use crate::geom::{Affine, PageGeom, RectPt, MM_PER_PT, PT_PER_MM};
use crate::identify::Identifier;
use crate::issue::{Issue, SheetRef};
use crate::pdf::{self, Doc};
use crate::raster::{self, CoarseMask};
use crate::register::{self, Registration};
use crate::textdiff::{self, Normalizer, TextChangeKind};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Een annotatie zoals gelezen, in weergavepunten van de opmerkingenpagina.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawAnnot {
    pub page: usize,
    pub subtype: String,
    pub rect: RectPt,
    pub contents: String,
    pub subject: String,
    pub author: String,
    pub date: String,
    pub nm: String,
    /// /IRT: antwoord op een andere annotatie (NM of objectnummer als tekst).
    pub in_reply_to: Option<String>,
    /// /RT: "R" = antwoord, "Group" = onderdeel van een groep (Bluebeam: wolk bij tekstvak).
    #[serde(default)]
    pub reply_type: Option<String>,
    /// /State (Acrobat/Bluebeam reviewstatus), bijv. "Accepted", "Completed".
    pub state: Option<String>,
    pub object: (u32, u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommentKind {
    /// Annotatie met tekst (eventueel met wolk/pijl/rechthoek erbij).
    Annotation,
    /// Vorm zonder tekst die nergens bij hoort.
    MarkOnly,
    /// Akkoordvink of informatieve stempel.
    Info,
    /// Rood ingetekend in de inhoud.
    ContentMarkup,
    /// Alleen een verwijzingsnummer ("3", "12a"): wijst naar een lijst elders, geen opdracht
    /// op deze plek.
    Label,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord)]
pub enum Proposal {
    LikelyDone,
    NotDone,
    Unclear,
    Info,
}

impl Proposal {
    pub fn label(&self) -> &'static str {
        match self {
            Proposal::LikelyDone => "waarschijnlijk verwerkt",
            Proposal::NotDone => "niet verwerkt",
            Proposal::Unclear => "onduidelijk",
            Proposal::Info => "informatief",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comment {
    /// Stabiel id (NM van de tekstannotatie, of pagina + positie).
    pub id: String,
    pub number: usize,
    pub kind: CommentKind,
    pub c_page: usize,
    pub sheet: Option<String>,
    pub text: String,
    pub author: String,
    pub date: String,
    pub subject: String,
    /// Opmerkingszone op de opmerkingenpagina (weergavepunten).
    pub zone_c: RectPt,
    /// Dezelfde zone op A en op B (weergavepunten van die pagina's).
    pub zone_a: Option<RectPt>,
    pub zone_b: Option<RectPt>,
    /// Onderdelen (wolk, pijl, tekstvak) op de opmerkingenpagina.
    pub parts: Vec<RectPt>,
    /// Markeringen van eigen auteurs op deze plek (bijv. afvinken), informatief.
    pub own_marks: Vec<String>,
    pub proposal: Proposal,
    pub reason: String,
    pub diff_px: u64,
    /// Tekstwijzigingen A->B in de zone, als "oud -> nieuw".
    pub text_changes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommentsResult {
    pub file: PathBuf,
    pub comments: Vec<Comment>,
    /// Registratie per opmerkingenpagina op A.
    pub page_regs: BTreeMap<usize, (String, Registration)>,
    pub warnings: Vec<String>,
}

/// Leest alle annotaties met lopdf; coordinaten via de paginageometrie van PDFium.
pub fn read_annotations(path: &Path, geoms: &[PageGeom]) -> Result<Vec<RawAnnot>> {
    fn keep_dicts(id: (u32, u16), obj: &mut lopdf::Object) -> Option<((u32, u16), lopdf::Object)> {
        // inhoudsstromen niet in het geheugen houden; objectstromen en xref wel
        if let lopdf::Object::Stream(s) = obj {
            let t = s.dict.get(b"Type").ok().and_then(|t| t.as_name().ok());
            if !matches!(t, Some(b"ObjStm") | Some(b"XRef")) {
                return Some((
                    id,
                    lopdf::Object::Stream(lopdf::Stream::new(s.dict.clone(), Vec::new())),
                ));
            }
        }
        Some((id, obj.clone()))
    }
    let doc = lopdf::Document::load_with_options(path, lopdf::LoadOptions::with_filter(keep_dicts))
        .with_context(|| format!("opmerkingen-PDF niet te lezen: {}", path.display()))?;
    let mut out = Vec::new();
    for (pno, page_id) in doc.get_pages() {
        let p = (pno - 1) as usize;
        let Some(geom) = geoms.get(p) else { continue };
        let u2d = geom.user_to_display();
        let Ok(page) = doc.get_dictionary(page_id) else {
            continue;
        };
        let Ok(annots) = page.get(b"Annots") else {
            continue;
        };
        let annots = match doc.dereference(annots) {
            Ok((_, lopdf::Object::Array(a))) => a.clone(),
            _ => continue,
        };
        for a in annots {
            let (id, obj) = match &a {
                lopdf::Object::Reference(r) => (*r, doc.get_object(*r).ok()),
                o => ((0, 0), Some(o)),
            };
            let Some(lopdf::Object::Dictionary(d)) = obj else {
                continue;
            };
            let name = |k: &[u8]| {
                d.get(k)
                    .ok()
                    .and_then(|o| o.as_name().ok())
                    .map(|n| String::from_utf8_lossy(n).into_owned())
                    .unwrap_or_default()
            };
            let text = |k: &[u8]| {
                d.get(k)
                    .ok()
                    .and_then(|o| doc.dereference(o).ok())
                    .and_then(|(_, o)| lopdf::decode_text_string(o).ok())
                    .unwrap_or_default()
            };
            let subtype = name(b"Subtype");
            if matches!(subtype.as_str(), "Popup" | "Link" | "Widget" | "") {
                continue;
            }
            let rect = match d.get(b"Rect").ok().and_then(|o| o.as_array().ok()) {
                Some(r) if r.len() == 4 => {
                    let v: Vec<f64> = r
                        .iter()
                        .map(|x| x.as_float().map(|f| f as f64).unwrap_or(0.0))
                        .collect();
                    RectPt::new(v[0], v[1], v[2], v[3]).transform(&u2d)
                }
                _ => continue,
            };
            let irt = d.get(b"IRT").ok().map(|o| match o {
                lopdf::Object::Reference(r) => format!("{} {}", r.0, r.1),
                o => format!("{o:?}"),
            });
            let state = {
                let s = text(b"State");
                if s.is_empty() {
                    None
                } else {
                    Some(s)
                }
            };
            out.push(RawAnnot {
                page: p,
                subtype,
                rect,
                contents: text(b"Contents").replace('\r', "\n").trim().to_string(),
                subject: text(b"Subj"),
                author: text(b"T"),
                date: text(b"M"),
                nm: text(b"NM"),
                in_reply_to: irt,
                reply_type: {
                    let rt = name(b"RT");
                    if rt.is_empty() {
                        None
                    } else {
                        Some(rt)
                    }
                },
                state,
                object: id,
            });
        }
    }
    Ok(out)
}

fn is_own(author: &str, cfg: &Config) -> bool {
    cfg.comments
        .own_authors
        .iter()
        .any(|o| !o.is_empty() && author.to_lowercase().contains(&o.to_lowercase()))
}

fn is_info(a: &RawAnnot, cfg: &Config) -> bool {
    let s = a.subject.to_lowercase();
    a.subtype == "Stamp"
        || cfg
            .comments
            .info_subjects
            .iter()
            .any(|i| s.contains(&i.to_lowercase()))
}

/// Groepeert annotaties tot opmerkingen.
pub fn group(annots: &[RawAnnot], cfg: &Config) -> Vec<Comment> {
    let attach = cfg.comments.attach_mm * PT_PER_MM;
    let mut used = vec![false; annots.len()];
    let mut out: Vec<Comment> = Vec::new();
    // antwoorden (IRT zonder /RT /Group) en eigen auteurs niet als eigen opmerking. Een
    // groepslid (/RT /Group, bijv. Bluebeams wolk bij een tekstvak) hoort bij zijn doel.
    let is_group =
        |a: &RawAnnot| a.in_reply_to.is_some() && a.reply_type.as_deref() == Some("Group");
    let replies: Vec<bool> = annots
        .iter()
        .map(|a| a.in_reply_to.is_some() && !is_group(a))
        .collect();
    let obj_key = |a: &RawAnnot| format!("{} {}", a.object.0, a.object.1);
    let mut comment_of: Vec<Option<usize>> = vec![None; annots.len()];
    let mk = |a: &RawAnnot, kind: CommentKind, text: String| Comment {
        id: if a.nm.is_empty() {
            format!("p{}-{:.0}-{:.0}", a.page + 1, a.rect.x0, a.rect.y0)
        } else {
            a.nm.clone()
        },
        number: 0,
        kind,
        c_page: a.page,
        sheet: None,
        text,
        author: a.author.clone(),
        date: a.date.clone(),
        subject: a.subject.clone(),
        zone_c: a.rect,
        zone_a: None,
        zone_b: None,
        parts: vec![a.rect],
        own_marks: Vec::new(),
        proposal: Proposal::Unclear,
        reason: String::new(),
        diff_px: 0,
        text_changes: Vec::new(),
    };
    // 1. tekstdragers
    for (i, a) in annots.iter().enumerate() {
        if a.contents.is_empty() || is_own(&a.author, cfg) || replies[i] {
            continue;
        }
        used[i] = true;
        let t = a.contents.trim();
        let is_label = t.len() <= 4
            && t.chars().next().is_some_and(|c| c.is_ascii_digit())
            && t.chars().all(|c| c.is_ascii_alphanumeric());
        let kind = if is_info(a, cfg) {
            CommentKind::Info
        } else if is_label {
            CommentKind::Label
        } else {
            CommentKind::Annotation
        };
        comment_of[i] = Some(out.len());
        out.push(mk(a, kind, a.contents.clone()));
    }
    // 1b. groepsleden direct bij hun doel
    for (i, a) in annots.iter().enumerate() {
        if used[i] || is_own(&a.author, cfg) || !is_group(a) {
            continue;
        }
        let target = a.in_reply_to.as_deref();
        let ci = annots
            .iter()
            .enumerate()
            .find(|(_, t)| Some(obj_key(t).as_str()) == target && t.page == a.page)
            .and_then(|(j, _)| comment_of[j]);
        if let Some(ci) = ci {
            out[ci].parts.push(a.rect);
            used[i] = true;
        }
    }
    // 2. vormen zonder tekst: aanhechten aan de dichtstbijzijnde tekstdrager op dezelfde
    //    pagina. Herhalen tot er niets meer bijkomt, zodat een ketting (tekst - pijl -
    //    rechthoek) ook werkt als de rechthoek eerder in de lijst staat dan de pijl.
    let shape_idx: Vec<usize> = (0..annots.len())
        .filter(|&i| !used[i] && !is_own(&annots[i].author, cfg) && !replies[i])
        .collect();
    for &i in &shape_idx {
        let a = &annots[i];
        if is_info(a, cfg) {
            used[i] = true;
            let t = if a.subject.is_empty() {
                a.subtype.clone()
            } else {
                a.subject.clone()
            };
            out.push(mk(a, CommentKind::Info, t));
        }
    }
    loop {
        let mut changed = false;
        for &i in &shape_idx {
            if used[i] {
                continue;
            }
            let a = &annots[i];
            let best = out
                .iter_mut()
                .filter(|c| c.c_page == a.page && c.kind == CommentKind::Annotation)
                .map(|c| {
                    let d = c
                        .parts
                        .iter()
                        .map(|p| p.distance(&a.rect))
                        .fold(f64::MAX, f64::min);
                    (d, c)
                })
                .filter(|(d, _)| *d <= attach)
                .min_by(|x, y| x.0.total_cmp(&y.0));
            if let Some((_, c)) = best {
                c.parts.push(a.rect);
                used[i] = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    for &i in &shape_idx {
        if !used[i] {
            used[i] = true;
            let a = &annots[i];
            let t = format!(
                "({} zonder tekst)",
                if a.subject.is_empty() {
                    &a.subtype
                } else {
                    &a.subject
                }
            );
            out.push(mk(a, CommentKind::MarkOnly, t));
        }
    }
    // zone: de vormen (wolk/pijl/rechthoek) als die er zijn, anders het tekstvak
    for c in out.iter_mut() {
        let shapes = &c.parts[1..];
        c.zone_c = if shapes.is_empty() {
            c.parts[0]
        } else {
            shapes.iter().skip(1).fold(shapes[0], |acc, r| acc.union(r))
        };
    }
    // 3. eigen markeringen (bijv. afvinken) noteren bij de opmerking waar ze overheen liggen
    for a in annots.iter().filter(|a| is_own(&a.author, cfg)) {
        for c in out.iter_mut().filter(|c| c.c_page == a.page) {
            if c.parts.iter().any(|p| p.intersects(&a.rect)) {
                c.own_marks.push(format!(
                    "{} {} ({})",
                    a.author,
                    a.subject,
                    short_date(&a.date)
                ));
            }
        }
    }
    // reviewstatus van de aannemer zelf (Acrobat/Bluebeam /State in antwoorden)
    out.sort_by(|x, y| {
        x.c_page
            .cmp(&y.c_page)
            .then(x.zone_c.y0.total_cmp(&y.zone_c.y0))
            .then(x.zone_c.x0.total_cmp(&y.zone_c.x0))
    });
    for (n, c) in out.iter_mut().enumerate() {
        c.number = n + 1;
    }
    out
}

fn short_date(d: &str) -> String {
    // D:20260620180700+02'00' -> 20-06-2026
    let s: Vec<char> = d.trim_start_matches("D:").chars().collect();
    if s.len() >= 8 && s[..8].iter().all(|c| c.is_ascii_digit()) {
        let t = |a: usize, b: usize| s[a..b].iter().collect::<String>();
        format!("{}-{}-{}", t(6, 8), t(4, 6), t(0, 4))
    } else {
        d.to_string()
    }
}

/// Rode markeringen in de inhoud: rode inkt in C die in A (binnen 2 px) niet rood is.
fn content_markups(
    page_a: &pdfium_render::prelude::PdfPage,
    size_a: (f64, f64),
    page_c: &pdfium_render::prelude::PdfPage,
    c_to_a: &Affine,
    cfg: &Config,
) -> Result<Vec<RectPt>> {
    let dpi = 75.0;
    let s = dpi / 72.0;
    let (w, h) = (
        (size_a.0 * s).round() as usize,
        (size_a.1 * s).round() as usize,
    );
    let to_px = Affine::scale(s);
    let ra = pdf::render_rgba(page_a, &to_px, w, h, false)?;
    let rc = pdf::render_rgba(page_c, &c_to_a.then(&to_px), w, h, false)?;
    let (rmin, omax) = (cfg.comments.red_min, cfg.comments.red_max_other);
    // tint in plaats van harde kanaalgrenzen: een dunne rode lijn wordt door anti-aliasing
    // roze (255,150,150); dat is nog steeds rood zolang R duidelijk boven G en B ligt
    let margin = rmin.saturating_sub(omax).max(40);
    let red = |d: &[u8], i: usize| {
        let (r, g, b) = (d[i * 4], d[i * 4 + 1], d[i * 4 + 2]);
        r >= rmin && r.saturating_sub(g.max(b)) >= margin && (g as i16 - b as i16).abs() < 60
    };
    let mut mask = CoarseMask::new(w, h, 2);
    let mut cls = vec![0u8; w * h];
    let r = 2isize;
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if !red(&rc.data, i) {
                continue;
            }
            let mut in_a = false;
            'n: for dy in -r..=r {
                for dx in -r..=r {
                    let (nx, ny) = (x as isize + dx, y as isize + dy);
                    if nx >= 0
                        && ny >= 0
                        && (nx as usize) < w
                        && (ny as usize) < h
                        && red(&ra.data, ny as usize * w + nx as usize)
                    {
                        in_a = true;
                        break 'n;
                    }
                }
            }
            if !in_a {
                cls[i] = raster::ADDED;
            }
        }
    }
    tracing::debug!(
        red_c = (0..w * h).filter(|&i| red(&rc.data, i)).count(),
        red_new = cls.iter().filter(|&&c| c != 0).count(),
        "rode inhoud"
    );
    let dm = raster::DiffMap {
        w,
        h,
        class: cls,
        removed: 0,
        added: 0,
    };
    mask.accumulate(&dm, 0);
    let gap = (8.0 * PT_PER_MM * s / 2.0).ceil() as usize;
    Ok(raster::regions(&mask, 1, gap)
        .into_iter()
        .filter(|r| r.added >= 25)
        .map(|r| {
            let k = mask.k as f64 / s;
            RectPt::new(
                r.cx0 as f64 * k,
                r.cy0 as f64 * k,
                (r.cx1 + 1) as f64 * k,
                (r.cy1 + 1) as f64 * k,
            )
        })
        .collect())
}

/// Verschilpixels A->B in een zone van A (weergavepunten), op `dpi`.
fn zone_diff(
    page_a: &pdfium_render::prelude::PdfPage,
    page_b: &pdfium_render::prelude::PdfPage,
    b_to_a: &Affine,
    zone: &RectPt,
    dpi: f64,
    cfg: &Config,
) -> Result<u64> {
    let s = dpi / 72.0;
    let (w, h) = (
        (zone.width() * s).ceil().max(1.0) as usize,
        (zone.height() * s).ceil().max(1.0) as usize,
    );
    let m = cfg.compare.diff.radius_px + 1;
    let to_px = Affine::translate(-zone.x0, -zone.y0)
        .then(&Affine::scale(s))
        .then(&Affine::translate(m as f64, m as f64));
    let (pw, ph) = (w + 2 * m, h + 2 * m);
    let ga = pdf::render_gray(page_a, &to_px, pw, ph)?;
    let gb = pdf::render_gray(page_b, &b_to_a.then(&to_px), pw, ph)?;
    let d = raster::diff(&ga, &gb, &cfg.compare.diff);
    let mut n = 0u64;
    for y in m..m + h {
        n += d.class[y * pw + m..y * pw + m + w]
            .iter()
            .filter(|&&c| c != 0)
            .count() as u64;
    }
    Ok(n)
}

fn ensure(docs: &mut BTreeMap<PathBuf, Doc>, f: &Path) -> Result<()> {
    if !docs.contains_key(f) {
        docs.insert(f.to_path_buf(), Doc::open(f)?);
    }
    Ok(())
}

/// Volledige analyse van een opmerkingen-PDF tegen uitgifte A en B.
pub fn analyze(c_file: &Path, a: &Issue, b: &Issue, cfg: &Config) -> Result<CommentsResult> {
    let ident = Identifier::new(&cfg.identify)?;
    let norm = Normalizer::new(&cfg.normalize.stamp_patterns)?;
    let dc = Doc::open(c_file)?;
    let mut geoms = Vec::new();
    let mut sheet_of_page: Vec<Option<String>> = Vec::new();
    let stem = c_file
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    for p in 0..dc.page_count() {
        let page = dc.page(p)?;
        let g = pdf::page_geom(&page)?;
        let runs = pdf::text_runs(&page, &g).unwrap_or_default();
        sheet_of_page.push(ident.identify(&runs, g.display_size(), &stem).number);
        geoms.push(g);
    }
    let annots = read_annotations(c_file, &geoms)?;
    let mut comments = group(&annots, cfg);
    let mut warnings = Vec::new();
    let mut page_regs: BTreeMap<usize, (String, Registration)> = BTreeMap::new();

    // per opmerkingenpagina met een tekeningnummer: uitlijnen, rode inhoud zoeken
    let mut docs: BTreeMap<PathBuf, Doc> = BTreeMap::new();
    let mut b_regs: BTreeMap<String, Registration> = BTreeMap::new();
    for (p, sheet) in sheet_of_page.iter().enumerate() {
        let Some(nr) = sheet else {
            if annots.iter().any(|a| a.page == p) && p > 0 {
                warnings.push(format!(
                    "opmerkingenpagina {}: geen tekeningnummer herkend; opmerkingen daar zijn 'algemeen' en rode inhoud is niet gezocht",
                    p + 1
                ));
            }
            continue;
        };
        let Some(sa) = a.sheets.get(nr) else {
            warnings.push(format!(
                "opmerkingenpagina {} ({nr}): blad niet in uitgifte A",
                p + 1
            ));
            continue;
        };
        ensure(&mut docs, &sa.file)?;
        let da = &docs[&sa.file];
        let pa = da.page(sa.page)?;
        let pc = dc.page(p)?;
        let reg = register::register(&pa, &sa.geom, &pc, &geoms[p], &cfg.register)?;
        if !reg.ok {
            warnings.push(format!("opmerkingenpagina {} ({nr}): {}", p + 1, reg.note));
        }
        // rode inhoud
        if reg.ok {
            let marks = content_markups(&pa, sa.geom.display_size(), &pc, &reg.transform, cfg)?;
            if !marks.is_empty() {
                // tekst die in C staat en niet in A: bijschrift bij de markering
                let runs_a = pdf::text_runs(&pa, &sa.geom).unwrap_or_default();
                let runs_c = pdf::text_runs(&pc, &geoms[p]).unwrap_or_default();
                let tc = textdiff::diff_text(
                    &runs_a,
                    &runs_c,
                    &reg.transform,
                    &norm,
                    cfg.compare.text_pos_tol_mm,
                );
                let inv = reg.transform.inverse().unwrap_or(Affine::IDENTITY);
                for (k, m) in marks.iter().enumerate() {
                    let near = m.expand(15.0 * PT_PER_MM);
                    let txt: Vec<String> = tc
                        .iter()
                        .filter(|t| t.kind == TextChangeKind::Added && near.intersects(&t.bbox))
                        .filter_map(|t| t.new.clone())
                        .collect();
                    let zone_c = m.transform(&inv);
                    // al gedekt door een annotatie? dan niet dubbel
                    if comments
                        .iter()
                        .any(|c| c.c_page == p && c.parts.iter().any(|r| r.intersects(&zone_c)))
                    {
                        continue;
                    }
                    comments.push(Comment {
                        id: format!("rood-p{}-{}", p + 1, k + 1),
                        number: 0,
                        kind: CommentKind::ContentMarkup,
                        c_page: p,
                        sheet: None,
                        text: if txt.is_empty() {
                            "(rode markering zonder tekst)".into()
                        } else {
                            txt.join(" ")
                        },
                        author: String::new(),
                        date: String::new(),
                        subject: "rood in inhoud".into(),
                        zone_c,
                        zone_a: None,
                        zone_b: None,
                        parts: vec![zone_c],
                        own_marks: vec![],
                        proposal: Proposal::Unclear,
                        reason: String::new(),
                        diff_px: 0,
                        text_changes: vec![],
                    });
                }
            }
        }
        page_regs.insert(p, (nr.clone(), reg));
    }

    // per opmerking: plek op A en B, voorstel
    let margin = cfg.comments.zone_margin_mm * PT_PER_MM;
    let mut text_by_sheet: BTreeMap<String, Vec<crate::textdiff::TextChange>> = BTreeMap::new();
    for c in comments.iter_mut() {
        if c.kind == CommentKind::Info {
            c.proposal = Proposal::Info;
            c.reason = "akkoordvink of stempel, geen actie".into();
            if let Some((nr, _)) = page_regs.get(&c.c_page) {
                c.sheet = Some(nr.clone());
            }
            continue;
        }
        let Some((nr, regc)) = page_regs.get(&c.c_page) else {
            c.proposal = Proposal::Unclear;
            c.reason = "algemene opmerking (pagina zonder tekeningnummer)".into();
            continue;
        };
        c.sheet = Some(nr.clone());
        if !regc.ok {
            c.proposal = Proposal::Unclear;
            c.reason = format!("opmerkingenblad niet uit te lijnen op A: {}", regc.note);
            continue;
        }
        let za = c.zone_c.transform(&regc.transform);
        c.zone_a = Some(za);
        let sa: &SheetRef = &a.sheets[nr];
        let Some(sb) = b.sheets.get(nr) else {
            c.proposal = Proposal::Unclear;
            c.reason = "blad komt in B niet voor (vervallen of hernummerd)".into();
            continue;
        };
        ensure(&mut docs, &sa.file)?;
        ensure(&mut docs, &sb.file)?;
        let (da, db) = (&docs[&sa.file], &docs[&sb.file]);
        let (pa, pb) = (da.page(sa.page)?, db.page(sb.page)?);
        if !b_regs.contains_key(nr) {
            b_regs.insert(
                nr.clone(),
                register::register(&pa, &sa.geom, &pb, &sb.geom, &cfg.register)?,
            );
        }
        let regb = &b_regs[nr];
        if !regb.ok {
            c.proposal = Proposal::Unclear;
            c.reason = format!("B niet uit te lijnen op A: {}", regb.note);
            continue;
        }
        if let Some(inv) = regb.transform.inverse() {
            c.zone_b = Some(za.transform(&inv));
        }
        let z = za.expand(margin);
        c.diff_px = zone_diff(&pa, &pb, &regb.transform, &z, 150.0, cfg)?;
        if !text_by_sheet.contains_key(nr) {
            let ra = pdf::text_runs(&pa, &sa.geom).unwrap_or_default();
            let rb = pdf::text_runs(&pb, &sb.geom).unwrap_or_default();
            text_by_sheet.insert(
                nr.clone(),
                textdiff::diff_text(
                    &ra,
                    &rb,
                    &regb.transform,
                    &norm,
                    cfg.compare.text_pos_tol_mm,
                ),
            );
        }
        c.text_changes = text_by_sheet[nr]
            .iter()
            .filter(|t| t.kind != TextChangeKind::Stamp && t.bbox.intersects(&z))
            .map(|t| match (&t.old, &t.new) {
                (Some(o), Some(n)) if o != n => format!("{o} -> {n}"),
                (Some(o), None) => format!("weg: {o}"),
                (None, Some(n)) => format!("nieuw: {n}"),
                (Some(o), Some(_)) => format!("verplaatst: {o}"),
                _ => String::new(),
            })
            .collect();
        let zone_px = (z.area() * (150.0f64 / 72.0).powi(2)).max(1.0);
        let (done_px, unclear_px) = (cfg.comments.done_min_px, cfg.comments.unclear_min_px);
        // verplaatste tekst is geen bewijs van verwerking; alleen gewijzigd/nieuw/weg telt
        let content_text = text_by_sheet[nr]
            .iter()
            .filter(|t| {
                matches!(
                    t.kind,
                    TextChangeKind::Changed | TextChangeKind::Added | TextChangeKind::Removed
                ) && t.bbox.intersects(&z)
            })
            .count();
        (c.proposal, c.reason) = if c.diff_px >= done_px || content_text > 0 {
            (
                Proposal::LikelyDone,
                format!(
                    "verschil A->B in de zone: {} px ({:.2} %){}",
                    c.diff_px,
                    100.0 * c.diff_px as f64 / zone_px,
                    if content_text == 0 {
                        String::new()
                    } else {
                        format!(", {content_text} tekstwijziging(en)")
                    }
                ),
            )
        } else if c.diff_px >= unclear_px {
            (
                Proposal::Unclear,
                format!("klein verschil A->B in de zone ({} px)", c.diff_px),
            )
        } else {
            (Proposal::NotDone, "geen verschil A->B in de zone".into())
        };
        if c.kind == CommentKind::Label && c.proposal == Proposal::NotDone {
            c.proposal = Proposal::Unclear;
            c.reason = "verwijzingsnummer (zie lijst elders); geen verschil op deze plek".into();
        }
    }
    comments.sort_by(|x, y| {
        x.c_page
            .cmp(&y.c_page)
            .then(x.zone_c.y0.total_cmp(&y.zone_c.y0))
            .then(x.zone_c.x0.total_cmp(&y.zone_c.x0))
    });
    for (n, c) in comments.iter_mut().enumerate() {
        c.number = n + 1;
    }
    Ok(CommentsResult {
        file: c_file.to_path_buf(),
        comments,
        page_regs,
        warnings,
    })
}

// ---------------------------------------------------------------------------
// Status van de gebruiker, naast de PDF's opgeslagen

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Status {
    #[default]
    Open,
    Done,
    NotDone,
    NotApplicable,
    Question,
}

impl Status {
    pub const ALL: [Status; 5] = [
        Status::Open,
        Status::Done,
        Status::NotDone,
        Status::NotApplicable,
        Status::Question,
    ];
    pub fn label(&self) -> &'static str {
        match self {
            Status::Open => "",
            Status::Done => "verwerkt",
            Status::NotDone => "niet verwerkt",
            Status::NotApplicable => "n.v.t.",
            Status::Question => "vraag",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReviewEntry {
    pub status: Status,
    pub note: String,
    pub updated: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReviewState {
    pub version: u32,
    pub a: PathBuf,
    pub b: PathBuf,
    pub comments_pdf: PathBuf,
    pub entries: BTreeMap<String, ReviewEntry>,
}

impl ReviewState {
    /// Standaardplek: naast de opmerkingen-PDF, `<naam>.sheetdiff.json`.
    pub fn default_path(c_file: &Path) -> PathBuf {
        let stem = c_file.file_stem().unwrap_or_default().to_string_lossy();
        c_file.with_file_name(format!("{stem}.sheetdiff.json"))
    }
    pub fn load_or_new(path: &Path, a: &Path, b: &Path, c: &Path) -> Result<Self> {
        Ok(Self::load_checked(path, a, b, c)?.0)
    }

    /// Leest het statusbestand. Hoort het bij een andere A/B dan nu, dan volgt een
    /// waarschuwing; de statussen blijven staan maar horen gecontroleerd te worden.
    pub fn load_checked(
        path: &Path,
        a: &Path,
        b: &Path,
        c: &Path,
    ) -> Result<(Self, Option<String>)> {
        if path.exists() {
            let s = std::fs::read_to_string(path)?;
            let st: ReviewState = serde_json::from_str(&s)
                .with_context(|| format!("statusbestand onleesbaar: {}", path.display()))?;
            let warn = if st.a != a || st.b != b {
                Some(format!(
                    "statusbestand {} hoort bij A={} / B={}; nu A={} / B={}. Statussen overgenomen, controleer ze.",
                    path.display(),
                    st.a.display(),
                    st.b.display(),
                    a.display(),
                    b.display()
                ))
            } else {
                None
            };
            return Ok((st, warn));
        }
        Ok((
            ReviewState {
                version: 1,
                a: a.into(),
                b: b.into(),
                comments_pdf: c.into(),
                entries: BTreeMap::new(),
            },
            None,
        ))
    }
    /// Schrijft atomair (tijdelijk bestand + hernoemen), zodat een crash de status niet sloopt.
    pub fn save(&self, path: &Path) -> Result<()> {
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
    pub fn entry(&self, id: &str) -> ReviewEntry {
        self.entries.get(id).cloned().unwrap_or_default()
    }
}

pub fn mm(v: f64) -> f64 {
    v * MM_PER_PT
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ann(
        page: usize,
        sub: &str,
        r: (f64, f64, f64, f64),
        text: &str,
        subj: &str,
        author: &str,
    ) -> RawAnnot {
        RawAnnot {
            page,
            subtype: sub.into(),
            rect: RectPt::new(r.0, r.1, r.2, r.3),
            contents: text.into(),
            subject: subj.into(),
            author: author.into(),
            date: "D:20260620180700+02'00'".into(),
            nm: String::new(),
            in_reply_to: None,
            reply_type: None,
            state: None,
            object: (0, 0),
        }
    }

    #[test]
    fn cloud_attaches_to_text_and_own_ink_is_noted() {
        let cfg = Config::default();
        let a = vec![
            ann(
                2,
                "Polygon",
                (695.0, 1502.0, 887.0, 1656.0),
                "",
                "Aannemer",
                "A. Aannemer",
            ),
            ann(
                2,
                "FreeText",
                (708.0, 1641.0, 877.0, 1735.0),
                "Groene arcering ontbreekt",
                "Cloud+",
                "A. Aannemer",
            ),
            ann(
                2,
                "Ink",
                (677.0, 1343.0, 939.0, 1759.0),
                "",
                "Pencil",
                "Jochem",
            ),
            ann(
                7,
                "PolyLine",
                (1032.0, 2348.0, 1081.0, 2387.0),
                "",
                "check akkoord",
                "A. Aannemer",
            ),
            ann(
                9,
                "Square",
                (100.0, 100.0, 200.0, 200.0),
                "",
                "Rechthoek",
                "A. Aannemer",
            ),
        ];
        let c = group(&a, &cfg);
        assert_eq!(c.len(), 3);
        let g = c.iter().find(|c| c.text.contains("Groene")).unwrap();
        assert_eq!(g.parts.len(), 2);
        // zone = de wolk, niet het tekstvak
        assert_eq!(g.zone_c, RectPt::new(695.0, 1502.0, 887.0, 1656.0));
        assert_eq!(g.own_marks.len(), 1);
        assert!(c.iter().any(|c| c.kind == CommentKind::Info));
        assert!(c.iter().any(|c| c.kind == CommentKind::MarkOnly));
    }

    #[test]
    fn short_date_format() {
        assert_eq!(short_date("D:20260620180700+02'00'"), "20-06-2026");
        assert_eq!(short_date("2026-06-20"), "2026-06-20");
        assert_eq!(short_date("D:éééé12345"), "D:éééé12345");
    }
}
