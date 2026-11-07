//! Parallel werken over processen. PDFium is per proces geserialiseerd (een globale mutex), dus
//! echte parallelliteit vraagt meerdere processen. De ouder houdt een wachtrij; elke werker krijgt
//! een taak tegelijk over stdin en geeft een regel terug over stdout. Zo blijft niemand wachten op
//! een werker met toevallig de zware bladen.

use crate::compare::{self, SheetDiff, Verdict};
use crate::config::Config;
use crate::issue::SheetRef;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::Mutex;
use std::time::Duration;

/// Zoveel documenten houdt een werker open voor basislagen.
const MAX_OPEN_DOCS: usize = 24;

/// Wat een werker bij het starten leest.
#[derive(Serialize, Deserialize)]
pub struct Job {
    pub cfg: Config,
    /// Inktkaarten komen als `<id>.ink` in deze map.
    #[serde(default)]
    pub ink_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InkTask {
    pub id: usize,
    pub file: PathBuf,
    pub page: usize,
}

/// Een regel op stdin van de werker.
#[derive(Serialize, Deserialize)]
pub enum Task {
    Ink(InkTask),
    Pair(Box<(SheetRef, SheetRef)>),
    /// Pagina's van een bestand lezen voor de scan.
    Scan {
        file: PathBuf,
        from: usize,
        to: usize,
    },
    /// Zelfde vectorinhoud op twee pagina's?
    SameVectors(PathBuf, usize, PathBuf, usize),
    /// Basislaag van een blad voor de viewer; de laag komt binair in `out`.
    Layer {
        src: crate::layer::SheetSrc,
        params: crate::raster::DiffParams,
        out: PathBuf,
    },
}

/// Werkerkant: leest taken van `input` tot die dicht gaat, schrijft een regel per taak naar `out`.
pub fn run_job(job_file: &Path, input: impl BufRead, mut out: impl Write) -> Result<()> {
    let job: Job = serde_json::from_str(&std::fs::read_to_string(job_file)?)?;
    let mut open: Option<crate::pdf::Doc> = None;
    let mut docs: HashMap<PathBuf, crate::pdf::Doc> = HashMap::new();
    for line in input.lines() {
        match serde_json::from_str::<Task>(&line?)? {
            Task::Ink(t) => {
                if open.as_ref().is_none_or(|d| d.path != t.file) {
                    open = crate::pdf::Doc::open(&t.file).ok();
                }
                // een blad dat mislukt ontbreekt straks; de ouder probeert het zelf en meldt de fout
                let ink = open
                    .as_ref()
                    .and_then(|d| crate::pairing::ink_of(d, t.page, &job.cfg.pair).ok());
                if let Some(m) = ink {
                    let tmp = job.ink_dir.join(format!("{}.tmp", t.id));
                    std::fs::write(&tmp, m.to_bytes())?;
                    std::fs::rename(&tmp, job.ink_dir.join(format!("{}.ink", t.id)))?;
                }
                writeln!(out, "klaar")?;
            }
            Task::Scan { file, from, to } => {
                let r = crate::issue::scan_file_pages(&file, from..to, &job.cfg)
                    .map_err(|e| crate::errors::nl(&e));
                writeln!(out, "{}", serde_json::to_string(&r)?)?;
            }
            Task::SameVectors(f1, p1, f2, p2) => {
                let same = crate::issue::same_vectors(&f1, p1, &f2, p2);
                writeln!(out, "{same}")?;
            }
            Task::Layer {
                src,
                params,
                out: file,
            } => {
                if docs.len() > MAX_OPEN_DOCS {
                    docs.clear();
                }
                for f in src.files() {
                    if !docs.contains_key(f) {
                        docs.insert(f.clone(), crate::pdf::Doc::open(f)?);
                    }
                }
                let l = crate::layer::render_base(&docs, &src, &params)?;
                let tmp = file.with_extension("tmp");
                std::fs::write(&tmp, l.to_bytes())?;
                std::fs::rename(&tmp, &file)?;
                writeln!(out, "klaar")?;
            }
            Task::Pair(p) => {
                let (a, b) = *p;
                let d = match compare::compare_sheet(&a, &b, &job.cfg, None) {
                    Ok(mut d) => {
                        d.mask = None;
                        d
                    }
                    Err(e) => failed(&a, &b, &job.cfg, &crate::errors::nl(&e)),
                };
                writeln!(out, "{}", serde_json::to_string(&d)?)?;
            }
        }
        out.flush()?;
    }
    Ok(())
}

/// Uitkomst van een taak in de wachtrij.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// De antwoordregel van de werker.
    Line(String),
    /// Geen antwoord binnen de tijd; de werker is gestopt.
    TimedOut,
    /// De werker is weggevallen of niet te starten.
    Died(String),
}

