//! sheetdiff-cli: headless vergelijken, voor scripts en agents.
//! Exitcodes: 0 geen verschillen, 1 verschillen, 2 niet vergelijkbaar / fout.

use anyhow::Result;
use clap::{Parser, Subcommand};
use sheetdiff_core::compare::{self, Progress};
use sheetdiff_core::{issue, report, Config};
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
    },
    /// Vergelijk twee losse pagina's (1-based paginanummers)
    Page {
        a: PathBuf,
        page_a: usize,
        b: PathBuf,
        page_b: usize,
        #[arg(long)]
        dpi: Option<f64>,
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
            eprintln!("FOUT: {e:#}");
            std::process::exit(2);
        }
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
        } => {
            use sheetdiff_core::geom::{Affine, PT_PER_MM};
            use sheetdiff_core::overlay::{self, OverlayColors, ViewMode};
            use sheetdiff_core::pdf::{self, Doc};
            let da = Doc::open(&a)?;
            let db = Doc::open(&b)?;
            let pa = da.page(page_a - 1)?;
            let pb = db.page(page_b - 1)?;
            let (ga, gb) = (pdf::page_geom(&pa)?, pdf::page_geom(&pb)?);
            let reg = sheetdiff_core::register::register(&pa, &ga, &pb, &gb, &cfg.register)?;
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
        } => {
            use sheetdiff_core::comments;
            let ia = issue::scan_issue(&a, &cfg)?;
            let ib = issue::scan_issue(&b, &cfg)?;
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
            sheetdiff_core::parallel::run_job(&job, std::io::stdout().lock())?;
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
        } => {
            if let Some(d) = dpi {
                cfg.compare.dpi = d;
            }
            let d = compare::compare_pages(&a, page_a - 1, &b, page_b - 1, &cfg)?;
            let mut s = String::new();
            report::sheet_markdown(&d, &mut s);
            print!("{s}");
            if let Some(r) = &d.registration {
                println!("uitlijning: {} (overlap {:.3})", r.note, r.score);
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
        } => {
            if let Some(d) = dpi {
                cfg.compare.dpi = d;
            }
            let mut ia = issue::scan_issue(&a, &cfg)?;
            let mut ib = issue::scan_issue(&b, &cfg)?;
            if let Some(o) = only {
                let keep: Vec<&str> = o.split(',').map(|s| s.trim()).collect();
                ia.sheets.retain(|k, _| keep.contains(&k.as_str()));
                ib.sheets.retain(|k, _| keep.contains(&k.as_str()));
            }
            let exe = std::env::current_exe()?;
            let nw = workers.unwrap_or_else(sheetdiff_core::parallel::default_workers);
            let runner = if nw == 0 {
                compare::Runner::InProcess
            } else {
                compare::Runner::Processes {
                    exe: &exe,
                    workers: nw,
                }
            };
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
