//! Bladen koppelen op inhoud. Na koppelen op sleutel blijven bladen over die geen bruikbare
//! naam hebben, of waarvan de naam veranderde. Die worden hier A x B gescoord op lijnwerk en
//! tekst en globaal optimaal toegewezen; wat onder `pair.min_score` blijft, is vervallen of nieuw.
//!
//! Per blad wordt een keer gerenderd (inktkaart op lage resolutie); scoren per paar is daarna
//! alleen nog bitrekenen, dus geen render per paar.

use crate::choices::{self, Applied, Choices};
use crate::compare::Runner;
use crate::config::{Config, PairCfg};
use crate::geom::Affine;
use crate::issue::{Issue, LinkMethod, SheetLink, SheetRef};
use crate::pdf::{self, Doc, Gray, TextRun};
use crate::textdiff::Normalizer;
use anyhow::Result;
use rayon::prelude::*;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

/// Lijnwerk van een blad als bitkaart, een bit per pixel, gedilateerd.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InkMap {
    pub w: usize,
    pub h: usize,
    /// 64-bits woorden per rij.
    pub wpr: usize,
    pub bits: Vec<u64>,
    /// Aantal inktpixels.
    pub count: u64,
    /// Inkt per kolom en per rij (profielen voor de grove uitlijning).
    pub cols: Vec<u32>,
    pub rows: Vec<u32>,
}

impl InkMap {
    /// Inkt = donkerder dan `level`, daarna `dilate` pixels verdikt.
    pub fn from_gray(g: &Gray, level: u8, dilate: usize) -> InkMap {
        let d = if dilate > 0 {
            crate::raster::min_filter(g, dilate)
        } else {
            g.clone()
        };
        let wpr = d.w.div_ceil(64).max(1);
        let mut bits = vec![0u64; wpr * d.h];
        let (mut cols, mut rows) = (vec![0u32; d.w], vec![0u32; d.h]);
        let mut count = 0u64;
        for y in 0..d.h {
            for x in 0..d.w {
                if d.data[y * d.w + x] < level {
                    bits[y * wpr + x / 64] |= 1u64 << (x % 64);
                    cols[x] += 1;
                    rows[y] += 1;
                    count += 1;
                }
            }
        }
        InkMap {
            w: d.w,
            h: d.h,
            wpr,
            bits,
            count,
            cols,
            rows,
        }
    }

    /// Binair formaat voor de overdracht van werker naar ouder. Als JSON kost een A0-kaart bijna
    /// een megabyte tekst, en de ouder leest ze allemaal.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(32 + self.bits.len() * 8 + (self.w + self.h) * 4);
        for n in [self.w as u64, self.h as u64, self.wpr as u64, self.count] {
            v.extend_from_slice(&n.to_le_bytes());
        }
        for b in &self.bits {
            v.extend_from_slice(&b.to_le_bytes());
        }
        for c in self.cols.iter().chain(&self.rows) {
            v.extend_from_slice(&c.to_le_bytes());
        }
        v
    }

    /// Omgekeerde van [`InkMap::to_bytes`]; `None` als de lengte niet klopt.
    pub fn from_bytes(s: &[u8]) -> Option<InkMap> {
        let word = |i: usize| -> Option<u64> {
            Some(u64::from_le_bytes(
                s.get(i * 8..i * 8 + 8)?.try_into().ok()?,
            ))
        };
        let (w, h, wpr, count) = (
            word(0)? as usize,
            word(1)? as usize,
            word(2)? as usize,
            word(3)?,
        );
        let n = wpr.checked_mul(h)?;
        if s.len() != 32 + n * 8 + (w + h) * 4 {
            return None;
        }
        let bits = s[32..32 + n * 8]
            .chunks_exact(8)
            .map(|c| u64::from_le_bytes(c.try_into().expect("8 bytes")))
            .collect();
        let mut prof = s[32 + n * 8..]
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes(c.try_into().expect("4 bytes")));
        let cols = prof.by_ref().take(w).collect();
        let rows = prof.collect();
        Some(InkMap {
            w,
            h,
            wpr,
            bits,
            count,
            cols,
            rows,
        })
    }

    fn get(&self, x: usize, y: usize) -> bool {
        self.bits[y * self.wpr + x / 64] >> (x % 64) & 1 == 1
    }

    /// De kaart een kwartslag gedraaid: met de klok mee, of ertegenin.
    pub fn quarter(&self, clockwise: bool) -> InkMap {
        let (w, h) = (self.h, self.w);
        let wpr = w.div_ceil(64).max(1);
        let mut bits = vec![0u64; wpr * h];
        let (mut cols, mut rows) = (vec![0u32; w], vec![0u32; h]);
        for y in 0..self.h {
            for x in 0..self.w {
                if self.get(x, y) {
                    let (nx, ny) = if clockwise {
                        (self.h - 1 - y, x)
                    } else {
                        (y, self.w - 1 - x)
                    };
                    bits[ny * wpr + nx / 64] |= 1u64 << (nx % 64);
                    cols[nx] += 1;
                    rows[ny] += 1;
                }
            }
        }
        InkMap {
            w,
            h,
            wpr,
            bits,
            count: self.count,
            cols,
            rows,
        }
    }

    /// Aantal pixels dat in beide kaarten inkt is, met B verschoven: b(x, y) ligt op a(x+dx, y+dy).
    fn intersection(&self, b: &InkMap, dx: i32, dy: i32) -> u64 {
        if dx < 0 {
            return b.intersection(self, -dx, -dy);
        }
        let (ws, bs) = (dx as usize / 64, dx as usize % 64);
        let mut n = 0u64;
        for yb in 0..b.h {
            let ya = yb as i64 + dy as i64;
            if ya < 0 || ya >= self.h as i64 {
                continue;
            }
            let ra = &self.bits[ya as usize * self.wpr..(ya as usize + 1) * self.wpr];
            let rb = &b.bits[yb * b.wpr..(yb + 1) * b.wpr];
            for (i, wb) in rb.iter().enumerate() {
                let j = i + ws;
                if j >= ra.len() {
                    break;
                }
                let mut wa = ra[j] >> bs;
                if bs > 0 && j + 1 < ra.len() {
                    wa |= ra[j + 1] << (64 - bs);
                }
                n += (wa & wb).count_ones() as u64;
            }
        }
        n
    }

    /// IoU van de inkt onder een verschuiving. Bewust niet het aandeel van het hele blad: dat is
    /// grotendeels wit en zou elk paar hoog laten scoren.
    pub fn iou(&self, b: &InkMap, dx: i32, dy: i32) -> f64 {
        let i = self.intersection(b, dx, dy);
        let u = self.count + b.count - i;
        if u == 0 {
            0.0
        } else {
            i as f64 / u as f64
        }
    }
}

