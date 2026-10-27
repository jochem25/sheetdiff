//! Parallel vergelijken over processen. PDFium is per proces geserialiseerd (een globale
//! mutex), dus echte parallelliteit vraagt meerdere processen. Elke werker krijgt een
//! jobbestand met bladparen en schrijft per afgerond blad een JSON-regel naar stdout.

use crate::compare::{self, SheetDiff, Verdict};
use crate::config::Config;
use crate::issue::SheetRef;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};

/// Zoveel seconden zonder afgerond blad = werker vastgelopen.
pub const SHEET_TIMEOUT_S: u64 = 600;

fn now_s() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Serialize, Deserialize)]
pub struct Job {
    pub cfg: Config,
    pub pairs: Vec<(SheetRef, SheetRef)>,
}

/// Werkerkant: voert een job uit en schrijft een JSON-regel per blad naar `out`.
pub fn run_job(job_file: &Path, mut out: impl std::io::Write) -> Result<()> {
    let job: Job = serde_json::from_str(&std::fs::read_to_string(job_file)?)?;
    for (a, b) in &job.pairs {
        let d = match compare::compare_sheet(a, b, &job.cfg, None) {
            Ok(mut d) => {
                d.mask = None;
                d
            }
            Err(e) => failed(a, b, &job.cfg, &format!("{e:#}")),
        };
        writeln!(out, "{}", serde_json::to_string(&d)?)?;
        out.flush()?;
    }
    Ok(())
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
    // zwaarste bladen eerst verdelen (grootte als maat), dan round-robin
    let mut pairs = pairs;
    pairs.sort_by_key(|(a, _)| {
        std::cmp::Reverse(std::fs::metadata(&a.file).map(|m| m.len()).unwrap_or(0))
    });
    let mut buckets: Vec<Vec<(SheetRef, SheetRef)>> = vec![Vec::new(); workers];
    for (i, p) in pairs.into_iter().enumerate() {
        buckets[i % workers].push(p);
    }
    static RUN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "sheetdiff-jobs-{}-{}",
        std::process::id(),
        RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir)?;
    let (tx, rx) = std::sync::mpsc::channel::<Result<SheetDiff, String>>();
    let mut children = Vec::new();
    for (i, b) in buckets.into_iter().enumerate() {
        let expected: Vec<(SheetRef, SheetRef)> = b.clone();
        let jf = dir.join(format!("job{i}.json"));
        std::fs::write(
            &jf,
            serde_json::to_string(&Job {
                cfg: cfg.clone(),
                pairs: b,
            })?,
        )?;
        let mut child = Command::new(exe)
            .arg("worker")
            .arg(&jf)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("werker niet te starten: {}", exe.display()))?;
        let stdout = child.stdout.take().context("geen stdout van werker")?;
        let tx = tx.clone();
        let cfg = cfg.clone();
        // waakhond: komt er te lang geen blad af (PDFium hangt), dan het proces stoppen; de
        // leesthread ziet EOF en meldt de resterende bladen als niet vergelijkbaar
        let child = std::sync::Arc::new(std::sync::Mutex::new(child));
        let last = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(now_s()));
        let done_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        {
            let (child, last, done_flag) = (child.clone(), last.clone(), done_flag.clone());
            std::thread::spawn(move || {
                use std::sync::atomic::Ordering::Relaxed;
                while !done_flag.load(Relaxed) {
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    if now_s().saturating_sub(last.load(Relaxed)) > SHEET_TIMEOUT_S {
                        if let Ok(mut c) = child.lock() {
                            let _ = c.kill();
                        }
                        break;
                    }
                }
            });
        }
        let reader = {
            let (last, done_flag) = (last.clone(), done_flag.clone());
            std::thread::spawn(move || {
                use std::sync::atomic::Ordering::Relaxed;
                let mut seen: std::collections::HashSet<String> = Default::default();
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    last.store(now_s(), Relaxed);
                    match serde_json::from_str::<SheetDiff>(&line) {
                        Ok(d) => {
                            seen.insert(d.key.clone());
                            let _ = tx.send(Ok(d));
                        }
                        Err(e) => {
                            let _ = tx.send(Err(format!("onleesbare werkeruitvoer: {e}")));
                        }
                    }
                }
                done_flag.store(true, Relaxed);
                // werker voortijdig gestopt: resterende bladen luid als niet vergelijkbaar
                for (a, b) in expected.iter().filter(|(a, _)| !seen.contains(&a.key)) {
                    let _ = tx.send(Ok(failed(
                        a,
                        b,
                        &cfg,
                        "werkerproces gestopt (vastgelopen of gecrasht) voor dit blad klaar was",
                    )));
                }
            })
        };
        children.push((child, reader));
    }
    drop(tx);
    for msg in rx {
        match msg {
            Ok(d) => on_done(d),
            Err(e) => tracing::warn!("{e}"),
        }
    }
    for (c, h) in children {
        let _ = h.join();
        if let Ok(mut c) = c.lock() {
            let _ = c.wait();
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
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
