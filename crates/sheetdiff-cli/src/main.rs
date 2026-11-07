//! sheetdiff-cli: headless vergelijken, voor scripts en agents.
//! Exitcodes: 0 geen verschillen, 1 verschillen, 2 niet vergelijkbaar / fout.

use anyhow::Result;
use clap::{Parser, Subcommand};
use sheetdiff_core::choices::Choices;
use sheetdiff_core::compare::{self, Progress};
use sheetdiff_core::register::ManualAlign;
use sheetdiff_core::{issue, pairing, report, Config};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "sheetdiff-cli",
    version,
    about = "Vergelijk tekeninguitgiftes (PDF) pixelprecies"
)]
struct Cli {
    /// TOML-profiel met instellingen
    #[arg(long, global = true)]
    profile: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

/// Handmatige keuzes uit de app. Standaard `<B> vs <A>.sheetdiff.json` bij uitgifte B.
#[derive(clap::Args)]
struct ChoicesArgs {
    /// Keuzebestand met handmatige koppelingen, ontkoppelingen en uitlijningen
    #[arg(long, visible_alias = "align-file")]
    choices: Option<PathBuf>,
    /// Bewaarde keuzes negeren
    #[arg(long, conflicts_with = "choices")]
    no_choices: bool,
}

impl ChoicesArgs {
    fn load(&self, a: &std::path::Path, b: &std::path::Path) -> Result<Choices> {
        if self.no_choices {
            return Ok(Choices::new(a, b));
        }
        let path = match &self.choices {
            Some(p) => {
                anyhow::ensure!(p.exists(), "keuzebestand bestaat niet: {}", p.display());
                p.clone()
            }
            None => Choices::default_path(a, b),
        };
        let (c, warn) = Choices::load(&path, a, b)?;
        if let Some(w) = warn {
            eprintln!("! {w}");
        }
        if !c.is_empty() {
            eprintln!(
                "keuzes: {} ({} koppelingen, {} ontkoppelingen, {} uitlijningen)",
                path.display(),
                c.links.len(),
                c.unlinks.len(),
                c.aligns.len()
            );
        }
        Ok(c)
    }
}

/// Handmatige uitlijning van twee losse pagina's.
#[derive(clap::Args)]
struct AlignArgs {
    /// rotatie,schaal,dx,dy (graden, factor, mm, mm); negatieve waarden met = (--align=-1,1,0,0)
    #[arg(long)]
    align: Option<String>,
    /// Keuzebestand; de uitlijning van dit bestand en deze pagina's geldt
    #[arg(long, conflicts_with = "align")]
    align_file: Option<PathBuf>,
}

impl AlignArgs {
    /// Pagina's 1-based.
    fn load(
        &self,
        a: &std::path::Path,
        page_a: usize,
        b: &std::path::Path,
        page_b: usize,
    ) -> Result<Option<ManualAlign>> {
        if let Some(s) = &self.align {
            return Ok(Some(ManualAlign::parse(s)?));
        }
        let Some(f) = &self.align_file else {
            return Ok(None);
        };
        anyhow::ensure!(f.exists(), "keuzebestand bestaat niet: {}", f.display());
        let (c, _) = Choices::load(f, a, b)?;
        let m = c.align_for(a, page_a, b, page_b);
        anyhow::ensure!(
            m.is_some(),
            "geen uitlijning voor deze pagina's in {}",
            f.display()
        );
        Ok(m)
    }
}

#[derive(Subcommand)]
enum Cmd {
    /// Inventariseer een uitgifte (map of PDF): bladen op tekeningnummer
    Scan {
        root: PathBuf,
        #[arg(long)]
        json: Option<PathBuf>,
    },
    /// Vergelijk uitgifte A met uitgifte B
    Compare {
        a: PathBuf,
        b: PathBuf,
        #[arg(long)]
        report: Option<PathBuf>,
        #[arg(long)]
        json: Option<PathBuf>,
        #[arg(long)]
        csv: Option<PathBuf>,
        #[arg(long)]
        dpi: Option<f64>,
        /// Alleen deze bladen (komma-gescheiden sleutels)
        #[arg(long)]
        only: Option<String>,
        #[arg(long)]
        xlsx: Option<PathBuf>,
        /// Map voor kopieen van B met revisiewolken
        #[arg(long)]
        clouds: Option<PathBuf>,
        /// Aantal werkerprocessen (0 = in dit proces)
        #[arg(long)]
        workers: Option<usize>,
        /// Pagina i van A tegen pagina i van B, namen genegeerd
        #[arg(long)]
        pair_by_order: bool,
        #[command(flatten)]
        choices: ChoicesArgs,
    },
    /// Toon hoe bladen gekoppeld worden, met alle inhoudsscores (kalibratie)
    Pairs {
        a: PathBuf,
        b: PathBuf,
        /// Alle scores als CSV: blad_a;blad_b;lijnen;tekst;score
        #[arg(long)]
        csv: Option<PathBuf>,
        /// Tekeningnummers negeren: alles op inhoud (blinde toets)
        #[arg(long)]
        no_names: bool,
        /// Eindkoppeling als lijst: sleutel, blad A, blad B
        #[arg(long)]
        list: Option<PathBuf>,
        #[arg(long)]
        workers: Option<usize>,
        #[command(flatten)]
        choices: ChoicesArgs,
    },
    /// Vergelijk twee losse pagina's (1-based paginanummers)
    Page {
        a: PathBuf,
        page_a: usize,
        b: PathBuf,
        page_b: usize,
        #[arg(long)]
        dpi: Option<f64>,
        #[command(flatten)]
        align: AlignArgs,
    },
    /// Overlay-PNG van twee pagina's (1-based), optioneel uitsnede in mm
    Overlay {
        a: PathBuf,
        page_a: usize,
        b: PathBuf,
        page_b: usize,
        #[arg(long, default_value_t = 100.0)]
        dpi: f64,
        /// x0,y0,x1,y1 in mm op blad A
        #[arg(long)]
        crop: Option<String>,
        #[arg(long, default_value = "overlay.png")]
        out: PathBuf,
        /// diff | a | b | blend
        #[arg(long, default_value = "diff")]
        mode: String,
        #[command(flatten)]
        align: AlignArgs,
    },
    /// Vectorvergelijking (spike) van twee pagina's (1-based)
    Vec {
        a: PathBuf,
        page_a: usize,
        b: PathBuf,
        page_b: usize,
    },
    /// Opmerkingen-PDF analyseren tegen uitgifte A en B
    Comments {
        c: PathBuf,
        a: PathBuf,
        b: PathBuf,
        #[arg(long)]
        json: Option<PathBuf>,
        /// Statusbestand (standaard naast de opmerkingen-PDF: <naam>.sheetdiff.json)
        #[arg(long)]
        state: Option<PathBuf>,
        #[arg(long)]
        xlsx: Option<PathBuf>,
        #[arg(long)]
        csv: Option<PathBuf>,
        /// Map voor PDF-kopieen van B met de opmerkingen en status erop
        #[arg(long)]
        pdf_dir: Option<PathBuf>,
        #[command(flatten)]
        choices: ChoicesArgs,
    },
    /// Intern: werkerproces voor parallel vergelijken
    #[command(hide = true)]
    Worker { job: PathBuf },
    /// Schrijf het standaardprofiel als TOML
    Profile,
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();
    match run() {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("FOUT: {}", sheetdiff_core::errors::nl(&e));
            std::process::exit(2);
        }
    }
}