/// Een lopend werkerproces.
pub struct Worker {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
}

impl Worker {
    pub fn start(exe: &Path, job: &Path) -> Result<Worker> {
        let mut child = Command::new(exe)
            .arg("worker")
            .arg(job)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("werker niet te starten: {}", exe.display()))?;
        let stdin = child.stdin.take().context("geen stdin van werker")?;
        let stdout = child.stdout.take().context("geen stdout van werker")?;
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Ok(Worker {
            child,
            stdin,
            lines,
        })
    }

    pub fn ask(&mut self, task: &str, timeout: Duration) -> Outcome {
        if let Err(e) = writeln!(self.stdin, "{task}").and_then(|_| self.stdin.flush()) {
            return Outcome::Died(format!("taak niet af te geven: {e}"));
        }
        match self.lines.recv_timeout(timeout) {
            Ok(l) => Outcome::Line(l),
            Err(RecvTimeoutError::Timeout) => Outcome::TimedOut,
            Err(RecvTimeoutError::Disconnected) => Outcome::Died("werkerproces gestopt".into()),
        }
    }

    pub fn stop(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    /// Stdin dicht; de werker stopt dan zelf.
    fn finish(self) {
        let Worker {
            mut child, stdin, ..
        } = self;
        drop(stdin);
        let _ = child.wait();
    }
}

/// Verdeelt `tasks` in volgorde over `workers` processen van `exe`. Elke taak krijgt precies een
/// uitkomst. Loopt een werker vast of valt hij weg, dan raakt dat alleen zijn eigen taak: de
/// volgende taak krijgt een nieuwe werker.
pub fn run_pool<T: Send>(
    exe: &Path,
    job: &Path,
    workers: usize,
    tasks: Vec<(String, T)>,
    timeout: Duration,
    mut on_done: impl FnMut(T, Outcome),
) {
    let queue = Mutex::new(tasks.into_iter().collect::<VecDeque<_>>());
    let (tx, rx) = mpsc::channel::<(T, Outcome)>();
    std::thread::scope(|scope| {
        for _ in 0..workers.max(1) {
            let (tx, queue) = (tx.clone(), &queue);
            scope.spawn(move || {
                let mut worker: Option<Worker> = None;
                loop {
                    let next = queue.lock().ok().and_then(|mut q| q.pop_front());
                    let Some((line, item)) = next else { break };
                    if worker.is_none() {
                        match Worker::start(exe, job) {
                            Ok(w) => worker = Some(w),
                            Err(e) => {
                                let _ = tx.send((item, Outcome::Died(crate::errors::nl(&e))));
                                continue;
                            }
                        }
                    }
                    let out = worker.as_mut().expect("net gestart").ask(&line, timeout);
                    if !matches!(out, Outcome::Line(_)) {
                        if let Some(w) = worker.take() {
                            w.stop();
                        }
                    }
                    let _ = tx.send((item, out));
                }
                if let Some(w) = worker {
                    w.finish();
                }
            });
        }
        drop(tx);
        for (item, out) in rx {
            on_done(item, out);
        }
    });
}

