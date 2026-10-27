//! Uitgifte inlezen: map met PDF's of een losse PDF -> bladen op tekeningnummer.
//! Een blad dat in meerdere boekjes zit, wordt een keer vergeleken; de boekjes worden onthouden.

use crate::config::Config;
use crate::geom::PageGeom;
use crate::identify::{IdMethod, Identifier};
use crate::pdf::{self, Doc};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SheetRef {
    /// Sleutel: tekeningnummer, of "nummer [titel]" bij een dubbel nummer met andere titel,
    /// of "?bestand#pagina" als er geen nummer is.
    pub key: String,
    pub number: Option<String>,
    pub title: String,
    pub file: PathBuf,
    pub page: usize,
    pub geom: PageGeom,
    pub method: IdMethod,
    pub text_runs: usize,
    /// Boekjes (bestandsnamen zonder extensie) waarin dit blad voorkomt.
    pub booklets: Vec<String>,
    /// Inhoudshandtekening (genormaliseerde tekst zonder printstempels + formaat), om twee
    /// pagina's met hetzelfde nummer maar andere inhoud uit elkaar te houden.
    #[serde(default)]
    pub signature: u64,
}

/// Zelfde vectorinhoud (tolerant, 0,15 mm) op twee pagina's van gelijk formaat. Alleen
/// gebruikt bij twee pagina's met hetzelfde nummer en dezelfde tekst. Bij twijfel (fout,
/// ander formaat) "niet gelijk", zodat beide versies zichtbaar blijven.
pub fn same_vectors(f1: &Path, p1: usize, f2: &Path, p2: usize) -> bool {
    let run = || -> Result<bool> {
        let (d1, d2) = (Doc::open(f1)?, Doc::open(f2)?);
        let (a, b) = (d1.page(p1)?, d2.page(p2)?);
        let (ga, gb) = (pdf::page_geom(&a)?, pdf::page_geom(&b)?);
        let (sa, sb) = (ga.display_size(), gb.display_size());
        if (sa.0 - sb.0).abs() > 0.5 || (sa.1 - sb.1).abs() > 0.5 {
            return Ok(false);
        }
        let (va, _) = crate::vector::extract(&a, &ga)?;
        let (vb, _) = crate::vector::extract(&b, &gb)?;
        let ch = crate::vector::compare(
            &va,
            &vb,
            &crate::geom::Affine::IDENTITY,
            &Default::default(),
        );
        Ok(ch.is_empty())
    };
    run().unwrap_or(false)
}

/// Handtekening van een pagina: gesorteerde tekstsleutels (stempels geneutraliseerd) plus
/// paginaformaat. Gelijk = zelfde inhoud voor zover de tekstlaag het ziet.
pub fn signature(
    runs: &[crate::pdf::TextRun],
    size: (f64, f64),
    norm: &crate::textdiff::Normalizer,
) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut keys: Vec<String> = runs.iter().map(|r| norm.key(&r.text)).collect();
    keys.sort();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    keys.hash(&mut h);
    (
        (size.0 * 10.0).round() as i64,
        (size.1 * 10.0).round() as i64,
    )
        .hash(&mut h);
    h.finish()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    pub root: PathBuf,
    pub sheets: BTreeMap<String, SheetRef>,
    /// Boekje -> sleutels in volgorde.
    pub booklets: BTreeMap<String, Vec<String>>,
    /// Pagina's die als tekeninglijst zijn herkend (bestand, pagina).
    pub list_pages: Vec<(PathBuf, usize)>,
    pub warnings: Vec<String>,
}