/// Werkerprocessen van deze exe; 0 = in dit proces.
fn runner(exe: &std::path::Path, workers: Option<usize>) -> compare::Runner<'_> {
    match workers.unwrap_or_else(sheetdiff_core::parallel::default_workers) {
        0 => compare::Runner::InProcess,
        workers => compare::Runner::Processes { exe, workers },
    }
}

fn run() -> Result<i32> {
    let cli = Cli::parse();
    let mut cfg = match &cli.profile {
        Some(p) => Config::load(p)?,
        None => Config::default(),
    };
    match cli.cmd {
        Cmd::Overlay {
            a,
            page_a,
            b,
            page_b,
            dpi,
            crop,
            out,
            mode,
            align,
        } => {
            use sheetdiff_core::geom::{Affine, PT_PER_MM};
            use sheetdiff_core::overlay::{self, OverlayColors, ViewMode};
            use sheetdiff_core::pdf::{self, Doc};
            let da = Doc::open(&a)?;
            let db = Doc::open(&b)?;
            let pa = da.page(page_a - 1)?;
            let pb = db.page(page_b - 1)?;
            let (ga, gb) = (pdf::page_geom(&pa)?, pdf::page_geom(&pb)?);
            let reg = match align.load(&a, page_a, &b, page_b)? {
                Some(m) => {
                    let mut r = m.registration(gb.display_size());
                    r.measure(&pa, ga.display_size(), &pb, &cfg.register)?;
                    r
                }
                None => sheetdiff_core::register::register(&pa, &ga, &pb, &gb, &cfg.register)?,
            };
            eprintln!("uitlijning: {} (overlap {:.3})", reg.note, reg.score);
            let (w_pt, h_pt) = ga.display_size();
            let (x0, y0, x1, y1) = match crop {
                Some(c) => {
                    let v: Vec<f64> = c
                        .split(',')
                        .map(|s| s.trim().parse::<f64>())
                        .collect::<Result<_, _>>()?;
                    anyhow::ensure!(v.len() == 4, "--crop verwacht x0,y0,x1,y1");
                    (
                        v[0] * PT_PER_MM,
                        v[1] * PT_PER_MM,
                        v[2] * PT_PER_MM,
                        v[3] * PT_PER_MM,
                    )
                }
                None => (0.0, 0.0, w_pt, h_pt),
            };
            let s = dpi / 72.0;
            let (w, h) = (
                ((x1 - x0) * s).round() as usize,
                ((y1 - y0) * s).round() as usize,
            );
            let to_px = Affine::translate(-x0, -y0).then(&Affine::scale(s));
            let mode = match mode.as_str() {
                "a" => ViewMode::A,
                "b" => ViewMode::B,
                "blend" => ViewMode::Blend,
                _ => ViewMode::Diff,
            };
            let (rgba, rem, add) = overlay::render_tile(
                Some(&pa),
                Some(&pb),
                &reg.transform,
                &to_px,
                w,
                h,
                &cfg.compare.diff,
                mode,
                &OverlayColors::default(),
                0.6,
                0.5,
            )?;
            overlay::save_png(&out, rgba, w, h)?;
            eprintln!(
                "{}x{} px, verwijderd {rem}, toegevoegd {add} -> {}",
                w,
                h,
                out.display()
            );
            Ok(0)
        }
        Cmd::Vec {
            a,
            page_a,
            b,
            page_b,
        } => {
            use sheetdiff_core::pdf::{self, Doc};
            use sheetdiff_core::vector::{self, VChangeKind, VParams};
            let (da, db) = (Doc::open(&a)?, Doc::open(&b)?);
            let (pa, pb) = (da.page(page_a - 1)?, db.page(page_b - 1)?);
            let (ga, gb) = (pdf::page_geom(&pa)?, pdf::page_geom(&pb)?);
            let t = std::time::Instant::now();
            let (va, sa) = vector::extract(&pa, &ga)?;
            let (vb, sb) = vector::extract(&pb, &gb)?;
            eprintln!(
                "extractie {:.2} s
A {:?}
B {:?}",
                t.elapsed().as_secs_f64(),
                sa,
                sb
            );
            let reg = sheetdiff_core::register::register(&pa, &ga, &pb, &gb, &cfg.register)?;
            let t = std::time::Instant::now();
            let ch = vector::compare(&va, &vb, &reg.transform, &VParams::default());
            let n = |k| ch.iter().filter(|c| c.kind == k).count();
            eprintln!(
                "vergelijking {:.2} s: verwijderd {}, toegevoegd {}, stijl {}",
                t.elapsed().as_secs_f64(),
                n(VChangeKind::Removed),
                n(VChangeKind::Added),
                n(VChangeKind::Style)
            );
            for c in ch.iter() {
                println!(
                    "{:?} {:.0},{:.0}-{:.0},{:.0} mm segs {} {}",
                    c.kind,
                    compare::mm(c.bbox.x0),
                    compare::mm(c.bbox.y0),
                    compare::mm(c.bbox.x1),
                    compare::mm(c.bbox.y1),
                    c.segs.len(),
                    c.note
                );
            }
            Ok(0)
        }
        Cmd::Comments {
            c,
            a,
            b,
            json,
            state,
            xlsx,
            csv,
            pdf_dir,
            choices,
        } => {
            use sheetdiff_core::comments;
            let choices = choices.load(&a, &b)?;
            let exe = std::env::current_exe()?;
            let run = runner(&exe, None);
            let mut ia = issue::scan_issue_with(&a, &cfg, &run)?;
            let mut ib = issue::scan_issue_with(&b, &cfg, &run)?;
            pairing::pair_with_choices(&mut ia, &mut ib, &cfg, &run, &choices, false)?;
            let t = std::time::Instant::now();
            let r = comments::analyze(&c, &ia, &ib, &cfg)?;
            for (p, (nr, reg)) in &r.page_regs {
                eprintln!("opm-pagina {} -> {nr}: {}", p + 1, reg.note);
            }
            for w in &r.warnings {
                eprintln!("! {w}");
            }
            for c in &r.comments {
                println!(
                    "#{:<3} p.{:<3} {:<9} {:<24} {:<14?} {} | {}{}{}",
                    c.number,
                    c.c_page + 1,
                    c.sheet.as_deref().unwrap_or("-"),
                    c.proposal.label(),
                    c.kind,
                    c.text
                        .replace('\n', " ")
                        .chars()
                        .take(60)
                        .collect::<String>(),
                    c.reason,
                    if c.own_marks.is_empty() {
                        String::new()
                    } else {
                        format!(" | eigen: {}", c.own_marks.join(", "))
                    },
                    if c.text_changes.is_empty() {
                        String::new()
                    } else {
                        format!(" | tekst: {}", c.text_changes.join("; "))
                    }
                );
            }
            eprintln!(
                "{} opmerkingen in {:.1} s",
                r.comments.len(),
                t.elapsed().as_secs_f64()
            );
            if let Some(p) = json {
                std::fs::write(p, serde_json::to_string_pretty(&r)?)?;
            }
            let sp = state.unwrap_or_else(|| comments::ReviewState::default_path(&c));
            let (st, warn) = comments::ReviewState::load_checked(&sp, &a, &b, &c)?;
            if let Some(w) = warn {
                eprintln!("! {w}");
            }
            eprintln!("status: {} ({} ingevuld)", sp.display(), st.entries.len());
            if let Some(p) = xlsx {
                sheetdiff_core::export::comments_xlsx(&r, &st, &p)?;
                eprintln!("xlsx: {}", p.display());
            }
            if let Some(p) = csv {
                std::fs::write(&p, sheetdiff_core::export::comments_csv(&r, &st))?;
                eprintln!("csv: {}", p.display());
            }
            if let Some(d) = pdf_dir {
                let missing = sheetdiff_core::export::comments_without_place(&r);
                if !missing.is_empty() {
                    eprintln!("! opmerkingen zonder plek op B (niet in de PDF, wel in xlsx/csv): {missing:?}");
                }
                for f in sheetdiff_core::export::comments_on_b(&r, &st, &ib, &d)? {
                    eprintln!("pdf: {}", f.display());
                }
            }
            Ok(0)
        }
        Cmd::Worker { job } => {
            sheetdiff_core::parallel::run_job(
                &job,
                std::io::stdin().lock(),
                std::io::stdout().lock(),
            )?;
            Ok(0)
        }
        Cmd::Pairs {
            a,
            b,
            csv,
            no_names,
            list,
            workers,
            choices,
        } => {
            if no_names {
                cfg.pair.use_names = false;
            }
            let choices = choices.load(&a, &b)?;
            let exe = std::env::current_exe()?;
            let runner = runner(&exe, workers);
            let t0 = std::time::Instant::now();
            let (mut ia, mut ib) = (
                issue::scan_issue_with(&a, &cfg, &runner)?,
                issue::scan_issue_with(&b, &cfg, &runner)?,
            );
            let t_scan = t0.elapsed().as_secs_f64();
            let t1 = std::time::Instant::now();
            let m = pairing::score_matrix(&mut ia, &mut ib, &cfg, &runner)?;
            let t_inks = t1.elapsed().as_secs_f64();
            let t2 = std::time::Instant::now();
            pairing::pair_with_choices(&mut ia, &mut ib, &cfg, &runner, &choices, false)?;
            eprintln!(
                "{} x {} bladen: scan {t_scan:.1} s, inktkaarten + scores {t_inks:.1} s, scores + toewijzing {:.2} s",
                m.keys_a.len(),
                m.keys_b.len(),
                t2.elapsed().as_secs_f64()
            );
            if let Some(p) = csv {
                let f = |v: Option<f64>| v.map(|x| format!("{x:.4}")).unwrap_or_default();
                let mut s = String::from("blad_a;blad_b;lijnen;tekst;score\n");
                for (i, ka) in m.keys_a.iter().enumerate() {
                    for (j, kb) in m.keys_b.iter().enumerate() {
                        let c = &m.scores[i][j];
                        s.push_str(&format!(
                            "{ka};{kb};{};{};{:.4}\n",
                            f(c.lines),
                            f(c.text),
                            c.score
                        ));
                    }
                }
                std::fs::write(p, s)?;
            }
            for l in &ia.links {
                println!("{}\t{}\t{}", l.label(), l.key_a, l.key_b);
            }
            for (k, n) in ia.notes.iter().chain(ib.notes.iter()) {
                println!("los\t{k}\t{n}");
            }
            if let Some(p) = list {
                let loc = |s: &issue::SheetRef| {
                    let stem = s.file.file_stem().unwrap_or_default().to_string_lossy();
                    format!("{stem}#{}", s.page + 1)
                };
                let mut s = String::new();
                for (k, sa) in &ia.sheets {
                    if let Some(sb) = ib.sheets.get(k) {
                        s.push_str(&format!("{k}\t{}\t{}\n", loc(sa), loc(sb)));
                    }
                }
                std::fs::write(p, s)?;
            }
            let same = ia
                .sheets
                .keys()
                .filter(|k| ib.sheets.contains_key(*k))
                .count();
            println!(
                "{same} paren, {} alleen in A, {} alleen in B",
                ia.sheets.len() - same,
                ib.sheets.len() - same
            );
            Ok(0)
        }
        Cmd::Profile => {
            println!("{}", cfg.to_toml());
            Ok(0)
        }
        Cmd::Scan { root, json } => {
            let iss = issue::scan_issue(&root, &cfg)?;
            for (k, s) in &iss.sheets {
                println!(
                    "{k:<14} {:<10?} p.{:<3} {:<45} {}",
                    s.method,
                    s.page + 1,
                    s.title,
                    s.booklets.join(",")
                );
            }
            println!(
                "{} bladen, {} tekeninglijst-pagina's, {} waarschuwingen",
                iss.sheets.len(),
                iss.list_pages.len(),
                iss.warnings.len()
            );
            for w in &iss.warnings {
                println!("  ! {w}");
            }
            if let Some(p) = json {
                std::fs::write(p, serde_json::to_string_pretty(&iss)?)?;
            }
            Ok(0)
        }
        Cmd::Page {
            a,
            page_a,
            b,
            page_b,
            dpi,
            align,
        } => {
            if let Some(d) = dpi {
                cfg.compare.dpi = d;
            }
            let m = align.load(&a, page_a, &b, page_b)?;
            let d = compare::compare_pages(&a, page_a - 1, &b, page_b - 1, &cfg, m)?;
            let mut s = String::new();
            report::sheet_markdown(&d, &mut s);
            print!("{s}");
            if let Some(r) = &d.registration {
                println!(
                    "uitlijning: {} (overlap {:.3}; A in B {:.3}, B in A {:.3})",
                    r.note, r.score, r.a_in_b, r.b_in_a
                );
            }
            for r in &d.regions {
                println!(
                    "  regio {:.0},{:.0} - {:.0},{:.0} mm  weg {} erbij {}{}",
                    compare::mm(r.bbox.x0),
                    compare::mm(r.bbox.y0),
                    compare::mm(r.bbox.x1),
                    compare::mm(r.bbox.y1),
                    r.removed_px,
                    r.added_px,
                    if r.stamp { "  [stempel]" } else { "" }
                );
            }
            println!("tijd {:.1} s", d.seconds);
            Ok(match d.verdict {
                compare::Verdict::Identical | compare::Verdict::StampOnly => 0,
                compare::Verdict::NotComparable => 2,
                _ => 1,
            })
        }
        Cmd::Compare {
            a,
            b,
            report: rep,
            json,
            csv,
            dpi,
            only,
            xlsx,
            clouds,
            workers,
            pair_by_order,
            choices,
        } => {
            if let Some(d) = dpi {
                cfg.compare.dpi = d;
            }
            let choices = choices.load(&a, &b)?;
            let exe = std::env::current_exe()?;
            let nw = workers.unwrap_or_else(sheetdiff_core::parallel::default_workers);
            let runner = runner(&exe, Some(nw));
            let (mut ia, mut ib) = if pair_by_order {
                let (mut ia, mut ib) = (
                    issue::scan_pages_with(&a, &cfg, &runner)?,
                    issue::scan_pages_with(&b, &cfg, &runner)?,
                );
                pairing::pair_with_choices(&mut ia, &mut ib, &cfg, &runner, &choices, true)?;
                (ia, ib)
            } else {
                let (mut ia, mut ib) = (
                    issue::scan_issue_with(&a, &cfg, &runner)?,
                    issue::scan_issue_with(&b, &cfg, &runner)?,
                );
                let t = std::time::Instant::now();
                let r =
                    pairing::pair_with_choices(&mut ia, &mut ib, &cfg, &runner, &choices, false)?;
                eprintln!(
                    "{} bladen op inhoud gekoppeld in {:.1} s",
                    ia.links.len() - r.links,
                    t.elapsed().as_secs_f64()
                );
                (ia, ib)
            };
            if let Some(o) = only {
                let keep: Vec<&str> = o.split(',').map(|s| s.trim()).collect();
                ia.sheets.retain(|k, _| keep.contains(&k.as_str()));
                ib.sheets.retain(|k, _| keep.contains(&k.as_str()));
            }
            let t0 = std::time::Instant::now();
            let d = compare::compare_issues_with(&ia, &ib, &cfg, runner, |p| match p {
                Progress::Start { total } => eprintln!("{total} bladen vergelijken..."),
                Progress::Sheet { done, total, diff } => eprintln!(
                    "[{done}/{total}] {:<14} {:<20} {:.1} s",
                    diff.key,
                    diff.verdict.label(),
                    diff.seconds
                ),
            })?;
            eprintln!(
                "klaar in {:.1} s ({} werkers)",
                t0.elapsed().as_secs_f64(),
                nw
            );
            report::write_all(&d, rep.as_deref(), json.as_deref(), csv.as_deref())?;
            if let Some(p) = xlsx {
                sheetdiff_core::export::diff_xlsx(&d, &p)?;
            }
            if let Some(dir) = clouds {
                for f in sheetdiff_core::export::clouds_on_b(&d, &ib, &dir)? {
                    eprintln!("wolken: {}", f.display());
                }
            }
            if rep.is_none() {
                print!("{}", report::markdown(&d));
            }
            Ok(d.exit_code())
        }
    }
}
