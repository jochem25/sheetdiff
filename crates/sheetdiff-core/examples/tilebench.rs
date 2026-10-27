//! Meet tegelrendertijden voor de viewer: tilebench <A.pdf> <pA> <B.pdf> <pB>
use sheetdiff_core::config::Config;
use sheetdiff_core::overlay::{OverlayColors, ViewMode};
use sheetdiff_core::pdf::{self, Doc};
use sheetdiff_core::register;
use sheetdiff_core::tiles::{Sheet, Style, TileService, TILE};
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let (fa, pa, fb, pb) = (
        PathBuf::from(&a[1]),
        a[2].parse::<usize>()? - 1,
        PathBuf::from(&a[3]),
        a[4].parse::<usize>()? - 1,
    );
    let cfg = Config::default();
    let reg = {
        let (da, db) = (Doc::open(&fa)?, Doc::open(&fb)?);
        let (x, y) = (da.page(pa)?, db.page(pb)?);
        register::register(
            &x,
            &pdf::page_geom(&x)?,
            &y,
            &pdf::page_geom(&y)?,
            &cfg.register,
        )?
    };
    eprintln!("registratie: {}", reg.note);
    let mut svc = TileService::start();
    svc.set_sheet(Sheet {
        a: Some((fa, pa)),
        b: Some((fb, pb)),
        b_to_a: reg.transform,
    });
    let style = Style {
        mode: ViewMode::Diff,
        params: cfg.compare.diff,
        colors: OverlayColors::default(),
        dim_same: 0.6,
        mix: 0.5,
    };
    for dpi in [36u32, 75, 150, 300, 600, 1200] {
        // een scherm van ~1920x1080 = 4x3 tegels rond het midden van een A0 (3370x2384 pt)
        let s = dpi as f64 / 72.0;
        let (cx, cy) = (
            (1685.0 * s) as i32 / TILE as i32,
            (1192.0 * s) as i32 / TILE as i32,
        );
        let t0 = std::time::Instant::now();
        let mut keys = std::collections::HashSet::new();
        for ty in cy - 1..cy + 2 {
            for tx in cx - 2..cx + 2 {
                keys.insert((tx.max(0), ty.max(0)));
            }
        }
        for &(tx, ty) in &keys {
            svc.request(dpi, tx, ty, &style);
        }
        let n = keys.len();
        let mut ms = Vec::new();
        for _ in 0..n {
            let t = svc.rx.recv()?;
            ms.push(t.ms);
        }
        ms.sort();
        println!(
            "{dpi:>5} dpi: {n} tegels in {:>5} ms (per tegel mediaan {} ms, max {} ms)",
            t0.elapsed().as_millis(),
            ms[ms.len() / 2],
            ms[ms.len() - 1]
        );
    }
    Ok(())
}