/// Alle PDF's in een map (niet recursief) of het bestand zelf, gesorteerd.
pub fn pdf_files(root: &Path) -> Result<Vec<PathBuf>> {
    if root.is_file() {
        return Ok(vec![root.to_path_buf()]);
    }
    let mut v: Vec<PathBuf> = std::fs::read_dir(root)
        .with_context(|| format!("map niet te lezen: {}", root.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")))
        .collect();
    v.sort();
    Ok(v)
}

pub fn scan_issue(root: &Path, cfg: &Config) -> Result<Issue> {
    let ident = Identifier::new(&cfg.identify)?;
    let norm = crate::textdiff::Normalizer::new(&cfg.normalize.stamp_patterns)?;
    let files = pdf_files(root)?;
    if files.is_empty() {
        anyhow::bail!("geen PDF's gevonden in {}", root.display());
    }
    let mut issue = Issue {
        root: root.to_path_buf(),
        sheets: BTreeMap::new(),
        booklets: BTreeMap::new(),
        list_pages: Vec::new(),
        warnings: Vec::new(),
    };
    for f in files {
        let doc = Doc::open(&f)?;
        let stem = f
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        for p in 0..doc.page_count() {
            let page = doc.page(p)?;
            let geom = pdf::page_geom(&page)?;
            let runs = match pdf::text_runs(&page, &geom) {
                Ok(r) => r,
                Err(e) => {
                    issue.warnings.push(format!(
                        "{} p.{}: tekstlaag onleesbaar ({e:#})",
                        stem,
                        p + 1
                    ));
                    Vec::new()
                }
            };
            let sig = signature(&runs, geom.display_size(), &norm);
            let mtime = std::fs::metadata(&f).and_then(|m| m.modified()).ok();
            let id = ident.identify(&runs, geom.display_size(), &stem);
            if id.looks_like_list && id.number.is_none() {
                issue.list_pages.push((f.clone(), p));
                continue;
            }
            if runs.is_empty() {
                issue.warnings.push(format!(
                    "{} p.{}: geen tekstlaag - alleen pixelvergelijking mogelijk",
                    stem,
                    p + 1
                ));
            }
            let mut key = id
                .number
                .clone()
                .unwrap_or_else(|| format!("?{}#{}", stem, p + 1));
            if let Some(existing) = issue.sheets.get(&key).cloned() {
                if existing.title != id.title && !id.title.is_empty() && !existing.title.is_empty()
                {
                    issue.warnings.push(format!(
                        "{key}: nummer dubbel gebruikt ('{}' en '{}'); koppeling op nummer + titel",
                        existing.title, id.title
                    ));
                    key = format!("{key} [{}]", id.title);
                } else if existing.signature != sig
                    || (existing.file != f && !same_vectors(&existing.file, existing.page, &f, p))
                {
                    // zelfde nummer, andere inhoud: het nieuwste bestand is het blad, de andere
                    // versie blijft apart staan (wordt als eigen blad gemeld, nooit stil weg)
                    let old_mtime = std::fs::metadata(&existing.file)
                        .and_then(|m| m.modified())
                        .ok();
                    let newer = matches!((mtime, old_mtime), (Some(n), Some(o)) if n > o);
                    let estem = existing
                        .file
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    issue.warnings.push(format!(
                        "{key}: twee versies met ANDERE inhoud ({} p.{} en {} p.{}); vergeleken wordt de nieuwste ({}), de andere staat apart",
                        estem, existing.page + 1, stem, p + 1,
                        if newer { &stem } else { &estem }
                    ));
                    if newer {
                        let mut old = issue.sheets.remove(&key).expect("bestaat");
                        let alt = format!("{key} (versie {estem})");
                        old.key = alt.clone();
                        issue.sheets.insert(alt, old);
                    } else {
                        key = format!("{key} (versie {stem})");
                    }
                }
            }
            issue
                .booklets
                .entry(stem.clone())
                .or_default()
                .push(key.clone());
            match issue.sheets.get_mut(&key) {
                Some(s) => {
                    if !s.booklets.contains(&stem) {
                        s.booklets.push(stem.clone());
                    }
                }
                None => {
                    issue.sheets.insert(
                        key.clone(),
                        SheetRef {
                            key: key.clone(),
                            number: id.number.clone(),
                            title: id.title.clone(),
                            file: f.clone(),
                            page: p,
                            geom,
                            method: id.method.clone(),
                            text_runs: runs.len(),
                            booklets: vec![stem.clone()],
                            signature: sig,
                        },
                    );
                }
            }
        }
    }
    Ok(issue)
}
