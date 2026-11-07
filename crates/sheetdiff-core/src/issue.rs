//! Uitgifte inlezen: map met PDF's of een losse PDF -> bladen op tekeningnummer.
//! Een blad dat in meerdere boekjes zit, wordt een keer vergeleken; de boekjes worden onthouden.

use crate::compare::Runner;
use crate::config::Config;
use crate::geom::{Affine, PageGeom};
use crate::identify::{IdMethod, Identifier, Identity};
use crate::pairing::{InkMap, Tokenizer};
use crate::pdf::{self, Doc};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

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
    /// Woorden van het blad (hashes, gesorteerd), voor koppelen op inhoud.
    #[serde(skip)]
    pub tokens: Arc<Vec<u64>>,
    /// Inktkaart; wordt pas gemaakt als het blad op inhoud gekoppeld moet worden.
    #[serde(skip)]
    pub ink: Option<Arc<InkMap>>,
    /// Handmatige uitlijning van het paar; staat op het blad van A. Gaat mee naar de werkers.
    #[serde(default)]
    pub align: Option<crate::register::ManualAlign>,
    /// Aantal pagina's van het bestand; 0 = onbekend. Alleen voor de weergavenaam.
    #[serde(default)]
    pub file_pages: usize,
}

/// Leesbare vorm van een sleutel. Een blad zonder nummer heeft de interne sleutel
/// `?bestand#pagina`; dat wordt `bestand`, met ` p.n` erachter als het bestand meer pagina's
/// heeft of het aantal onbekend is (`pages` 0). Andere sleutels blijven zoals ze zijn.
pub fn display_key(key: &str, pages: usize) -> String {
    let Some((stem, tail)) = key.strip_prefix('?').and_then(|k| k.rsplit_once('#')) else {
        return key.to_string();
    };
    let digits = tail.chars().take_while(char::is_ascii_digit).count();
    let (page, rest) = tail.split_at(digits);
    if page.is_empty() {
        return key.trim_start_matches('?').to_string();
    }
    if pages == 1 {
        format!("{stem}{rest}")
    } else {
        format!("{stem} p.{page}{rest}")
    }
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
        let ch = crate::vector::compare(&va, &vb, &Affine::IDENTITY, &Default::default());
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
    let mut keys: Vec<String> = runs.iter().map(|r| norm.key(&r.text)).collect();
    keys.sort();
    // FNV-1a: de handtekening staat in het keuzebestand en moet dus een nieuwe build overleven;
    // DefaultHasher belooft dat niet
    let mut h = 0xcbf29ce484222325u64;
    let mut eat = |bytes: &[u8]| {
        for b in bytes {
            h = (h ^ *b as u64).wrapping_mul(0x100000001b3);
        }
    };
    for k in &keys {
        eat(k.as_bytes());
        eat(&[0xff]);
    }
    eat(&((size.0 * 10.0).round() as i64).to_le_bytes());
    eat(&((size.1 * 10.0).round() as i64).to_le_bytes());
    h
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
    /// Bladen die niet op sleutel maar op een andere manier aan B gekoppeld zijn; alleen
    /// gevuld op uitgifte A.
    #[serde(default)]
    pub links: Vec<SheetLink>,
    /// Toelichting per los blad: waarom het niet op inhoud gekoppeld is.
    #[serde(default)]
    pub notes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinkMethod {
    /// Op inhoud, score boven `pair.doubt_score`.
    Content,
    /// Op inhoud, score tussen `pair.min_score` en `pair.doubt_score`.
    Doubt,
    /// Pagina i tegen pagina i.
    Order,
    /// Door de gebruiker aangewezen.
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SheetLink {
    /// Gezamenlijke sleutel na het koppelen.
    pub key: String,
    /// Oorspronkelijke sleutels.
    pub key_a: String,
    pub key_b: String,
    pub method: LinkMethod,
    /// Inhoudsscore 0..1, als die bepaald is.
    #[serde(default)]
    pub score: Option<f64>,
    /// Op paginavolgorde gekoppeld terwijl de inhoud onder `pair.min_score` ligt.
    #[serde(default)]
    pub low: bool,
}

impl SheetLink {
    /// Label voor rapport en UI.
    pub fn label(&self) -> String {
        let pct = (self.score.unwrap_or(0.0) * 100.0).round();
        match self.method {
            LinkMethod::Content => format!("op inhoud ({pct:.0} %)"),
            LinkMethod::Doubt => format!("twijfel ({pct:.0} %)"),
            LinkMethod::Order => "op paginavolgorde".into(),
            LinkMethod::Manual => "handmatig".into(),
        }
    }

    /// Melding voor rapport en UI, als dit paar aandacht vraagt.
    pub fn note(&self) -> Option<String> {
        let names = format!(
            "A '{}' met B '{}'",
            display_key(&self.key_a, 0),
            display_key(&self.key_b, 0)
        );
        match self.method {
            LinkMethod::Content => None,
            LinkMethod::Order if !self.low => None,
            LinkMethod::Order => Some(format!(
                "op paginavolgorde: {names} lijken niet op elkaar (inhoud {:.0} %); waarschijnlijk is het boekje verschoven",
                self.score.unwrap_or(0.0) * 100.0
            )),
            _ => Some(format!(
                "{}: {names}; controleer of dit hetzelfde blad is",
                self.label()
            )),
        }
    }
}

pub(crate) fn rekey(issue: &mut Issue, old: &str, new: &str) {
    if old == new {
        return;
    }
    if let Some(mut s) = issue.sheets.remove(old) {
        s.key = new.to_string();
        issue.sheets.insert(new.to_string(), s);
    }
    for k in issue.booklets.values_mut().flatten() {
        if k == old {
            *k = new.to_string();
        }
    }
}

/// Koppelt blad `key_a` van A aan blad `key_b` van B door beide dezelfde sleutel te geven; zo
/// werken vergelijken, opmerkingen en export daarna als bij een gewone koppeling. De sleutel
/// wordt die van B, tenzij alleen A een nummer heeft. Geeft de gezamenlijke sleutel terug.
pub fn link_sheets(
    a: &mut Issue,
    b: &mut Issue,
    key_a: &str,
    key_b: &str,
    method: LinkMethod,
) -> Result<String> {
    let (Some(sa), Some(sb)) = (a.sheets.get(key_a), b.sheets.get(key_b)) else {
        anyhow::bail!("koppelen: blad '{key_a}' of '{key_b}' bestaat niet");
    };
    if b.sheets.contains_key(key_a) || a.sheets.contains_key(key_b) {
        anyhow::bail!("koppelen: '{key_a}' of '{key_b}' is al gekoppeld");
    }
    let key = if sa.number.is_some() && sb.number.is_none() {
        key_a
    } else {
        key_b
    }
    .to_string();
    rekey(a, key_a, &key);
    rekey(b, key_b, &key);
    a.links.push(SheetLink {
        key: key.clone(),
        key_a: key_a.to_string(),
        key_b: key_b.to_string(),
        method,
        score: None,
        low: false,
    });
    Ok(key)
}

/// Maakt een paar weer los: elk blad krijgt zijn oorspronkelijke sleutel terug. Waren die gelijk,
/// dan krijgen ze een achtervoegsel, anders vallen ze meteen weer samen. Geeft de sleutels van
/// A en B terug.
pub fn unlink_sheets(a: &mut Issue, b: &mut Issue, key: &str) -> Result<(String, String)> {
    if !a.sheets.contains_key(key) || !b.sheets.contains_key(key) {
        anyhow::bail!("ontkoppelen: '{key}' is geen paar");
    }
    let link = a.links.iter().position(|l| l.key == key);
    let (mut ka, mut kb) = match link.map(|i| a.links.remove(i)) {
        Some(l) => (l.key_a, l.key_b),
        None => (key.to_string(), key.to_string()),
    };
    if ka == kb {
        ka = format!("{ka} (A)");
        kb = format!("{kb} (B)");
    }
    // de oorspronkelijke sleutel kan intussen van een ander blad zijn
    let taken = |k: &str| k != key && (a.sheets.contains_key(k) || b.sheets.contains_key(k));
    while taken(&ka) {
        ka.push_str(" (A)");
    }
    while taken(&kb) {
        kb.push_str(" (B)");
    }
    rekey(a, key, &ka);
    rekey(b, key, &kb);
    Ok((ka, kb))
}

/// 'bestand' of 'map', voor meldingen over een pad dat er niet (meer) is: dan valt alleen aan de
/// extensie te zien wat het was.
pub fn path_word(p: &Path) -> &'static str {
    if p.is_dir() {
        "map"
    } else if p.is_file() || p.extension().is_some() {
        "bestand"
    } else {
        "map"
    }
}

/// Alle PDF's in een map (niet recursief) of het bestand zelf, gesorteerd.
pub fn pdf_files(root: &Path) -> Result<Vec<PathBuf>> {
    if root.is_file() {
        return Ok(vec![root.to_path_buf()]);
    }
    if !root.exists() {
        anyhow::bail!("{} niet gevonden: {}", path_word(root), root.display());
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
    scan(root, cfg, false, &Runner::InProcess)
}

/// Als [`scan_issue`], verdeeld over werkerprocessen.
pub fn scan_issue_with(root: &Path, cfg: &Config, runner: &Runner) -> Result<Issue> {
    scan(root, cfg, false, runner)
}

/// Elke pagina is een eigen blad, in volgorde: sleutel `0001 <nummer of bestand p.n>`. Geen
/// samenvoegen van dubbele bladen en geen overslaan van tekeninglijsten, zodat pagina i echt
/// pagina i blijft. Voor koppelen op paginavolgorde.
pub fn scan_pages(root: &Path, cfg: &Config) -> Result<Issue> {
    scan(root, cfg, true, &Runner::InProcess)
}

/// Als [`scan_pages`], verdeeld over werkerprocessen.
pub fn scan_pages_with(root: &Path, cfg: &Config, runner: &Runner) -> Result<Issue> {
    scan(root, cfg, true, runner)
}

/// Wat de scan van een pagina weet. Los van het samenvoegen tot bladen, zodat werkers dit kunnen
/// doen en de volgorde van het samenvoegen toch vast blijft.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageScan {
    pub page: usize,
    pub geom: PageGeom,
    pub text_runs: usize,
    pub signature: u64,
    pub id: Identity,
    pub tokens: Vec<u64>,
    /// Tekstlaag onleesbaar.
    pub warning: Option<String>,
}

fn stem_of(f: &Path) -> String {
    f.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Leest de pagina's `pages` van een PDF.
pub fn scan_file_pages(
    f: &Path,
    pages: std::ops::Range<usize>,
    cfg: &Config,
) -> Result<Vec<PageScan>> {
    let ident = Identifier::new(&cfg.identify)?;
    let norm = crate::textdiff::Normalizer::new(&cfg.normalize.stamp_patterns)?;
    let tokenizer = Tokenizer::new(cfg)?;
    let doc = Doc::open(f)?;
    let stem = stem_of(f);
    let mut out = Vec::with_capacity(pages.len());
    for p in pages {
        let page = doc.page(p)?;
        let geom = pdf::page_geom(&page)?;
        let (runs, warning) = match pdf::text_runs(&page, &geom) {
            Ok(r) => (r, None),
            Err(e) => (
                Vec::new(),
                Some(format!(
                    "{} p.{}: tekstlaag onleesbaar ({e:#})",
                    stem,
                    p + 1
                )),
            ),
        };
        out.push(PageScan {
            page: p,
            geom,
            text_runs: runs.len(),
            signature: signature(&runs, geom.display_size(), &norm),
            id: ident.identify(&runs, geom.display_size(), &stem),
            tokens: tokenizer.tokens(&runs),
            warning,
        });
    }
    Ok(out)
}

/// Sleutel van een pagina op nummer, voor het samenvoegen.
fn base_key(id: &Identity, stem: &str, page: usize) -> String {
    match (&id.number, &id.variant) {
        (Some(n), Some(v)) => format!("{n} [{v}]"),
        (Some(n), None) => n.clone(),
        _ => format!("?{}#{}", stem, page + 1),
    }
}

type VectorPair = (PathBuf, usize, PathBuf, usize);

/// Paren die het samenvoegen straks vraagt: dezelfde sleutel en tekst, ander bestand. Die kosten
/// elk ca. 0,6 s en zijn vooraf bekend, dus kunnen ze parallel. Een paar dat hier toch niet tussen
/// zit, rekent het samenvoegen zelf uit.
fn vector_candidates(pages: &[(PathBuf, Vec<PageScan>)]) -> Vec<VectorPair> {
    let mut seen: HashMap<String, Vec<(&PathBuf, &PageScan)>> = HashMap::new();
    let mut out = BTreeSet::new();
    for (f, list) in pages {
        let stem = stem_of(f);
        for ps in list {
            if ps.id.looks_like_list && ps.id.number.is_none() {
                continue;
            }
            let group = seen.entry(base_key(&ps.id, &stem, ps.page)).or_default();
            for (ef, e) in group.iter() {
                if e.signature == ps.signature && *ef != f {
                    out.insert(((*ef).clone(), e.page, f.clone(), ps.page));
                }
            }
            group.push((f, ps));
        }
    }
    out.into_iter().collect()
}

fn scan(root: &Path, cfg: &Config, by_page: bool, runner: &Runner) -> Result<Issue> {
    let mut page_no = 0usize;
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
        links: Vec::new(),
        notes: BTreeMap::new(),
    };
    let pages = match runner {
        Runner::InProcess => None,
        Runner::Processes { exe, .. } => Some(crate::parallel::scan_files(&files, cfg, exe)?),
    };
    let pages = match pages {
        Some(p) => p,
        None => {
            let mut v = Vec::with_capacity(files.len());
            for f in &files {
                let n = Doc::open(f)?.page_count();
                v.push((f.clone(), scan_file_pages(f, 0..n, cfg)?));
            }
            v
        }
    };
    let same: HashMap<VectorPair, bool> = match runner {
        Runner::Processes { exe, .. } if !by_page => {
            crate::parallel::same_vectors_all(vector_candidates(&pages), cfg, exe)?
        }
        _ => HashMap::new(),
    };
    let same_as = |f1: &Path, p1: usize, f2: &Path, p2: usize| {
        let k = (f1.to_path_buf(), p1, f2.to_path_buf(), p2);
        same.get(&k).copied().unwrap_or_else(|| {
            tracing::info!(
                "dubbel blad buiten de voorselectie: {} p.{}",
                f2.display(),
                p2 + 1
            );
            same_vectors(f1, p1, f2, p2)
        })
    };
    for (f, list) in pages {
        let stem = stem_of(&f);
        let file_pages = list.len();
        let mtime = std::fs::metadata(&f).and_then(|m| m.modified()).ok();
        for ps in list {
            let PageScan {
                page: p,
                geom,
                text_runs,
                signature: sig,
                id,
                tokens,
                warning,
            } = ps;
            issue.warnings.extend(warning);
            let tokens = Arc::new(tokens);
            page_no += 1;
            if by_page {
                let name = id
                    .number
                    .clone()
                    .unwrap_or_else(|| format!("{stem} p.{}", p + 1));
                let key = format!("{page_no:04} {name}");
                issue
                    .booklets
                    .entry(stem.clone())
                    .or_default()
                    .push(key.clone());
                issue.sheets.insert(
                    key.clone(),
                    SheetRef {
                        key,
                        number: id.number.clone(),
                        title: id.title.clone(),
                        file: f.clone(),
                        page: p,
                        geom,
                        method: id.method.clone(),
                        text_runs,
                        booklets: vec![stem.clone()],
                        signature: sig,
                        tokens,
                        ink: None,
                        align: None,
                        file_pages,
                    },
                );
                continue;
            }
            if id.looks_like_list && id.number.is_none() {
                issue.list_pages.push((f.clone(), p));
                continue;
            }
            if text_runs == 0 {
                issue.warnings.push(format!(
                    "{} p.{}: geen tekstlaag - alleen pixelvergelijking mogelijk",
                    stem,
                    p + 1
                ));
            }
            let mut key = base_key(&id, &stem, p);
            if let Some(existing) = issue.sheets.get(&key).cloned() {
                if existing.title != id.title && !id.title.is_empty() && !existing.title.is_empty()
                {
                    issue.warnings.push(format!(
                        "{key}: nummer dubbel gebruikt ('{}' en '{}'); koppeling op nummer + titel",
                        existing.title, id.title
                    ));
                    key = format!("{key} [{}]", id.title);
                } else if existing.signature != sig
                    || (existing.file != f && !same_as(&existing.file, existing.page, &f, p))
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
                            text_runs,
                            booklets: vec![stem.clone()],
                            signature: sig,
                            tokens,
                            ink: None,
                            align: None,
                            file_pages,
                        },
                    );
                }
            }
        }
    }
    Ok(issue)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_path_is_named_for_what_it_was() {
        let e = pdf_files(Path::new("Z:/weg/Z_01_07_D5.pdf")).unwrap_err();
        assert_eq!(
            e.to_string(),
            "bestand niet gevonden: Z:/weg/Z_01_07_D5.pdf"
        );
        let e = pdf_files(Path::new("Z:/weg/boekjes")).unwrap_err();
        assert_eq!(e.to_string(), "map niet gevonden: Z:/weg/boekjes");
    }

    #[test]
    fn internal_key_does_not_reach_the_display() {
        let k = "?1234-OVK_101 plattegrond_wijzA#1";
        assert_eq!(display_key(k, 1), "1234-OVK_101 plattegrond_wijzA");
        assert_eq!(display_key(k, 3), "1234-OVK_101 plattegrond_wijzA p.1");
        assert_eq!(display_key(k, 0), "1234-OVK_101 plattegrond_wijzA p.1");
        assert_eq!(display_key("?a#b#12 (A)", 2), "a#b p.12 (A)");
        assert_eq!(display_key("?a#2 (versie b)", 1), "a (versie b)");
        // een echt nummer blijft staan, ook met variant of versie
        assert_eq!(display_key("OVK_101", 1), "OVK_101");
        assert_eq!(
            display_key("Z.09.09_A1_09 [OptieA]", 1),
            "Z.09.09_A1_09 [OptieA]"
        );
    }

    fn sheet(key: &str, number: Option<&str>) -> SheetRef {
        SheetRef {
            key: key.into(),
            number: number.map(Into::into),
            title: String::new(),
            file: PathBuf::from("x.pdf"),
            page: 0,
            geom: PageGeom {
                crop: [0.0, 0.0, 100.0, 100.0],
                rotate: crate::geom::Rot90::R0,
            },
            method: IdMethod::None,
            text_runs: 0,
            booklets: vec!["boekje".into()],
            signature: 0,
            tokens: Default::default(),
            ink: None,
            align: None,
            file_pages: 1,
        }
    }

    fn issue(sheets: &[(&str, Option<&str>)]) -> Issue {
        Issue {
            root: PathBuf::from("x"),
            sheets: sheets
                .iter()
                .map(|(k, n)| (k.to_string(), sheet(k, *n)))
                .collect(),
            booklets: [(
                "boekje".to_string(),
                sheets.iter().map(|(k, _)| k.to_string()).collect(),
            )]
            .into(),
            list_pages: vec![],
            warnings: vec![],
            links: vec![],
            notes: Default::default(),
        }
    }

    fn page(nr: &str, page: usize, signature: u64) -> PageScan {
        PageScan {
            page,
            geom: PageGeom {
                crop: [0.0, 0.0, 100.0, 100.0],
                rotate: crate::geom::Rot90::R0,
            },
            text_runs: 1,
            signature,
            id: Identity {
                number: Some(nr.into()),
                variant: None,
                title: String::new(),
                method: IdMethod::TitleBlock,
                looks_like_list: false,
                number_bbox: None,
            },
            tokens: vec![],
            warning: None,
        }
    }

    #[test]
    fn only_same_number_and_text_in_another_file_is_a_candidate() {
        let (a, b) = (PathBuf::from("a.pdf"), PathBuf::from("b.pdf"));
        let pages = vec![
            (
                a.clone(),
                vec![page("TO-1", 0, 1), page("TO-2", 1, 2), page("TO-1", 2, 1)],
            ),
            (
                b.clone(),
                vec![page("TO-1", 0, 1), page("TO-2", 1, 9), page("TO-3", 2, 3)],
            ),
        ];
        let c = vector_candidates(&pages);
        assert_eq!(
            c,
            vec![(a.clone(), 0, b.clone(), 0), (a.clone(), 2, b.clone(), 0)]
        );
    }

    #[test]
    fn manual_link_shares_the_key_of_b() {
        let mut a = issue(&[("?a#1", None), ("?a#2", None)]);
        let mut b = issue(&[("?b#1", None), ("?b#2", None)]);
        let key = link_sheets(&mut a, &mut b, "?a#2", "?b#1", LinkMethod::Manual).unwrap();
        assert_eq!(key, "?b#1");
        assert_eq!(a.links[0].method, LinkMethod::Manual);
        assert_eq!(a.sheets["?b#1"].key, "?b#1");
        assert!(a.booklets["boekje"].contains(&"?b#1".to_string()));
    }

    #[test]
    fn key_of_a_is_kept_when_only_a_has_a_number() {
        let mut a = issue(&[("TO-200", Some("TO-200"))]);
        let mut b = issue(&[("?Basis#1", None)]);
        let key = link_sheets(&mut a, &mut b, "TO-200", "?Basis#1", LinkMethod::Manual).unwrap();
        assert_eq!(key, "TO-200");
        assert!(b.sheets.contains_key("TO-200"));
    }

    #[test]
    fn unlink_restores_both_keys() {
        let mut a = issue(&[("?a#1", None)]);
        let mut b = issue(&[("?b#1", None)]);
        let key = link_sheets(&mut a, &mut b, "?a#1", "?b#1", LinkMethod::Manual).unwrap();
        let (ka, kb) = unlink_sheets(&mut a, &mut b, &key).unwrap();
        assert_eq!((ka.as_str(), kb.as_str()), ("?a#1", "?b#1"));
        assert!(a.sheets.contains_key("?a#1") && b.sheets.contains_key("?b#1"));
        assert!(a.links.is_empty());
    }

    #[test]
    fn unlink_of_a_pair_on_name_keeps_the_sheets_apart() {
        let mut a = issue(&[("?x#1", None)]);
        let mut b = issue(&[("?x#1", None)]);
        let (ka, kb) = unlink_sheets(&mut a, &mut b, "?x#1").unwrap();
        assert_ne!(ka, kb);
        assert!(a.sheets.contains_key(&ka) && b.sheets.contains_key(&kb));
        assert!(!a.sheets.contains_key(&kb) && !b.sheets.contains_key(&ka));
    }

    #[test]
    fn labels_per_method() {
        let l = |method, score| SheetLink {
            key: "k".into(),
            key_a: "a".into(),
            key_b: "b".into(),
            method,
            score,
            low: false,
        };
        assert_eq!(
            l(LinkMethod::Content, Some(0.87)).label(),
            "op inhoud (87 %)"
        );
        assert_eq!(l(LinkMethod::Doubt, Some(0.62)).label(), "twijfel (62 %)");
        assert_eq!(l(LinkMethod::Order, Some(0.9)).label(), "op paginavolgorde");
        assert_eq!(l(LinkMethod::Manual, None).label(), "handmatig");
        assert!(l(LinkMethod::Content, Some(0.87)).note().is_none());
        assert!(l(LinkMethod::Doubt, Some(0.62)).note().is_some());
    }

    #[test]
    fn linking_a_matched_sheet_is_refused() {
        let mut a = issue(&[("TO-100", Some("TO-100"))]);
        let mut b = issue(&[("TO-100", Some("TO-100")), ("TO-200", Some("TO-200"))]);
        assert!(link_sheets(&mut a, &mut b, "TO-100", "TO-200", LinkMethod::Manual).is_err());
    }
}