fn failed(a: &SheetRef, b: &SheetRef, cfg: &Config, why: &str) -> SheetDiff {
    SheetDiff {
        key: a.key.clone(),
        title: a.title.clone(),
        verdict: Verdict::NotComparable,
        reasons: vec![format!("fout: {why}")],
        a: Some(a.into()),
        b: Some(b.into()),
        registration: None,
        dpi: cfg.compare.dpi,
        removed_px: 0,
        added_px: 0,
        pixel_pct: 0.0,
        regions: vec![],
        text: vec![],
        seconds: 0.0,
        vector: None,
        link: None,
        pair_label: String::new(),
        mask: None,
    }
}

/// Aantal werkers: helft van de logische kernen, begrensd (geheugen: ~300 MB per werker op
/// 300 dpi A0).
pub fn default_workers() -> usize {
    let n = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    (n / 2).clamp(1, 8)
}

/// Tijdelijke map met het jobbestand voor de werkers.
fn job_dir(prefix: &str, job: &Job) -> Result<(PathBuf, PathBuf)> {
    static RUN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "sheetdiff-{prefix}-{}-{}",
        std::process::id(),
        RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir)?;
    let file = dir.join("job.json");
    std::fs::write(&file, serde_json::to_string(job)?)?;
    Ok((dir, file))
}

/// Ouderkant: verdeelt de paren over `workers` processen van `exe` (aangeroepen als
/// `exe worker <jobbestand>`), en roept `on_done` aan per afgerond blad.
pub fn compare_pairs(
    pairs: Vec<(SheetRef, SheetRef)>,
    cfg: &Config,
    exe: &Path,
    workers: usize,
    mut on_done: impl FnMut(SheetDiff),
) -> Result<()> {
    if pairs.is_empty() {
        return Ok(());
    }
    let workers = workers.clamp(1, pairs.len());
    // zwaarste bladen eerst (bestandsgrootte als maat), zodat er geen staart overblijft
    let mut pairs = pairs;
    pairs.sort_by_key(|(a, _)| {
        std::cmp::Reverse(std::fs::metadata(&a.file).map(|m| m.len()).unwrap_or(0))
    });
    let (dir, job) = job_dir(
        "jobs",
        &Job {
            cfg: cfg.clone(),
            ink_dir: PathBuf::new(),
        },
    )?;
    let mut tasks = Vec::with_capacity(pairs.len());
    for (a, b) in pairs {
        let line = serde_json::to_string(&Task::Pair(Box::new((a.clone(), b.clone()))))?;
        tasks.push((line, (a, b)));
    }
    let timeout = Duration::from_secs(cfg.compare.sheet_timeout_s);
    run_pool(exe, &job, workers, tasks, timeout, |(a, b), out| {
        let why = match out {
            Outcome::Line(l) => match serde_json::from_str::<SheetDiff>(&l) {
                Ok(d) => return on_done(d),
                Err(e) => format!("onleesbare werkeruitvoer: {e}"),
            },
            Outcome::TimedOut => format!(
                "vastgelopen: geen uitkomst binnen {} s; de rest van de uitgifte is wel vergeleken",
                cfg.compare.sheet_timeout_s
            ),
            Outcome::Died(e) => format!("{e} voor dit blad klaar was"),
        };
        on_done(failed(&a, &b, cfg, &why));
    });
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

/// Aantal werkers voor inktkaarten: meer dan bij vergelijken, want op 40 dpi kost een werker
/// weinig geheugen.
fn ink_workers() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, MAX_INK_WORKERS)
}

const MAX_INK_WORKERS: usize = 32;

/// Pagina's per scantaak: klein genoeg om een groot boekje te verdelen.
const SCAN_CHUNK: usize = 4;