/// Verschuiving van profiel `b` t.o.v. `a` met de grootste correlatie.
fn best_profile_shift(a: &[u32], b: &[u32], max: i32) -> i32 {
    let mut best = (0u64, 0i32);
    for s in -max..=max {
        let mut c = 0u64;
        for (i, &vb) in b.iter().enumerate() {
            let j = i as i64 + s as i64;
            if j >= 0 && (j as usize) < a.len() {
                c += a[j as usize] as u64 * vb as u64;
            }
        }
        if c > best.0 || (c == best.0 && s.abs() < best.1.abs()) {
            best = (c, s);
        }
    }
    best.1
}

/// Lijnwerk-overlap na grove uitlijning: de beste van 'zoals het ligt' en 'op de profielen
/// gelegd'. Een tekening kan binnen een vast kader verschuiven; dan past een van de twee.
pub fn line_score(a: &InkMap, b: &InkMap, max_shift_px: i32) -> f64 {
    if a.count == 0 || b.count == 0 {
        return 0.0;
    }
    let (dx, dy) = (
        best_profile_shift(&a.cols, &b.cols, max_shift_px),
        best_profile_shift(&a.rows, &b.rows, max_shift_px),
    );
    let base = a.iou(b, 0, 0);
    if dx == 0 && dy == 0 {
        base
    } else {
        base.max(a.iou(b, dx, dy))
    }
}

/// Woorden van een blad zonder printstempels en datums: gesorteerde, unieke hashes.
pub struct Tokenizer {
    norm: Normalizer,
    date: Regex,
}

impl Tokenizer {
    pub fn new(cfg: &Config) -> Result<Tokenizer> {
        Ok(Tokenizer {
            norm: Normalizer::new(&cfg.normalize.stamp_patterns)?,
            date: Regex::new(r"\b\d{1,4}[-/.]\d{1,2}[-/.]\d{1,4}\b")?,
        })
    }

