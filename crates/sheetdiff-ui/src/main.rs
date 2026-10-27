//! sheetdiff: desktopapp voor het vergelijken van tekeninguitgiftes.
//! `sheetdiff worker <job>` is de interne werkermodus voor parallel vergelijken.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod render;
mod theme;
mod viewer;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(|s| s.as_str()) == Some("worker") {
        let code = match args.get(2) {
            Some(job) => match sheetdiff_core::parallel::run_job(
                std::path::Path::new(job),
                std::io::stdout().lock(),
            ) {
                Ok(()) => 0,
                Err(_) => 2,
            },
            None => 2,
        };
        std::process::exit(code);
    }
    let icon = image::load_from_memory(include_bytes!("../assets/icon-256.png"))
        .map(|i| i.to_rgba8())
        .ok()
        .map(|i| eframe::egui::IconData {
            width: i.width(),
            height: i.height(),
            rgba: i.into_raw(),
        });
    let mut vp = eframe::egui::ViewportBuilder::default()
        .with_title("sheetdiff - Kolthof Bouwadvies")
        .with_inner_size([1500.0, 900.0])
        .with_min_inner_size([1000.0, 640.0]);
    if let Some(i) = icon {
        vp = vp.with_icon(i);
    }
    let opts = eframe::NativeOptions {
        viewport: vp,
        ..Default::default()
    };
    eframe::run_native(
        "sheetdiff",
        opts,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