/// Leest alle bestanden over werkerprocessen. Geeft per bestand de pagina's, op volgorde.
pub fn scan_files(
    files: &[PathBuf],
    cfg: &Config,
    exe: &Path,
) -> Result<Vec<(PathBuf, Vec<crate::issue::PageScan>)>> {
    let mut tasks = Vec::new();
    for (i, f) in files.iter().enumerate() {
        let n = crate::pdf::Doc::open(f)?.page_count();
        for from in (0..n).step_by(SCAN_CHUNK) {
            let to = (from + SCAN_CHUNK).min(n);
            let t = Task::Scan {
                file: f.clone(),
                from,
                to,
            };
            tasks.push((serde_json::to_string(&t)?, (i, f.clone(), from, to)));
        }
    }
    let (dir, job) = job_dir(
        "scan",
        &Job {
            cfg: cfg.clone(),
            ink_dir: PathBuf::new(),
        },
    )?;
    let workers = ink_workers().min(tasks.len().max(1));
    let timeout = Duration::from_secs(cfg.compare.sheet_timeout_s);
    let mut parts: Vec<Vec<(usize, Vec<crate::issue::PageScan>)>> = vec![Vec::new(); files.len()];
    let mut error = None;
    run_pool(
        exe,
        &job,
        workers,
        tasks,
        timeout,
        |(i, f, from, to), out| {
            let got = match out {
                Outcome::Line(l) => serde_json::from_str::<
                    std::result::Result<Vec<crate::issue::PageScan>, String>,
                >(&l)
                .map_err(|e| format!("onleesbare werkeruitvoer: {e}"))
                .and_then(|r| r),
                Outcome::TimedOut => Err(format!(
                    "vastgelopen: geen uitkomst binnen {} s",
                    cfg.compare.sheet_timeout_s
                )),
                Outcome::Died(e) => Err(e),
            };
            match got {
                Ok(p) => parts[i].push((from, p)),
                Err(e) => {
                    error.get_or_insert(format!("{} p.{}-{to}: {e}", f.display(), from + 1));
                }
            }
        },
    );
    let _ = std::fs::remove_dir_all(&dir);
    if let Some(e) = error {
        anyhow::bail!("scan mislukt: {e}");
    }
    Ok(files
        .iter()
        .cloned()
        .zip(parts)
        .map(|(f, mut p)| {
            p.sort_by_key(|x| x.0);
            (f, p.into_iter().flat_map(|x| x.1).collect())
        })
        .collect())
}

/// [`crate::issue::same_vectors`] voor een lijst paren, over werkerprocessen. Wat niet lukt, staat
/// er niet in en wordt door de aanroeper zelf bepaald; vastgelopen telt als 'niet gelijk'.
pub fn same_vectors_all(
    pairs: Vec<(PathBuf, usize, PathBuf, usize)>,
    cfg: &Config,
    exe: &Path,
) -> Result<HashMap<(PathBuf, usize, PathBuf, usize), bool>> {
    let mut out = HashMap::new();
    if pairs.is_empty() {
        return Ok(out);
    }
    let (dir, job) = job_dir(
        "vec",
        &Job {
            cfg: cfg.clone(),
            ink_dir: PathBuf::new(),
        },
    )?;
    let workers = ink_workers().min(pairs.len());
    let mut tasks = Vec::with_capacity(pairs.len());
    for p in pairs {
        let t = Task::SameVectors(p.0.clone(), p.1, p.2.clone(), p.3);
        tasks.push((serde_json::to_string(&t)?, p));
    }
    let timeout = Duration::from_secs(cfg.compare.sheet_timeout_s);
    run_pool(exe, &job, workers, tasks, timeout, |p, r| match r {
        Outcome::Line(l) => {
            out.insert(p, l.trim() == "true");
        }
        Outcome::TimedOut => {
            out.insert(p, false);
        }
        Outcome::Died(_) => {}
    });
    let _ = std::fs::remove_dir_all(&dir);
    Ok(out)
}