    pub fn tokens(&self, runs: &[TextRun]) -> Vec<u64> {
        use std::hash::{Hash, Hasher};
        let mut out: Vec<u64> = Vec::new();
        for r in runs {
            if self.norm.has_stamp(&r.text) {
                continue;
            }
            let t = self.date.replace_all(&r.text, " ").to_lowercase();
            for w in t.split(|c: char| !c.is_alphanumeric()) {
                // losse tekens (revisieletter, streepje) zeggen niets over welk blad het is
                if w.chars().count() < 2 {
                    continue;
                }
                let mut h = std::collections::hash_map::DefaultHasher::new();
                w.hash(&mut h);
                out.push(h.finish());
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// Jaccard op twee gesorteerde sets.
pub fn jaccard(a: &[u64], b: &[u64]) -> f64 {
    let (mut i, mut j, mut n) = (0, 0, 0usize);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                n += 1;
                i += 1;
                j += 1;
            }
        }
    }
    let u = a.len() + b.len() - n;
    if u == 0 {
        0.0
    } else {
        n as f64 / u as f64
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PairScore {
    /// Inktoverlap 0..1; `None` als een blad geen lijnwerk heeft.
    pub lines: Option<f64>,
    /// Tekstoverlap 0..1; `None` als een blad te weinig tekst heeft.
    pub text: Option<f64>,
    pub score: f64,
}

/// Formaten gelijk binnen de tolerantie (harde voorwaarde).
pub fn same_format(a: (f64, f64), b: (f64, f64), tol: f64) -> bool {
    (a.0 - b.0).abs() <= tol * a.0.max(b.0) && (a.1 - b.1).abs() <= tol * a.1.max(b.1)
}

/// Gelijk formaat, of gelijk na een kwartslag.
fn fits(a: (f64, f64), b: (f64, f64), tol: f64) -> bool {
    same_format(a, b, tol) || same_format(a, (b.1, b.0), tol)
}

/// Score van een bladpaar. Ontbreekt een van beide onderdelen, dan telt alleen de ander; een
/// scan zonder tekstlaag is geen bewijs dat het een ander blad is. Past het formaat pas na een
/// kwartslag, dan telt de beste van de twee draairichtingen.
pub fn score_pair(a: &SheetRef, b: &SheetRef, p: &PairCfg) -> PairScore {
    let turned = b.ink.as_ref().map(|i| (i.quarter(true), i.quarter(false)));
    score_with(a, b, turned.as_ref(), p)
}

/// Als [`score_pair`], met de al gedraaide kaarten van `b`.
fn score_with(
    a: &SheetRef,
    b: &SheetRef,
    turned: Option<&(InkMap, InkMap)>,
    p: &PairCfg,
) -> PairScore {
    let none = PairScore {
        lines: None,
        text: None,
        score: 0.0,
    };
    let (sa, sb) = (a.geom.display_size(), b.geom.display_size());
    if !fits(sa, sb, p.size_tol) {
        return none;
    }
    let max_shift = (p.max_shift_mm / 25.4 * p.dpi).round() as i32;
    let lines = match (&a.ink, &b.ink) {
        (Some(x), Some(y)) if x.count > 0 && y.count > 0 => {
            if same_format(sa, sb, p.size_tol) {
                Some(line_score(x, y, max_shift))
            } else {
                turned.map(|(cw, ccw)| {
                    line_score(x, cw, max_shift).max(line_score(x, ccw, max_shift))
                })
            }
        }
        _ => None,
    };
    let text = (a.tokens.len() >= p.min_tokens && b.tokens.len() >= p.min_tokens)
        .then(|| jaccard(&a.tokens, &b.tokens));
    let score = match (lines, text) {
        (Some(l), Some(t)) => {
            (p.weight_lines * l + p.weight_text * t) / (p.weight_lines + p.weight_text)
        }
        (Some(l), None) => l,
        (None, Some(t)) => t,
        (None, None) => 0.0,
    };
    PairScore { lines, text, score }
}

/// Toewijzing die de som van `w` maximaliseert (Hongaarse methode, O(n^2 m)). `w[i][j]` >= 0;
/// geeft per rij de gekozen kolom. Een gewicht 0 betekent 'liever niet' en wordt niet gekoppeld.
pub fn assign_max(w: &[Vec<f64>]) -> Vec<Option<usize>> {
    let n = w.len();
    let m = w.first().map_or(0, |r| r.len());
    if n == 0 || m == 0 {
        return vec![None; n];
    }
    if n > m {
        let t: Vec<Vec<f64>> = (0..m).map(|j| w.iter().map(|r| r[j]).collect()).collect();
        let mut out = vec![None; n];
        for (j, i) in assign_max(&t).into_iter().enumerate() {
            if let Some(i) = i {
                out[i] = Some(j);
            }
        }
        return out;
    }
    // een score die geen getal is (eigen config met beide gewichten 0) telt als 'liever niet':
    // met NaN vindt de zoeklus nooit een kolom en komt hij niet terug
    let cost = |i: usize, j: usize| {
        let x = w[i - 1][j - 1];
        if x.is_finite() {
            -x
        } else {
            0.0
        }
    };
    let (mut u, mut v) = (vec![0.0; n + 1], vec![0.0; m + 1]);
    let (mut p, mut way) = (vec![0usize; m + 1], vec![0usize; m + 1]);
    for i in 1..=n {
        p[0] = i;
        let mut j0 = 0usize;
        let mut minv = vec![f64::INFINITY; m + 1];
        let mut used = vec![false; m + 1];
        loop {
            used[j0] = true;
            let (i0, mut delta, mut j1) = (p[j0], f64::INFINITY, 0usize);
            for j in 1..=m {
                if !used[j] {
                    let cur = cost(i0, j) - u[i0] - v[j];
                    if cur < minv[j] {
                        minv[j] = cur;
                        way[j] = j0;
                    }
                    if minv[j] < delta {
                        delta = minv[j];
                        j1 = j;
                    }
                }
            }
            for j in 0..=m {
                if used[j] {
                    u[p[j]] += delta;
                    v[j] -= delta;
                } else {
                    minv[j] -= delta;
                }
            }
            j0 = j1;
            if p[j0] == 0 {
                break;
            }
        }
        loop {
            let j1 = way[j0];
            p[j0] = p[j1];
            j0 = j1;
            if j0 == 0 {
                break;
            }
        }
    }
    let mut out = vec![None; n];
    for j in 1..=m {
        if p[j] > 0 && w[p[j] - 1][j - 1] > 0.0 {
            out[p[j] - 1] = Some(j - 1);
        }
    }
    out
}

/// Rendert de inktkaart van een pagina.
pub fn render_ink(file: &std::path::Path, page: usize, p: &PairCfg) -> Result<InkMap> {
    ink_of(&Doc::open(file)?, page, p)
}

/// Inktkaart uit een open document: een groot boekje openen kost meer dan een blad renderen.
pub fn ink_of(doc: &Doc, page: usize, p: &PairCfg) -> Result<InkMap> {
    let pg = doc.page(page)?;
    let geom = pdf::page_geom(&pg)?;
    let g = pdf::render_page_gray(&pg, p.dpi, &Affine::IDENTITY, geom.display_size())?;
    Ok(InkMap::from_gray(&g, p.ink_level, p.dilate_px))
}

/// Bovengrens van de kaartencache; daarboven gaan de oudste kaarten weg.
const INK_CACHE_MAX_BYTES: u64 = 1 << 30;

fn ink_cache_dir() -> std::path::PathBuf {
    std::env::temp_dir().join("sheetdiff-inkt")
}

/// Bestandsnaam van een kaart in de cache: verandert mee met het bestand en de instellingen.
fn ink_cache_name(s: &SheetRef, p: &PairCfg) -> Option<String> {
    let meta = std::fs::metadata(&s.file).ok()?;
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    let text = format!(
        "1|{}|{}|{mtime}|{}|{}|{}|{}",
        s.file.display(),
        meta.len(),
        s.page,
        p.dpi,
        p.ink_level,
        p.dilate_px
    );
    let mut h = 0xcbf29ce484222325u64;
    for b in text.as_bytes() {
        h = (h ^ *b as u64).wrapping_mul(0x100000001b3);
    }
    Some(format!("{h:016x}.ink"))
}

/// Haalt de oudste kaarten weg als de cache te groot is. Fouten zijn niet erg: het is een cache.
fn trim_ink_cache(dir: &std::path::Path, max: u64) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<_> = rd
        .filter_map(|e| {
            let e = e.ok()?;
            let m = e.metadata().ok()?;
            Some((m.modified().ok()?, m.len(), e.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    files.sort();
    for (_, len, path) in files {
        if total <= max {
            break;
        }
        if std::fs::remove_file(path).is_ok() {
            total -= len;
        }
    }
}

/// Vult de ontbrekende inktkaarten: uit de cache, dan over werkerprocessen als dat kan, wat dan
/// nog ontbreekt in dit proces. Een onleesbaar blad houdt geen kaart en koppelt dan alleen op tekst.
fn fill_ink(sheets: &mut [&mut SheetRef], cfg: &Config, runner: &Runner) -> Result<()> {
    let names: Vec<Option<String>> = sheets
        .iter()
        .map(|s| {
            cfg.pair
                .ink_cache
                .then(|| ink_cache_name(s, &cfg.pair))
                .flatten()
        })
        .collect();
    let dir = ink_cache_dir();
    let cached: Vec<Option<InkMap>> = names
        .par_iter()
        .map(|n| InkMap::from_bytes(&std::fs::read(dir.join(n.as_ref()?)).ok()?))
        .collect();
    for (s, c) in sheets.iter_mut().zip(cached) {
        if s.ink.is_none() {
            s.ink = c.map(Arc::new);
        }
    }
    let todo: Vec<usize> = (0..sheets.len())
        .filter(|&i| sheets[i].ink.is_none())
        .collect();
    let mut got: HashMap<usize, Option<InkMap>> = HashMap::new();
    if let Runner::Processes { exe, .. } = runner {
        if todo.len() > 1 {
            let tasks: Vec<crate::parallel::InkTask> = todo
                .iter()
                .map(|&i| crate::parallel::InkTask {
                    id: i,
                    file: sheets[i].file.clone(),
                    page: sheets[i].page,
                })
                .collect();
            got = crate::parallel::render_inks(tasks, cfg, exe)?;
        }
    }
    let todo_store = todo.clone();
    for i in todo {
        let ink = match got.remove(&i) {
            Some(m) => {
                if m.is_none() {
                    tracing::warn!("{}: inktkaart vastgelopen, alleen op tekst", sheets[i].key);
                }
                m
            }
            None => match render_ink(&sheets[i].file, sheets[i].page, &cfg.pair) {
                Ok(m) => Some(m),
                Err(e) => {
                    tracing::warn!("{}: inktkaart mislukt: {e:#}", sheets[i].key);
                    None
                }
            },
        };
        sheets[i].ink = ink.map(Arc::new);
    }
    if cfg.pair.ink_cache && !todo_store.is_empty() {
        let _ = std::fs::create_dir_all(&dir);
        todo_store.par_iter().for_each(|&i| {
            if let (Some(n), Some(m)) = (&names[i], &sheets[i].ink) {
                let tmp = dir.join(format!("{n}.{}", std::process::id()));
                if std::fs::write(&tmp, m.to_bytes()).is_ok() {
                    let _ = std::fs::rename(&tmp, dir.join(n));
                }
            }
        });
        trim_ink_cache(&dir, INK_CACHE_MAX_BYTES);
    }
    Ok(())
}

/// Scorematrix voor de CLI en de kalibratie.
pub struct ScoreMatrix {
    pub keys_a: Vec<String>,
    pub keys_b: Vec<String>,
    pub scores: Vec<Vec<PairScore>>,
}

/// Sleutels in leesvolgorde: bestand, dan pagina.
fn reading_order(i: &Issue, keys: &BTreeSet<String>) -> Vec<String> {
    let mut v: Vec<&SheetRef> = keys.iter().map(|k| &i.sheets[k]).collect();
    v.sort_by(|x, y| x.file.cmp(&y.file).then(x.page.cmp(&y.page)));
    v.into_iter().map(|s| s.key.clone()).collect()
}

fn pools(a: &Issue, b: &Issue, p: &PairCfg) -> (BTreeSet<String>, BTreeSet<String>) {
    // Een naam zonder nummer (bestand#pagina) bewijst niets: een los bestand met dezelfde
    // naam mag het juiste paar niet wegkapen. Die bladen doen dus ook mee.
    let open = |k: &String, x: &Issue, y: &Issue| {
        // handmatig gekoppeld staat vast
        if a.links.iter().any(|l| l.key == *k) {
            return false;
        }
        match y.sheets.get(k) {
            None => true,
            Some(o) => !p.use_names || o.number.is_none() || x.sheets[k].number.is_none(),
        }
    };
    (
        a.sheets.keys().filter(|k| open(k, a, b)).cloned().collect(),
        b.sheets.keys().filter(|k| open(k, b, a)).cloned().collect(),
    )
}

/// Scoort alle bladen uit de twee lijsten, parallel over de rijen.
fn matrix(a: &Issue, b: &Issue, ka: &[String], kb: &[String], p: &PairCfg) -> Vec<Vec<PairScore>> {
    // gedraaide kaarten alleen voor bladen die aan de overkant een blad op zijn kant hebben
    let turned: HashMap<&String, (InkMap, InkMap)> = kb
        .par_iter()
        .filter_map(|y| {
            let s = &b.sheets[y];
            let size = s.geom.display_size();
            let on_side = ka.iter().any(|x| {
                let o = a.sheets[x].geom.display_size();
                !same_format(o, size, p.size_tol) && fits(o, size, p.size_tol)
            });
            let ink = s.ink.as_ref().filter(|_| on_side)?;
            Some((y, (ink.quarter(true), ink.quarter(false))))
        })
        .collect();
    ka.par_iter()
        .map(|x| {
            let sa = &a.sheets[x];
            kb.iter()
                .map(|y| score_with(sa, &b.sheets[y], turned.get(y), p))
                .collect()
        })
        .collect()
}

/// Rendert wat nodig is voor de opgegeven sleutels. Alleen bladen waarvan aan de overkant een
/// blad van gelijk formaat bestaat, hoeven een kaart.
fn prepare(
    a: &mut Issue,
    b: &mut Issue,
    ka: &[String],
    kb: &[String],
    cfg: &Config,
    runner: &Runner,
) -> Result<()> {
    let tol = cfg.pair.size_tol;
    let size = |i: &Issue, k: &String| i.sheets[k].geom.display_size();
    let need_a: BTreeSet<String> = ka
        .iter()
        .filter(|x| kb.iter().any(|y| fits(size(a, x), size(b, y), tol)))
        .cloned()
        .collect();
    let need_b: BTreeSet<String> = kb
        .iter()
        .filter(|y| ka.iter().any(|x| fits(size(a, x), size(b, y), tol)))
        .cloned()
        .collect();
    let mut list: Vec<&mut SheetRef> = a
        .sheets
        .iter_mut()
        .filter(|(k, _)| need_a.contains(*k))
        .map(|(_, s)| s)
        .chain(
            b.sheets
                .iter_mut()
                .filter(|(k, _)| need_b.contains(*k))
                .map(|(_, s)| s),
        )
        .collect();
    fill_ink(&mut list, cfg, runner)
}

/// Alle scores tussen de bladen die op inhoud gekoppeld zouden worden.
pub fn score_matrix(
    a: &mut Issue,
    b: &mut Issue,
    cfg: &Config,
    runner: &Runner,
) -> Result<ScoreMatrix> {
    let (pa, pb) = pools(a, b, &cfg.pair);
    let (ka, kb) = (reading_order(a, &pa), reading_order(b, &pb));
    prepare(a, b, &ka, &kb, cfg, runner)?;
    let scores = matrix(a, b, &ka, &kb, &cfg.pair);
    Ok(ScoreMatrix {
        keys_a: ka,
        keys_b: kb,
        scores,
    })
}

/// Hernoemt sleutels in een keer, zodat een wisseling van twee sleutels niet botst.
fn rename_all(i: &mut Issue, map: &HashMap<String, String>) {
    let mut moved = Vec::new();
    for (old, new) in map {
        if let Some(s) = i.sheets.remove(old) {
            moved.push((new.clone(), s));
        }
    }
    for (new, mut s) in moved {
        s.key = new.clone();
        i.sheets.insert(new, s);
    }
    for k in i.booklets.values_mut().flatten() {
        if let Some(n) = map.get(k) {
            *k = n.clone();
        }
    }
}

fn pct(v: f64) -> String {
    format!("{:.0} %", v * 100.0)
}

/// Waarom een blad los bleef, met de beste kandidaat aan de overkant.
fn unpaired_note(best: Option<(f64, &String)>, side: &str, p: &PairCfg) -> String {
    match best {
        Some((s, k)) if s > 0.0 => {
            let why = if s >= p.min_score {
                "die past beter bij een ander blad".to_string()
            } else {
                format!("onder de drempel van {}", pct(p.min_score))
            };
            let k = crate::issue::display_key(k, 0);
            format!("beste kandidaat in {side}: '{k}' ({}), {why}", pct(s))
        }
        _ => format!("geen blad van gelijk formaat in {side}"),
    }
}

/// Hoeveel twee bladen op naam op elkaar lijken, 0..1: gezamenlijk begin van de bestandsnaam
/// ('_wijzA' erachter telt dus nauwelijks) en dezelfde pagina. Alleen voor gelijke scores.
fn name_likeness(a: &SheetRef, b: &SheetRef) -> f64 {
    let stem = |s: &SheetRef| {
        s.file
            .file_stem()
            .map(|x| x.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    };
    let (x, y) = (stem(a), stem(b));
    let common = x.chars().zip(y.chars()).take_while(|(p, q)| p == q).count();
    let longest = x.chars().count().max(y.chars().count()).max(1);
    let same_page = if a.page == b.page { 1.0 } else { 0.0 };
    (common as f64 / longest as f64 + same_page) / 2.0
}

/// Koppelt de overgebleven bladen op inhoud. Geeft het aantal koppelingen op inhoud terug.
pub fn pair_sheets(a: &mut Issue, b: &mut Issue, cfg: &Config, runner: &Runner) -> Result<usize> {
    pair_sheets_except(a, b, cfg, runner, &[])
}

/// Koppelt met de keuzes van de gebruiker: handmatige koppelingen staan vast, ontkoppelde paren
/// worden niet opnieuw gemaakt. `by_order`: `a` en `b` komen uit [`crate::issue::scan_pages`].
pub fn pair_with_choices(
    a: &mut Issue,
    b: &mut Issue,
    cfg: &Config,
    runner: &Runner,
    choices: &Choices,
    by_order: bool,
) -> Result<Applied> {
    if by_order {
        pair_by_order(a, b, cfg, runner)?;
        let mut applied = choices::apply(choices, a, b);
        choices::apply_aligns(choices, a, b, &mut applied);
        return Ok(applied);
    }
    let mut applied = choices::apply(choices, a, b);
    pair_sheets_except(a, b, cfg, runner, &applied.forbidden)?;
    choices::apply_aligns(choices, a, b, &mut applied);
    Ok(applied)
}

/// Als [`pair_sheets`]; de paren in `forbidden` (sleutel A, sleutel B) worden nooit gemaakt.
pub fn pair_sheets_except(
    a: &mut Issue,
    b: &mut Issue,
    cfg: &Config,
    runner: &Runner,
    forbidden: &[(String, String)],
) -> Result<usize> {
    let p = &cfg.pair;
    if !p.content {
        return Ok(0);
    }
    let (pa, pb) = pools(a, b, p);
    if pa.is_empty() || pb.is_empty() {
        return Ok(0);
    }
    let (ka, kb) = (reading_order(a, &pa), reading_order(b, &pb));
    prepare(a, b, &ka, &kb, cfg, runner)?;
    let mut scores = matrix(a, b, &ka, &kb, p);
    for (x, y) in forbidden {
        if let (Some(i), Some(j)) = (
            ka.iter().position(|k| k == x),
            kb.iter().position(|k| k == y),
        ) {
            scores[i][j].score = 0.0;
        }
    }

    // Gelijke score: naam en paginavolgorde geven de doorslag. Beide zijn klein genoeg om nooit
    // een echt scoreverschil om te draaien. Bladen met gelijke inhoud (legenda, voorblad) komen
    // zo terug in het eigen boekje.
    const ORDER_WEIGHT: f64 = 1e-4;
    const NAME_BONUS: f64 = 1e-3;
    let (na, nb) = (ka.len().max(2) as f64 - 1.0, kb.len().max(2) as f64 - 1.0);
    let w: Vec<Vec<f64>> = (0..ka.len())
        .map(|i| {
            (0..kb.len())
                .map(|j| {
                    let s = scores[i][j].score;
                    if s < p.min_score {
                        return 0.0;
                    }
                    let shift = (i as f64 / na - j as f64 / nb).abs();
                    let name = if p.use_names {
                        NAME_BONUS * name_likeness(&a.sheets[&ka[i]], &b.sheets[&kb[j]])
                    } else {
                        0.0
                    };
                    s + name - ORDER_WEIGHT * shift
                })
                .collect()
        })
        .collect();
    let chosen = assign_max(&w);

    // nieuwe sleutels bepalen
    let (mut map_a, mut map_b) = (HashMap::new(), HashMap::new());
    let mut used: BTreeSet<String> = BTreeSet::new();
    let mut links = Vec::new();
    let mut taken_b = vec![false; kb.len()];
    for (i, c) in chosen.iter().enumerate() {
        let Some(j) = *c else { continue };
        taken_b[j] = true;
        let (sa, sb) = (&a.sheets[&ka[i]], &b.sheets[&kb[j]]);
        let base = if sa.number.is_some() && sb.number.is_none() {
            &ka[i]
        } else {
            &kb[j]
        };
        let mut key = base.clone();
        let mut n = 2;
        while !used.insert(key.clone()) {
            key = format!("{base} ({n})");
            n += 1;
        }
        let score = scores[i][j].score;
        let same_name = p.use_names && ka[i] == kb[j];
        // Twee verschillende nummers op bijna dezelfde tekening: dat kan een hernummerd blad
        // zijn, maar net zo goed de verdieping erboven. Nooit zonder twijfel.
        let other_number = p.use_names && sa.number.is_some() && sb.number.is_some();
        let doubt = score < p.doubt_score || (other_number && !same_name);
        if !same_name || doubt {
            links.push(SheetLink {
                key: key.clone(),
                key_a: ka[i].clone(),
                key_b: kb[j].clone(),
                method: if doubt {
                    LinkMethod::Doubt
                } else {
                    LinkMethod::Content
                },
                score: Some(score),
                low: false,
            });
        }
        map_a.insert(ka[i].clone(), key.clone());
        map_b.insert(kb[j].clone(), key);
    }
    // Niet gekoppeld: de eigen sleutel, tenzij die nu van een ander paar is of aan de overkant
    // ook los staat; anders zouden ze alsnog op sleutel samenvallen.
    for (i, k) in ka.iter().enumerate() {
        if chosen[i].is_none() {
            let key = if used.contains(k) {
                format!("{k} (A)")
            } else {
                k.clone()
            };
            used.insert(key.clone());
            map_a.insert(k.clone(), key);
        }
    }
    for (j, k) in kb.iter().enumerate() {
        if !taken_b[j] {
            let key = if used.contains(k) {
                format!("{k} (B)")
            } else {
                k.clone()
            };
            used.insert(key.clone());
            map_b.insert(k.clone(), key);
        }
    }

    // beste kandidaat bij elk los blad: de onderbouwing van 'vervallen' of 'nieuw' in het rapport
    let mut notes_a = BTreeMap::new();
    let mut notes_b = BTreeMap::new();
    for (i, k) in ka.iter().enumerate() {
        if chosen[i].is_none() {
            let best = (0..kb.len())
                .map(|j| (scores[i][j].score, &kb[j]))
                .max_by(|x, y| x.0.total_cmp(&y.0));
            notes_a.insert(map_a[k].clone(), unpaired_note(best, "B", p));
        }
    }
    for (j, k) in kb.iter().enumerate() {
        if !taken_b[j] {
            let best = (0..ka.len())
                .map(|i| (scores[i][j].score, &ka[i]))
                .max_by(|x, y| x.0.total_cmp(&y.0));
            notes_b.insert(map_b[k].clone(), unpaired_note(best, "A", p));
        }
    }

    map_a.retain(|k, v| k != v);
    map_b.retain(|k, v| k != v);
    rename_all(a, &map_a);
    rename_all(b, &map_b);
    let n = links.len();
    a.links.extend(links);
    a.notes.extend(notes_a);
    b.notes.extend(notes_b);
    Ok(n)
}

/// Pagina i van A tegen pagina i van B, namen genegeerd; `a` en `b` komen uit
/// [`crate::issue::scan_pages`]. Het overschot wordt vervallen of nieuw. Scoort elk paar, zodat
/// een verschoven boekje als waarschuwing naar boven komt.
pub fn pair_by_order(a: &mut Issue, b: &mut Issue, cfg: &Config, runner: &Runner) -> Result<usize> {
    let ka: Vec<String> = a.sheets.keys().cloned().collect();
    let kb: Vec<String> = b.sheets.keys().cloned().collect();
    let n = ka.len().min(kb.len());
    prepare(a, b, &ka[..n], &kb[..n], cfg, runner)?;
    let scores: Vec<PairScore> = (0..n)
        .into_par_iter()
        .map(|i| score_pair(&a.sheets[&ka[i]], &b.sheets[&kb[i]], &cfg.pair))
        .collect();
    let mut map_a = HashMap::new();
    let mut links = Vec::new();
    for i in 0..n {
        let s = scores[i].score;
        if ka[i] != kb[i] {
            map_a.insert(ka[i].clone(), kb[i].clone());
        }
        links.push(SheetLink {
            key: kb[i].clone(),
            key_a: ka[i].clone(),
            key_b: kb[i].clone(),
            method: LinkMethod::Order,
            score: Some(s),
            low: s < cfg.pair.min_score,
        });
    }
    rename_all(a, &map_a);
    a.links.extend(links);
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(w: usize, h: usize, on: &[(usize, usize)]) -> InkMap {
        let mut g = Gray::new(w, h, 255);
        for &(x, y) in on {
            g.data[y * w + x] = 0;
        }
        InkMap::from_gray(&g, 200, 0)
    }

    #[test]
    fn iou_counts_ink_not_paper() {
        let a = map(200, 100, &[(3, 3), (70, 40), (130, 90)]);
        assert_eq!(a.iou(&a, 0, 0), 1.0);
        let b = map(200, 100, &[(3, 3), (71, 40), (131, 90)]);
        // een van de vijf inktpixels overlapt, ook al is 99,9 % van het blad gelijk wit
        assert!((a.iou(&b, 0, 0) - 0.2).abs() < 1e-9);
    }

    #[test]
    fn shift_crosses_word_boundaries_both_ways() {
        let a = map(200, 100, &[(70, 40), (130, 90), (199, 10)]);
        let b = map(200, 100, &[(5, 37), (65, 87), (134, 7)]);
        assert_eq!(a.iou(&b, 65, 3), 1.0);
        assert_eq!(b.iou(&a, -65, -3), 1.0);
        assert!(a.iou(&b, 0, 0) < 0.01);
    }

    #[test]
    fn profile_shift_finds_a_moved_drawing() {
        let pts: Vec<(usize, usize)> = (0..60).map(|i| (20 + i, 20 + (i * 7) % 40)).collect();
        let moved: Vec<(usize, usize)> = pts.iter().map(|&(x, y)| (x + 9, y + 4)).collect();
        let (a, b) = (map(120, 90, &pts), map(120, 90, &moved));
        assert!(a.iou(&b, 0, 0) < 0.2);
        assert_eq!(line_score(&a, &b, 15), 1.0);
    }

    #[test]
    fn jaccard_on_sorted_sets() {
        assert_eq!(jaccard(&[1, 2, 3], &[2, 3, 4]), 0.5);
        assert_eq!(jaccard(&[], &[]), 0.0);
    }

    #[test]
    fn tokens_skip_dates_stamps_and_single_characters() {
        let t = Tokenizer::new(&Config::default()).unwrap();
        let run = |s: &str| TextRun {
            text: s.into(),
            bbox: crate::geom::RectPt::new(0.0, 0.0, 1.0, 1.0),
        };
        let a = t.tokens(&[run("Kozijn 295"), run("04-11-2026"), run("A")]);
        let b = t.tokens(&[run("kozijn 295"), run("28-10-2026"), run("-")]);
        let c = t.tokens(&[run("Kozijn 295"), run("28-10-2026 14:02")]);
        assert_eq!(a, b);
        assert_eq!(a, c);
        assert_eq!(a.len(), 2);
    }

    #[test]
    fn assignment_prefers_the_global_optimum_over_greedy() {
        // gulzig pakt 0.9 en houdt 0.1 over (som 1,0); optimaal is 0.8 + 0.8
        let w = vec![vec![0.9, 0.8], vec![0.8, 0.1]];
        assert_eq!(assign_max(&w), vec![Some(1), Some(0)]);
    }

    #[test]
    fn assignment_leaves_zero_weights_alone_and_handles_rectangles() {
        assert_eq!(
            assign_max(&[vec![0.0, 0.7], vec![0.0, 0.6]]),
            vec![Some(1), None]
        );
        assert_eq!(
            assign_max(&[vec![0.2], vec![0.9], vec![0.1]]),
            vec![None, Some(0), None]
        );
        assert_eq!(assign_max(&[vec![0.0]]), vec![None]);
    }

    #[test]
    fn the_cache_name_changes_with_page_and_settings() {
        let f = std::env::temp_dir().join(format!("sheetdiff-cache-{}.bin", std::process::id()));
        std::fs::write(&f, b"x").unwrap();
        let mut s = SheetRef {
            key: "k".into(),
            number: None,
            title: String::new(),
            file: f.clone(),
            page: 0,
            geom: crate::geom::PageGeom {
                crop: [0.0, 0.0, 10.0, 10.0],
                rotate: crate::geom::Rot90::R0,
            },
            method: crate::identify::IdMethod::None,
            text_runs: 0,
            booklets: vec![],
            signature: 0,
            tokens: Default::default(),
            ink: None,
            align: None,
            file_pages: 1,
        };
        let p = PairCfg::default();
        let first = ink_cache_name(&s, &p).unwrap();
        assert_eq!(ink_cache_name(&s, &p).unwrap(), first);
        s.page = 1;
        assert_ne!(ink_cache_name(&s, &p).unwrap(), first);
        s.page = 0;
        let q = PairCfg {
            dpi: 30.0,
            ..PairCfg::default()
        };
        assert_ne!(ink_cache_name(&s, &q).unwrap(), first);
        // een gewijzigd bestand krijgt een andere kaart
        std::fs::write(&f, b"xy").unwrap();
        assert_ne!(ink_cache_name(&s, &p).unwrap(), first);
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn trimming_removes_the_oldest_until_it_fits() {
        let d = std::env::temp_dir().join(format!("sheetdiff-trim-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        for n in ["a", "b", "c"] {
            std::fs::write(d.join(n), [0u8; 10]).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        trim_ink_cache(&d, 20);
        assert!(!d.join("a").exists() && d.join("b").exists() && d.join("c").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn assignment_survives_a_score_that_is_not_a_number() {
        // beide gewichten 0 in een eigen config gaf 0/0: de toewijzing kwam dan nooit terug
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(assign_max(&[
                vec![f64::NAN, f64::NAN],
                vec![f64::NAN, f64::NAN],
            ]));
        });
        let got = rx.recv_timeout(std::time::Duration::from_secs(5));
        assert_eq!(got, Ok(vec![None, None]));
    }

    #[test]
    fn ink_map_survives_the_binary_format() {
        let a = map(130, 70, &[(3, 3), (70, 40), (129, 69)]);
        let bytes = a.to_bytes();
        assert_eq!(InkMap::from_bytes(&bytes), Some(a));
        assert_eq!(InkMap::from_bytes(&bytes[..bytes.len() - 1]), None);
        assert_eq!(InkMap::from_bytes(&[]), None);
    }

    #[test]
    fn a_quarter_turn_is_undone_by_the_opposite_turn() {
        let pts: Vec<(usize, usize)> = (0..60).map(|i| (20 + i, 20 + (i * 7) % 40)).collect();
        let a = map(130, 70, &pts);
        let cw = a.quarter(true);
        assert_eq!((cw.w, cw.h, cw.count), (70, 130, a.count));
        assert_eq!(cw.quarter(false), a);
        assert_eq!(a.quarter(false).quarter(true), a);
        // een punt linksboven komt met de klok mee rechtsboven
        let dot = map(130, 70, &[(0, 0)]).quarter(true);
        assert!(dot.get(69, 0));
    }

    #[test]
    fn format_is_a_hard_condition_within_tolerance() {
        assert!(same_format((2384.0, 1684.0), (2384.0, 1690.0), 0.015));
        assert!(!same_format((2384.0, 1684.0), (1684.0, 1191.0), 0.015));
    }
}