/// Maakt inktkaarten over werkerprocessen van `exe`. `Some` = kaart, `None` = blad vastgelopen, niet
/// opnieuw proberen. Wat ontbreekt (mislukt of werker weggevallen), moet de aanroeper zelf doen.
pub fn render_inks(
    tasks: Vec<InkTask>,
    cfg: &Config,
    exe: &Path,
) -> Result<HashMap<usize, Option<crate::pairing::InkMap>>> {
    if tasks.is_empty() {
        return Ok(HashMap::new());
    }
    let workers = ink_workers().min(tasks.len());
    let (dir, job) = job_dir(
        "ink",
        &Job {
            cfg: cfg.clone(),
            ink_dir: PathBuf::new(),
        },
    )?;
    std::fs::write(
        &job,
        serde_json::to_string(&Job {
            cfg: cfg.clone(),
            ink_dir: dir.clone(),
        })?,
    )?;
    // zwaarste bladen eerst: bestandsgrootte per blad als maat
    let mut per_file: HashMap<PathBuf, u64> = HashMap::new();
    for t in &tasks {
        *per_file.entry(t.file.clone()).or_default() += 1;
    }
    let weight = |t: &InkTask| {
        let size = std::fs::metadata(&t.file).map(|m| m.len()).unwrap_or(0);
        size / per_file[&t.file].max(1)
    };
    let mut tasks = tasks;
    tasks.sort_by_cached_key(|t| (std::cmp::Reverse(weight(t)), t.file.clone(), t.page));
    let mut lines = Vec::with_capacity(tasks.len());
    for t in tasks {
        let line = serde_json::to_string(&Task::Ink(t.clone()))?;
        lines.push((line, t));
    }
    let mut stuck = Vec::new();
    let mut done = Vec::new();
    let timeout = Duration::from_secs(cfg.compare.sheet_timeout_s);
    run_pool(exe, &job, workers, lines, timeout, |t, out| match out {
        Outcome::TimedOut => stuck.push(t.id),
        _ => done.push(t.id),
    });
    use rayon::prelude::*;
    let mut out: HashMap<usize, Option<crate::pairing::InkMap>> = done
        .into_par_iter()
        .filter_map(|id| {
            let s = std::fs::read(dir.join(format!("{id}.ink"))).ok()?;
            Some((id, Some(crate::pairing::InkMap::from_bytes(&s)?)))
        })
        .collect();
    out.extend(stuck.into_iter().map(|id| (id, None)));
    let _ = std::fs::remove_dir_all(&dir);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::{PageGeom, Rot90};

    fn sheet(k: &str) -> SheetRef {
        SheetRef {
            key: k.into(),
            number: Some(k.into()),
            title: String::new(),
            file: std::path::PathBuf::from("bestaat-niet.pdf"),
            page: 0,
            geom: PageGeom {
                crop: [0.0, 0.0, 100.0, 100.0],
                rotate: Rot90::R0,
            },
            method: crate::identify::IdMethod::None,
            text_runs: 0,
            booklets: vec![],
            signature: 0,
            tokens: Default::default(),
            ink: None,
            align: None,
            file_pages: 1,
        }
    }

    /// Een werker die niets oplevert (hier: cmd.exe, die "worker" niet kent) mag nooit tot
    /// ontbrekende of "gelijke" bladen leiden: elk blad komt terug als NIET VERGELIJKBAAR.
    #[cfg(windows)]
    #[test]
    fn dead_worker_reports_every_sheet_not_comparable() {
        let exe = std::path::PathBuf::from(std::env::var("COMSPEC").unwrap_or("cmd.exe".into()));
        let pairs = vec![
            (sheet("A-1"), sheet("A-1")),
            (sheet("A-2"), sheet("A-2")),
            (sheet("A-3"), sheet("A-3")),
        ];
        let mut got = Vec::new();
        compare_pairs(pairs, &Config::default(), &exe, 2, |d| got.push(d)).unwrap();
        assert_eq!(got.len(), 3);
        assert!(got.iter().all(|d| d.verdict == Verdict::NotComparable));
        let mut keys: Vec<_> = got.iter().map(|d| d.key.clone()).collect();
        keys.sort();
        assert_eq!(keys, vec!["A-1", "A-2", "A-3"]);
    }
}
