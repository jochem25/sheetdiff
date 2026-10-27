//! De app: vijf schermen zoals in de goedgekeurde mockup (start, bladweergave, verschillen,
//! opmerkingen, export), linkerpaneel met bladen, rechterpaneel met context, statusbalk.

use crate::render::{RenderService, Resp, SheetSrc, ZoneReq};
use crate::theme::{self, chip};
use crate::viewer::{self, CommentMark, LayerTex, Mode, Viewer};
use eframe::egui::{self, Color32, RichText, Vec2};
use sheetdiff_core::comments::{self, CommentsResult, ReviewState, Status};
use sheetdiff_core::compare::{self, IssueDiff, Progress, SheetDiff, Verdict};
use sheetdiff_core::geom::{Affine, RectPt, MM_PER_PT, PT_PER_MM};
use sheetdiff_core::issue::{self, Issue};
use sheetdiff_core::textdiff::TextChangeKind;
use sheetdiff_core::{export, report, Config};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Start,
    View,
    Diffs,
    Comments,
    Export,
}

enum JobMsg {
    Log(String),
    Progress(usize, usize),
    Scanned(Box<Issue>, Box<Issue>),
    Done(Box<IssueDiff>),
    Comments(Box<CommentsResult>),
    Error(String),
    Finished,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Saved {
    a: String,
    b: String,
    c: String,
    cfg: Option<Config>,
    workers: usize,
}

pub struct App {
    screen: Screen,
    path_a: String,
    path_b: String,
    path_c: String,
    cfg: Config,
    workers: usize,
    job_rx: Option<Receiver<JobMsg>>,
    progress: (usize, usize),
    log: Vec<String>,
    issue_a: Option<Issue>,
    issue_b: Option<Issue>,
    diff: Option<IssueDiff>,
    comments: Option<CommentsResult>,
    review: ReviewState,
    review_path: Option<PathBuf>,
    sel_sheet: Option<usize>,
    sel_region: Option<usize>,
    sel_comment: Option<usize>,
    filter: Option<Verdict>,
    viewer: Viewer,
    render: RenderService,
    render_gen: u64,
    zone: Option<(usize, LayerTex, Option<egui::TextureHandle>)>,
    hover_mm: Option<(f64, f64)>,
    logo: egui::TextureHandle,
    message: Option<(String, bool)>,
    auto: Option<Auto>,
    align: Option<AlignEdit>,
    sheet_job: Option<Receiver<(String, Result<SheetDiff, String>)>>,
    zone_error: Option<String>,
    status_dirty: Option<f64>,
    last_params: Option<sheetdiff_core::raster::DiffParams>,
}

/// Handmatige uitlijning van B op A voor het geopende blad.
#[derive(Clone, Copy, PartialEq)]
struct AlignEdit {
    rot: sheetdiff_core::geom::Rot90,
    scale: f64,
    dx: f64,
    dy: f64,
}

/// Automatische screenshotmodus (SHEETDIFF_AUTOSHOT=<map>): loopt door de schermen en
/// schrijft per stap een PNG. Voor mijlpaalverslagen en regressiecontrole.
struct Auto {
    dir: PathBuf,
    step: usize,
    wait_until: f64,
    shot_pending: Option<String>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        let saved: Saved = cc
            .storage
            .and_then(|s| eframe::get_value(s, "sheetdiff"))
            .unwrap_or_default();
        let logo = {
            let img = image::load_from_memory(include_bytes!("../assets/beeldmerk-96.png"))
                .expect("logo")
                .to_rgba8();
            cc.egui_ctx.load_texture(
                "logo",
                egui::ColorImage::from_rgba_unmultiplied(
                    [img.width() as usize, img.height() as usize],
                    img.as_raw(),
                ),
                Default::default(),
            )
        };
        Self {
            screen: Screen::Start,
            path_a: saved.a,
            path_b: saved.b,
            path_c: saved.c,
            cfg: saved.cfg.unwrap_or_default(),
            workers: if saved.workers == 0 {
                sheetdiff_core::parallel::default_workers()
            } else {
                saved.workers
            },
            job_rx: None,
            progress: (0, 0),
            log: Vec::new(),
            issue_a: None,
            issue_b: None,
            diff: None,
            comments: None,
            review: ReviewState::default(),
            review_path: None,
            sel_sheet: None,
            sel_region: None,
            sel_comment: None,
            filter: None,
            viewer: Viewer::default(),
            render: RenderService::start(cc.egui_ctx.clone()),
            render_gen: 0,
            zone: None,
            hover_mm: None,
            logo,
            message: None,
            align: None,
            sheet_job: None,
            zone_error: None,
            status_dirty: None,
            last_params: None,
            auto: std::env::var("SHEETDIFF_AUTOSHOT").ok().map(|d| Auto {
                dir: PathBuf::from(d),
                step: 0,
                wait_until: 0.0,
                shot_pending: None,
            }),
        }
        .with_env_paths()
    }

    fn with_env_paths(mut self) -> Self {
        for (k, slot) in [
            ("SHEETDIFF_A", &mut self.path_a),
            ("SHEETDIFF_B", &mut self.path_b),
            ("SHEETDIFF_C", &mut self.path_c),
        ] {
            if let Ok(v) = std::env::var(k) {
                *slot = v;
            }
        }
        self
    }

    fn auto_step(&mut self, ctx: &egui::Context) {
        let Some(mut a) = self.auto.take() else {
            return;
        };
        let now = ctx.input(|i| i.time);
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
        // screenshot ontvangen?
        let shot = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let (Some(img), Some(name)) = (shot, a.shot_pending.clone()) {
            let _ = std::fs::create_dir_all(&a.dir);
            let raw: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
            if let Some(buf) =
                image::RgbaImage::from_raw(img.size[0] as u32, img.size[1] as u32, raw)
            {
                let _ = buf.save(a.dir.join(format!("{name}.png")));
            }
            a.shot_pending = None;
            a.step += 1;
            a.wait_until = now + 0.3;
        }
        if a.shot_pending.is_some() || now < a.wait_until {
            self.auto = Some(a);
            return;
        }
        let shoot = |a: &mut Auto, name: &str| {
            a.shot_pending = Some(name.to_string());
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        };
        let busy = self.job_rx.is_some();
        match a.step {
            0 => {
                self.start_compare(ctx);
                a.step = 1;
                a.wait_until = now + 1.0;
            }
            1 if !busy => shoot(&mut a, "01_start_bladenlijst"),
            2 => {
                let want = std::env::var("SHEETDIFF_SHEET").ok();
                let idx = self
                    .diff
                    .as_ref()
                    .and_then(|d| match &want {
                        Some(k) => d.sheets.iter().position(|s| &s.key == k),
                        None => None,
                    })
                    .or_else(|| self.first_changed());
                if let Some(i) = idx {
                    self.open_sheet(i);
                }
                self.screen = Screen::View;
                self.viewer.mode = Mode::Overlay;
                a.step = 3;
                a.wait_until = now + 1.0;
            }
            3 if self.viewer.base.is_some() => shoot(&mut a, "02_overlay"),
            4 => {
                self.viewer.mode = Mode::Pixel;
                a.step = 5;
                a.wait_until = now + 0.5;
            }
            5 => shoot(&mut a, "03_pixelverschil"),
            6 => {
                self.viewer.mode = Mode::Side;
                self.viewer.fit_pending = true;
                a.step = 7;
                a.wait_until = now + 0.6;
            }
            7 => shoot(&mut a, "04_naast_elkaar"),
            8 => {
                self.viewer.mode = Mode::Vector;
                self.viewer.fit_pending = true;
                a.step = 9;
                a.wait_until = now + 0.6;
            }
            9 if self.viewer.vector.is_some() => shoot(&mut a, "05_vectorverschil"),
            10 => {
                // eerste regio met tekst, ingezoomd tot 1:1-pixel op 300 dpi
                let target = self.cur().and_then(|s| {
                    s.regions
                        .iter()
                        .position(|r| !r.stamp && !r.text.is_empty())
                        .or_else(|| s.regions.iter().position(|r| !r.stamp))
                });
                self.viewer.mode = Mode::Pixel;
                if let Some(i) = target {
                    self.goto_region(i);
                    let ppp = ctx.pixels_per_point();
                    let (cx, cy) = self
                        .cur()
                        .map(|s| s.regions[i].bbox.center())
                        .unwrap_or((0.0, 0.0));
                    self.viewer.zoom = (300.0 / 72.0) / ppp;
                    let pane = self.viewer.panel_rect.size();
                    self.viewer.pan = Vec2::new(
                        cx as f32 - pane.x / 2.0 / self.viewer.zoom,
                        cy as f32 - pane.y / 2.0 / self.viewer.zoom,
                    );
                    self.viewer.last_change = now;
                }
                a.step = 11;
                a.wait_until = now + 2.5;
            }
            11 if self.viewer.detail.is_some() => shoot(&mut a, "06_pixelzoom_1op1"),
            12 => {
                self.viewer.mode = Mode::Blink;
                self.viewer.blink_auto = false;
                self.viewer.blink_b = true;
                a.step = 13;
                a.wait_until = now + 0.4;
            }
            13 => shoot(&mut a, "07_knipperen_B"),
            14 => {
                self.screen = Screen::Diffs;
                a.step = 15;
                a.wait_until = now + 0.4;
            }
            15 => shoot(&mut a, "08_verschillenlijst"),
            16 => {
                self.screen = Screen::Comments;
                let pick = self.comments.as_ref().and_then(|r| {
                    r.comments
                        .iter()
                        .position(|c| c.zone_a.is_some() && !c.own_marks.is_empty())
                        .or_else(|| r.comments.iter().position(|c| c.zone_a.is_some()))
                });
                if let Some(i) = pick {
                    self.select_comment(i);
                }
                a.step = 17;
                a.wait_until = now + 1.0;
            }
            17 if self.comments.is_none() || self.zone.is_some() => shoot(&mut a, "09_opmerkingen"),
            18 => {
                self.screen = Screen::Export;
                a.step = 19;
                a.wait_until = now + 0.4;
            }
            19 => shoot(&mut a, "10_export"),
            20 => {
                self.screen = Screen::View;
                self.viewer.mode = Mode::Overlay;
                self.viewer.fit_pending = true;
                if let Some(r) = self.cur().and_then(|s| s.registration.clone()) {
                    self.align = Some(AlignEdit {
                        rot: r.rotation,
                        scale: r.scale,
                        dx: r.dx_mm,
                        dy: r.dy_mm,
                    });
                }
                a.step = 21;
                a.wait_until = now + 1.0;
            }
            21 => shoot(&mut a, "11_uitlijning"),
            22 => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                a.step = 23;
            }
            _ => {}
        }
        self.auto = Some(a);
    }

    // ------------------------------------------------------------------ jobs

    fn start_compare(&mut self, ctx: &egui::Context) {
        let (tx, rx) = std::sync::mpsc::channel::<JobMsg>();
        self.job_rx = Some(rx);
        self.diff = None;
        self.comments = None;
        self.sel_sheet = None;
        self.progress = (0, 0);
        self.log.clear();
        self.sheet_job = None; // oud herberekenresultaat hoort niet bij de nieuwe vergelijking
        self.align = None;
        let (a, b, c) = (
            PathBuf::from(self.path_a.trim().trim_matches('"')),
            PathBuf::from(self.path_b.trim().trim_matches('"')),
            self.path_c.trim().trim_matches('"').to_string(),
        );
        let cfg = self.cfg.clone();
        let workers = self.workers;
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let send = |m: JobMsg| {
                let _ = tx.send(m);
                ctx.request_repaint();
            };
            let run = || -> anyhow::Result<()> {
                send(JobMsg::Log(format!("uitgifte A lezen: {}", a.display())));
                let ia = issue::scan_issue(&a, &cfg)?;
                send(JobMsg::Log(format!("uitgifte B lezen: {}", b.display())));
                let ib = issue::scan_issue(&b, &cfg)?;
                send(JobMsg::Scanned(Box::new(ia.clone()), Box::new(ib.clone())));
                let exe = std::env::current_exe()?;
                let runner = if workers == 0 {
                    compare::Runner::InProcess
                } else {
                    compare::Runner::Processes { exe: &exe, workers }
                };
                let d = compare::compare_issues_with(&ia, &ib, &cfg, runner, |p| match p {
                    Progress::Start { total } => send(JobMsg::Progress(0, total)),
                    Progress::Sheet { done, total, .. } => send(JobMsg::Progress(done, total)),
                })?;
                send(JobMsg::Done(Box::new(d)));
                if !c.is_empty() {
                    send(JobMsg::Log("opmerkingen analyseren...".into()));
                    let r = comments::analyze(std::path::Path::new(&c), &ia, &ib, &cfg)?;
                    send(JobMsg::Comments(Box::new(r)));
                }
                Ok(())
            };
            if let Err(e) = run() {
                send(JobMsg::Error(format!("{e:#}")));
            }
            send(JobMsg::Finished);
        });
    }

    fn poll_jobs(&mut self) {
        let Some(rx) = &self.job_rx else { return };
        let mut finished = false;
        let mut msgs = Vec::new();
        loop {
            match rx.try_recv() {
                Ok(m) => msgs.push(m),
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    // kanaal dicht: normaal na 'Finished'; zonder 'Finished' is de thread
                    // gecrasht -> luid melden en vrijgeven
                    if !msgs.iter().any(|m| matches!(m, JobMsg::Finished)) {
                        msgs.push(JobMsg::Error(
                            "de vergelijking is onverwacht gestopt".into(),
                        ));
                        msgs.push(JobMsg::Finished);
                    }
                    break;
                }
            }
        }
        for m in msgs {
            match m {
                JobMsg::Log(s) => self.log.push(s),
                JobMsg::Progress(d, t) => self.progress = (d, t),
                JobMsg::Scanned(a, b) => {
                    self.issue_a = Some(*a);
                    self.issue_b = Some(*b);
                }
                JobMsg::Done(d) => {
                    self.log
                        .push(format!("{} bladen vergeleken", d.sheets.len()));
                    let mut d = *d;
                    d.sheets.sort_by(|x, y| x.key.cmp(&y.key));
                    self.diff = Some(d);
                }
                JobMsg::Comments(r) => {
                    let c = PathBuf::from(self.path_c.trim().trim_matches('"'));
                    let sp = ReviewState::default_path(&c);
                    match ReviewState::load_checked(
                        &sp,
                        &PathBuf::from(&self.path_a),
                        &PathBuf::from(&self.path_b),
                        &c,
                    ) {
                        Ok((st, warn)) => {
                            if let Some(w) = warn {
                                self.log.push(w);
                            }
                            self.review = st;
                            self.review_path = Some(sp);
                        }
                        Err(e) => self.log.push(format!("statusbestand: {e:#}")),
                    }
                    for w in &r.warnings {
                        self.log.push(w.clone());
                    }
                    self.log.push(format!("{} opmerkingen", r.comments.len()));
                    self.comments = Some(*r);
                }
                JobMsg::Error(e) => {
                    self.log.push(format!("FOUT: {e}"));
                    self.message = Some((e, true));
                }
                JobMsg::Finished => finished = true,
            }
        }
        if finished {
            self.job_rx = None;
        }
    }

    fn poll_render(&mut self, ctx: &egui::Context) {
        while let Ok(r) = self.render.rx.try_recv() {
            match r {
                Resp::Base(g, l) if g == self.render_gen => {
                    self.viewer.base = Some(viewer::make_tex(ctx, "base", l));
                    self.viewer.detail = None;
                }
                Resp::Detail(g, l) if g == self.render_gen => {
                    self.viewer.detail = Some(viewer::make_tex(ctx, "detail", l));
                }
                Resp::Vector(g, v) if g == self.render_gen => self.viewer.vector = Some(v),
                Resp::Zone(id, l, c) => {
                    let tex = viewer::make_tex(ctx, "zone", l);
                    let ct = c.map(|c| {
                        ctx.load_texture(
                            "zone-c",
                            egui::ColorImage::from_rgba_unmultiplied([c.w, c.h], &c.data),
                            Default::default(),
                        )
                    });
                    self.zone = Some((id, tex, ct));
                }
                Resp::ZoneError(id, e) => {
                    if self.sel_comment == Some(id) {
                        self.zone_error = Some(e.clone());
                    }
                    self.log.push(format!("uitsnede: {e}"));
                }
                Resp::Error(g, e) => {
                    if g == self.render_gen || g == 0 {
                        self.viewer.error = Some(e.clone());
                    }
                    self.log.push(format!("render: {e}"));
                }
                _ => {}
            }
        }
    }

    // ------------------------------------------------------------------ bladen

    fn sheet_src(&self, s: &SheetDiff) -> Option<SheetSrc> {
        let size = self
            .issue_a
            .as_ref()
            .and_then(|i| i.sheets.get(&s.key))
            .or_else(|| self.issue_b.as_ref().and_then(|i| i.sheets.get(&s.key)))
            .map(|r| r.geom.display_size())?;
        Some(SheetSrc {
            a: s.a.as_ref().map(|l| (l.file.clone(), l.page)),
            b: s.b.as_ref().map(|l| (l.file.clone(), l.page)),
            b_to_a: s
                .registration
                .as_ref()
                .map(|r| r.transform)
                .unwrap_or(Affine::IDENTITY),
            size_pt: size,
        })
    }

    fn open_sheet(&mut self, idx: usize) {
        let Some(d) = &self.diff else { return };
        let Some(s) = d.sheets.get(idx) else { return };
        let Some(src) = self.sheet_src(s) else { return };
        self.viewer.reset(src.size_pt);
        self.render_gen = self.render.open(src, self.cfg.compare.diff);
        if self.sel_sheet != Some(idx) {
            self.align = None; // uitlijning hoort bij een blad
        }
        self.sel_sheet = Some(idx);
        self.sel_region = None;
    }

    fn cur(&self) -> Option<&SheetDiff> {
        self.diff
            .as_ref()
            .and_then(|d| self.sel_sheet.and_then(|i| d.sheets.get(i)))
    }

    fn real_regions(&self) -> Vec<usize> {
        self.cur()
            .map(|s| {
                (0..s.regions.len())
                    .filter(|&i| !s.regions[i].stamp)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn goto_region(&mut self, i: usize) {
        if let Some(s) = self.cur() {
            if let Some(r) = s.regions.get(i) {
                let b = r.bbox;
                self.sel_region = Some(i);
                self.viewer.zoom_to(&b, 40.0);
                self.viewer.last_change = 0.0;
            }
        }
    }

    fn step_region(&mut self, dir: i32) {
        let rr = self.real_regions();
        if rr.is_empty() {
            return;
        }
        let pos = self
            .sel_region
            .and_then(|s| rr.iter().position(|&x| x == s));
        let next = match pos {
            None => 0,
            Some(p) => (p as i32 + dir).clamp(0, rr.len() as i32 - 1) as usize,
        };
        self.goto_region(rr[next]);
    }

    fn comment_marks(&self) -> Vec<CommentMark> {
        let (Some(r), Some(s)) = (&self.comments, self.cur()) else {
            return vec![];
        };
        r.comments
            .iter()
            .enumerate()
            .filter(|(_, c)| c.sheet.as_deref() == Some(&s.key))
            .filter_map(|(i, c)| {
                c.zone_a.map(|z| CommentMark {
                    number: c.number,
                    zone: z,
                    color: status_color(self.review.entry(&c.id).status, c.proposal),
                    selected: self.sel_comment == Some(i),
                })
            })
            .collect()
    }

    fn select_comment(&mut self, i: usize) {
        self.sel_comment = Some(i);
        self.zone = None;
        self.zone_error = None;
        let Some(r) = &self.comments else { return };
        let Some(c) = r.comments.get(i).cloned() else {
            return;
        };
        let creg = r.page_regs.get(&c.c_page).map(|(_, reg)| reg.transform);
        let cfile = r.file.clone();
        let (Some(nr), Some(za)) = (c.sheet.clone(), c.zone_a) else {
            return;
        };
        // blad openen in de viewer (op de achtergrond) en uitsnede aanvragen
        let idx = self
            .diff
            .as_ref()
            .and_then(|d| d.sheets.iter().position(|s| s.key == nr));
        if let Some(idx) = idx {
            if self.sel_sheet != Some(idx) {
                self.open_sheet(idx);
            }
            if let Some(s) = self.cur().cloned() {
                if let Some(src) = self.sheet_src(&s) {
                    let rect = za.expand(20.0 * PT_PER_MM);
                    let dpi = (900.0 / rect.width().max(rect.height()) * 72.0).clamp(30.0, 300.0);
                    self.render.zone(ZoneReq {
                        id: i,
                        src,
                        c: creg.map(|t| (cfile.clone(), c.c_page, t)),
                        rect,
                        dpi,
                        params: self.cfg.compare.diff,
                    });
                }
            }
            self.viewer.fit_pending = false;
            self.viewer.zoom_to(&za, 60.0);
        }
    }

    fn set_status(&mut self, id: &str, status: Status, note: String) {
        let e = self.review.entries.entry(id.to_string()).or_default();
        e.status = status;
        e.note = note;
        e.updated = now_iso();
        if self.review_path.is_none() {
            self.message = Some(("status wordt NIET opgeslagen: geen (leesbaar) statusbestand naast de opmerkingen-PDF".into(), true));
        }
        // opslaan gebundeld (niet bij elke toetsaanslag), zie flush_status
        self.status_dirty.get_or_insert(0.0);
    }

    // ------------------------------------------------------------------ UI

    fn titlebar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("titel")
            .exact_size(38.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::TITEL)
                    .inner_margin(egui::Margin::symmetric(12, 6)),
            )
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add(egui::Image::new(&self.logo).fit_to_exact_size(Vec2::new(26.0, 20.0)));
                    ui.label(
                        RichText::new("sheetdiff")
                            .strong()
                            .size(15.0)
                            .color(theme::PETROL),
                    );
                    let sub = match (&self.diff, self.cur()) {
                        (_, Some(s)) => format!("{} - {}", s.key, s.title),
                        (Some(d), None) => format!("{} bladen", d.sheets.len()),
                        _ => "tekeninguitgiftes vergelijken".into(),
                    };
                    ui.label(RichText::new(sub).color(theme::GRIJS));
                });
            });
        egui::Panel::top("tabs")
            .exact_size(34.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::TITEL)
                    .inner_margin(egui::Margin::symmetric(8, 0)),
            )
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    for (s, label) in [
                        (Screen::Start, "a. Start"),
                        (Screen::View, "b. Bladweergave"),
                        (Screen::Diffs, "c. Verschillen"),
                        (Screen::Comments, "d. Opmerkingen"),
                        (Screen::Export, "e. Export"),
                    ] {
                        let on = self.screen == s;
                        let txt = RichText::new(label).color(if on {
                            theme::PETROL
                        } else {
                            theme::INKT
                        });
                        let txt = if on { txt.strong() } else { txt };
                        let b = egui::Button::new(txt)
                            .fill(if on { Color32::WHITE } else { theme::TITEL })
                            .stroke(egui::Stroke::NONE);
                        let r = ui.add(b);
                        if on {
                            let rr = r.rect;
                            ui.painter().hline(
                                rr.x_range(),
                                rr.bottom(),
                                egui::Stroke::new(2.0, theme::TEAL),
                            );
                        }
                        if r.clicked() {
                            self.screen = s;
                            if s == Screen::View && self.sel_sheet.is_none() {
                                if let Some(i) = self.first_changed() {
                                    self.open_sheet(i);
                                }
                            }
                        }
                    }
                });
            });
    }

    fn first_changed(&self) -> Option<usize> {
        let d = self.diff.as_ref()?;
        d.sheets
            .iter()
            .position(|s| s.verdict == Verdict::Changed)
            .or(if d.sheets.is_empty() { None } else { Some(0) })
    }

    fn statusbar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status")
            .exact_size(24.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::PETROL)
                    .inner_margin(egui::Margin::symmetric(12, 3)),
            )
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    let t = |s: String| RichText::new(s).color(theme::STATUS_TEKST).size(11.5);
                    if let Some(d) = &self.diff {
                        ui.label(t(format!("{} bladen", d.sheets.len())));
                        for v in [
                            Verdict::Changed,
                            Verdict::MovedOnly,
                            Verdict::StampOnly,
                            Verdict::Identical,
                            Verdict::NotComparable,
                        ] {
                            ui.label(t(format!("{} {}", v.label(), d.count(v))));
                        }
                    } else {
                        ui.label(t("geen vergelijking geladen".into()));
                    }
                    ui.label(t(format!("{} dpi analyse", self.cfg.compare.dpi)));
                    if self.screen == Screen::View {
                        ui.label(t(format!("zoom {:.0} dpi", self.viewer.zoom * 72.0)));
                        if let Some((x, y)) = self.hover_mm {
                            ui.label(t(format!("{x:.0}, {y:.0} mm")));
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(t("Kolthof Bouwadvies - sheetdiff".into()));
                    });
                });
            });
    }

    fn left_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("bladen")
            .default_size(250.0)
            .resizable(true)
            .frame(
                egui::Frame::new()
                    .fill(theme::VLAK)
                    .inner_margin(egui::Margin::same(0)),
            )
            .show(ui, |ui| {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new("BLADEN")
                            .size(11.0)
                            .color(theme::PETROL)
                            .strong(),
                    );
                    egui::ComboBox::from_id_salt("filter")
                        .selected_text(self.filter.map(|v| v.label()).unwrap_or("alle"))
                        .width(130.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.filter, None, "alle");
                            for v in [
                                Verdict::Changed,
                                Verdict::MovedOnly,
                                Verdict::NotComparable,
                                Verdict::AddedInB,
                                Verdict::RemovedFromB,
                                Verdict::StampOnly,
                                Verdict::Identical,
                            ] {
                                ui.selectable_value(&mut self.filter, Some(v), v.label());
                            }
                        });
                });
                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        let Some(d) = &self.diff else {
                            ui.add_space(8.0);
                            ui.label(RichText::new("   nog geen vergelijking").color(theme::GRIJS));
                            return;
                        };
                        let mut open = None;
                        for (i, s) in d.sheets.iter().enumerate() {
                            if self.filter.is_some_and(|f| f != s.verdict) {
                                continue;
                            }
                            let on = self.sel_sheet == Some(i) && self.screen != Screen::Start;
                            let nc = self
                                .comments
                                .as_ref()
                                .map(|r| {
                                    r.comments
                                        .iter()
                                        .filter(|c| c.sheet.as_deref() == Some(&s.key))
                                        .count()
                                })
                                .unwrap_or(0);
                            let resp = ui
                                .horizontal(|ui| {
                                    let r = ui.allocate_space(Vec2::new(3.0, 20.0)).1;
                                    if on {
                                        ui.painter().rect_filled(r, 0.0, theme::TEAL);
                                    }
                                    ui.add_space(6.0);
                                    ui.add_sized(
                                        [70.0, 18.0],
                                        egui::Label::new(RichText::new(&s.key).strong()).truncate(),
                                    );
                                    chip(ui, s.verdict.label(), theme::verdict_colors(s.verdict));
                                    if nc > 0 {
                                        ui.label(
                                            RichText::new(format!("{nc} opm"))
                                                .size(10.5)
                                                .color(theme::GRIJS),
                                        );
                                    }
                                })
                                .response
                                .interact(egui::Sense::click());
                            if on {
                                ui.painter().rect_filled(
                                    resp.rect,
                                    0.0,
                                    theme::ACTIEF.gamma_multiply(0.5),
                                );
                            }
                            let resp = resp.on_hover_text(&s.title);
                            if resp.clicked() {
                                open = Some(i);
                            }
                        }
                        if let Some(i) = open {
                            self.open_sheet(i);
                            if self.screen == Screen::Start || self.screen == Screen::Export {
                                self.screen = Screen::View;
                            }
                        }
                        if let Some(r) = &self.comments {
                            ui.add_space(10.0);
                            ui.label(
                                RichText::new("   OPMERKINGEN")
                                    .size(11.0)
                                    .color(theme::PETROL)
                                    .strong(),
                            );
                            if ui
                                .selectable_label(
                                    self.screen == Screen::Comments,
                                    format!("   {} opmerkingen", r.comments.len()),
                                )
                                .clicked()
                            {
                                self.screen = Screen::Comments;
                            }
                        }
                    });
            });
    }

    fn screen_start(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            ui.add_space(14.0);
            ui.label(RichText::new("Uitgiftes vergelijken").size(20.0).color(theme::PETROL).strong());
            ui.label(RichText::new("Kies twee uitgiftes (map met boekjes of losse PDF) en optioneel de PDF met opmerkingen van de aannemer.").color(theme::GRIJS));
            ui.add_space(8.0);
            let busy = self.job_rx.is_some();
            egui::Grid::new("paden").num_columns(3).spacing([10.0, 8.0]).show(ui, |ui| {
                for (label, which) in [("Uitgifte A (oud)", 0), ("Uitgifte B (nieuw)", 1), ("Opmerkingen (optioneel)", 2)] {
                    ui.label(RichText::new(label).strong());
                    let s = match which {
                        0 => &mut self.path_a,
                        1 => &mut self.path_b,
                        _ => &mut self.path_c,
                    };
                    ui.add_sized([620.0, 22.0], egui::TextEdit::singleline(s));
                    ui.horizontal(|ui| {
                        if which < 2 && ui.button("Map...").clicked() {
                            if let Some(p) = rfd::FileDialog::new().pick_folder() {
                                *s = p.display().to_string();
                            }
                        }
                        if ui.button("PDF...").clicked() {
                            if let Some(p) = rfd::FileDialog::new().add_filter("PDF", &["pdf"]).pick_file() {
                                *s = p.display().to_string();
                            }
                        }
                    });
                    ui.end_row();
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let can = !busy && !self.path_a.trim().is_empty() && !self.path_b.trim().is_empty();
                let btn = egui::Button::new(RichText::new("Vergelijken").color(Color32::WHITE).strong()).fill(theme::PETROL);
                if ui.add_enabled(can, btn).clicked() {
                    self.start_compare(ui.ctx());
                }
                ui.label(RichText::new(format!(
                    "{} dpi, tolerantie {}, lijnafstand {} px, {} werkers",
                    self.cfg.compare.dpi, self.cfg.compare.diff.tolerance, self.cfg.compare.diff.radius_px, self.workers
                )).color(theme::GRIJS));
            });
            egui::CollapsingHeader::new("Instellingen").show(ui, |ui| {
                egui::Grid::new("inst").num_columns(2).show(ui, |ui| {
                    ui.label("Analyse-dpi");
                    ui.add(egui::Slider::new(&mut self.cfg.compare.dpi, 150.0..=600.0).step_by(50.0));
                    ui.end_row();
                    ui.label("Kleurtolerantie (grijswaarden)");
                    ui.add(egui::Slider::new(&mut self.cfg.compare.diff.tolerance, 8..=160));
                    ui.end_row();
                    ui.label("Minimale lijnafstand (px)");
                    ui.add(egui::Slider::new(&mut self.cfg.compare.diff.radius_px, 0..=4));
                    ui.end_row();
                    ui.label("Werkerprocessen (0 = in de app)");
                    ui.add(egui::Slider::new(&mut self.workers, 0..=16));
                    ui.end_row();
                    ui.label("Eigen auteurs (opmerkingen)");
                    let mut s = self.cfg.comments.own_authors.join(", ");
                    if ui.text_edit_singleline(&mut s).changed() {
                        self.cfg.comments.own_authors = s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect();
                    }
                    ui.end_row();
                });
                if ui.button("Standaardwaarden").clicked() {
                    self.cfg = Config::default();
                }
            });
            if busy {
                let (d, t) = self.progress;
                let f = if t == 0 { 0.0 } else { d as f32 / t as f32 };
                ui.add(egui::ProgressBar::new(f).text(format!("{d} / {t} bladen")).desired_width(640.0));
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
            }
            for l in self.log.iter().rev().take(6).collect::<Vec<_>>().into_iter().rev() {
                let col = if l.starts_with("FOUT") || l.contains("NIET") || l.contains("ANDERE inhoud") { theme::ROOD } else { theme::GRIJS };
                ui.label(RichText::new(l).color(col).size(12.0));
            }
            note(ui, "Vergelijkt op tekeningnummer (uit de titelhoek), niet op paginavolgorde of boekje. Een blad dat niet betrouwbaar uit te lijnen is krijgt NIET VERGELIJKBAAR en telt nooit als gelijk.");
            let mut warn: Vec<String> = Vec::new();
            if let Some(d) = &self.diff {
                warn.extend(d.warnings.iter().cloned());
            }
            if !warn.is_empty() {
                egui::CollapsingHeader::new(RichText::new(format!("Waarschuwingen ({})", warn.len())).color(theme::OKER)).show(ui, |ui| {
                    for w in warn {
                        ui.label(RichText::new(w).size(12.0));
                    }
                });
            }
            self.sheet_table(ui);
        });
    }

    fn sheet_table(&mut self, ui: &mut egui::Ui) {
        let Some(d) = &self.diff else { return };
        ui.add_space(8.0);
        let mut open = None;
        egui::Grid::new("bladtabel")
            .striped(true)
            .num_columns(9)
            .spacing([14.0, 6.0])
            .show(ui, |ui| {
                for h in [
                    "Blad",
                    "Oordeel",
                    "Pixels",
                    "Regio's",
                    "Tekst gew/+/-/verpl",
                    "Gevonden door",
                    "Uitlijning B op A",
                    "Opm.",
                    "Tijd",
                ] {
                    ui.label(RichText::new(h).size(11.5).color(theme::GRIJS).strong());
                }
                ui.end_row();
                for (i, s) in d.sheets.iter().enumerate() {
                    if self.filter.is_some_and(|f| f != s.verdict) {
                        continue;
                    }
                    if ui
                        .link(RichText::new(&s.key).strong())
                        .on_hover_text(&s.title)
                        .clicked()
                    {
                        open = Some(i);
                    }
                    chip(ui, s.verdict.label(), theme::verdict_colors(s.verdict));
                    ui.label(format!("{:.3} %", s.pixel_pct).replace('.', ","));
                    ui.label(
                        s.regions
                            .iter()
                            .filter(|r| !r.stamp && r.moved_mm.is_none())
                            .count()
                            .to_string(),
                    );
                    let n = |k| s.text.iter().filter(|t| t.kind == k).count();
                    ui.label(format!(
                        "{} / {} / {} / {}",
                        n(TextChangeKind::Changed),
                        n(TextChangeKind::Added),
                        n(TextChangeKind::Removed),
                        n(TextChangeKind::Moved)
                    ));
                    ui.horizontal(|ui| {
                        let r = s.regions.iter().any(|r| !r.stamp);
                        let v = s.vector.as_ref().is_some_and(|v| !v.clusters.is_empty());
                        let t = s.text.iter().any(|t| t.kind != TextChangeKind::Stamp);
                        for (on, l) in [(r, "raster"), (v, "vector"), (t, "tekst")] {
                            chip(
                                ui,
                                l,
                                if on {
                                    theme::verdict_colors(Verdict::Identical)
                                } else {
                                    (theme::VLAK, theme::LIJN)
                                },
                            );
                        }
                    });
                    let reg = s
                        .registration
                        .as_ref()
                        .map(|r| {
                            if r.identity {
                                "identiek".to_string()
                            } else {
                                r.note.clone()
                            }
                        })
                        .unwrap_or_default();
                    ui.label(
                        RichText::new(reg.clone())
                            .color(if reg == "identiek" {
                                theme::GRIJS
                            } else {
                                theme::OKER
                            })
                            .size(12.0),
                    );
                    let nc = self
                        .comments
                        .as_ref()
                        .map(|r| {
                            r.comments
                                .iter()
                                .filter(|c| c.sheet.as_deref() == Some(&s.key))
                                .count()
                        })
                        .unwrap_or(0);
                    ui.label(if nc > 0 {
                        nc.to_string()
                    } else {
                        String::new()
                    });
                    ui.label(format!("{:.1} s", s.seconds));
                    ui.end_row();
                }
            });
        if let Some(i) = open {
            self.open_sheet(i);
            self.screen = Screen::View;
        }
    }

    fn b_size(&self) -> Option<(f64, f64)> {
        let s = self.cur()?;
        self.issue_b
            .as_ref()?
            .sheets
            .get(&s.key)
            .map(|r| r.geom.display_size())
    }

    fn align_window(&mut self, ctx: &egui::Context) {
        let Some(mut e) = self.align else { return };
        let before = e;
        let mut open = true;
        let mut apply = false;
        let mut auto = false;
        egui::Window::new("Uitlijning B op A")
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                if let Some(r) = self.cur().and_then(|s| s.registration.as_ref()) {
                    ui.label(RichText::new(format!("gevonden: {} (overlap {:.1} %)", r.note, r.score * 100.0)).size(12.0).color(theme::GRIJS));
                }
                egui::Grid::new("al").num_columns(2).show(ui, |ui| {
                    use sheetdiff_core::geom::Rot90;
                    ui.label("Rotatie");
                    egui::ComboBox::from_id_salt("rot").selected_text(format!("{} gr", e.rot.degrees())).show_ui(ui, |ui| {
                        for r in Rot90::ALL {
                            ui.selectable_value(&mut e.rot, r, format!("{} gr", r.degrees()));
                        }
                    });
                    ui.end_row();
                    ui.label("Schaal");
                    ui.add(egui::DragValue::new(&mut e.scale).speed(0.0001).range(0.9..=1.1).fixed_decimals(4));
                    ui.end_row();
                    ui.label("Verschuiving x (mm)");
                    ui.add(egui::DragValue::new(&mut e.dx).speed(0.05).fixed_decimals(2));
                    ui.end_row();
                    ui.label("Verschuiving y (mm)");
                    ui.add(egui::DragValue::new(&mut e.dy).speed(0.05).fixed_decimals(2));
                    ui.end_row();
                });
                ui.label(RichText::new("De weergave volgt direct; 'Opnieuw vergelijken' rekent het blad met deze uitlijning door.").size(11.5).color(theme::GRIJS));
                ui.horizontal(|ui| {
                    let busy = self.sheet_job.is_some();
                    if ui.add_enabled(!busy, egui::Button::new(RichText::new("Opnieuw vergelijken").color(Color32::WHITE)).fill(theme::PETROL)).clicked() {
                        apply = true;
                    }
                    if ui.add_enabled(!busy, egui::Button::new("Automatisch")).clicked() {
                        auto = true;
                    }
                    if busy {
                        ui.add(egui::Spinner::new());
                    }
                });
            });
        // live voorbeeld in de viewer
        if e != before {
            if let (Some(bs), Some(s)) = (self.b_size(), self.cur().cloned()) {
                if let Some(mut src) = self.sheet_src(&s) {
                    src.b_to_a = sheetdiff_core::register::Registration::manual(
                        e.rot, bs, e.scale, e.dx, e.dy,
                    )
                    .transform;
                    // oude basislaag blijft staan tot de nieuwe er is
                    self.viewer.detail = None;
                    self.viewer.detail_requested = None;
                    self.render_gen = self.render.open_with(src, self.cfg.compare.diff, false);
                }
            }
        }
        if apply || auto {
            self.recompare_sheet(if auto { None } else { Some(e) }, ctx);
        }
        self.align = if open { Some(e) } else { None };
    }

    fn recompare_sheet(&mut self, e: Option<AlignEdit>, ctx: &egui::Context) {
        let Some(s) = self.cur().cloned() else {
            return;
        };
        let key = s.key.clone();
        let (Some(sa), Some(sb)) = (
            self.issue_a
                .as_ref()
                .and_then(|i| i.sheets.get(&s.key))
                .cloned(),
            self.issue_b
                .as_ref()
                .and_then(|i| i.sheets.get(&s.key))
                .cloned(),
        ) else {
            return;
        };
        let manual = e.map(|e| {
            let mut r = sheetdiff_core::register::Registration::manual(
                e.rot,
                sb.geom.display_size(),
                e.scale,
                e.dx,
                e.dy,
            );
            r.note = format!(
                "handmatig: rotatie {} gr, schaal {:.4}, verschuiving {:.2} / {:.2} mm",
                e.rot.degrees(),
                e.scale,
                e.dx,
                e.dy
            );
            r
        });
        let cfg = self.cfg.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.sheet_job = Some(rx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let r = compare::compare_sheet(&sa, &sb, &cfg, manual.as_ref())
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send((key, r));
            ctx.request_repaint();
        });
    }

    fn poll_sheet_job(&mut self) {
        let Some(rx) = &self.sheet_job else { return };
        let res = match rx.try_recv() {
            Ok(v) => Some(v),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Some((
                String::new(),
                Err("herberekening onverwacht gestopt".into()),
            )),
        };
        if let Some((key, r)) = res {
            self.sheet_job = None;
            match r {
                Ok(mut d) => {
                    d.mask = None;
                    // op sleutel terugzetten: de lijst kan intussen opnieuw zijn opgebouwd
                    let idx = self
                        .diff
                        .as_ref()
                        .and_then(|df| df.sheets.iter().position(|s| s.key == key));
                    let Some(idx) = idx else {
                        self.message = Some((
                            format!("{key}: resultaat hoort niet meer bij de huidige vergelijking"),
                            true,
                        ));
                        return;
                    };
                    if let Some(slot) = self.diff.as_mut().and_then(|df| df.sheets.get_mut(idx)) {
                        *slot = d;
                    }
                    if self.sel_sheet != Some(idx) {
                        self.message = Some((format!("{key} opnieuw vergeleken"), false));
                        return;
                    }
                    let keep = (self.viewer.zoom, self.viewer.pan);
                    self.open_sheet(idx);
                    self.viewer.fit_pending = false;
                    (self.viewer.zoom, self.viewer.pan) = keep;
                    if let Some(r) = self.cur().and_then(|s| s.registration.clone()) {
                        self.align = self.align.map(|_| AlignEdit {
                            rot: r.rotation,
                            scale: r.scale,
                            dx: r.dx_mm,
                            dy: r.dy_mm,
                        });
                    }
                    self.message = Some(("blad opnieuw vergeleken".into(), false));
                }
                Err(e) => self.message = Some((e, true)),
            }
        }
    }

    /// Schrijft het statusbestand hooguit eens per halve seconde.
    fn flush_status(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        match self.status_dirty {
            Some(0.0) => {
                self.status_dirty = Some(now + 0.5);
                ctx.request_repaint_after(std::time::Duration::from_millis(600));
            }
            Some(t) if now >= t => {
                self.status_dirty = None;
                if let Some(p) = &self.review_path {
                    if let Err(e) = self.review.save(p) {
                        self.message = Some((format!("status niet opgeslagen: {e:#}"), true));
                    }
                }
            }
            _ => {}
        }
    }

    fn screen_view(&mut self, ui: &mut egui::Ui) {
        if self.sel_sheet.is_none() {
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new("Kies een blad links (of start eerst een vergelijking).")
                        .color(theme::GRIJS),
                )
            });
            return;
        }
        if self.last_params != Some(self.cfg.compare.diff) {
            if self.last_params.is_some() {
                self.render.params(self.cfg.compare.diff);
            }
            self.last_params = Some(self.cfg.compare.diff);
        }
        // werkbalk
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().slider_width = 110.0;
            ui.add_space(6.0);
            ui.label(
                RichText::new(self.cur().map(|s| s.key.clone()).unwrap_or_default())
                    .strong()
                    .color(theme::PETROL),
            );
            ui.separator();
            for (m, l) in [
                (Mode::Overlay, "Overlay"),
                (Mode::Blink, "Knipperen"),
                (Mode::Side, "Naast elkaar"),
                (Mode::Pixel, "Pixelverschil"),
                (Mode::Vector, "Vectorverschil"),
            ] {
                if ui.selectable_label(self.viewer.mode == m, l).clicked() {
                    self.viewer.mode = m;
                    self.viewer.blink_auto = m == Mode::Blink;
                }
            }
            ui.separator();
            if self.viewer.mode == Mode::Overlay {
                ui.label("A");
                ui.add(egui::Slider::new(&mut self.viewer.mix, 0.0..=1.0).show_value(false));
                ui.label("B");
            }
            ui.checkbox(&mut self.viewer.show_regions, "regio's");
            ui.checkbox(&mut self.viewer.show_comments, "opmerkingen");
            if ui.button("Passend").clicked() {
                self.viewer.fit_pending = true;
            }
            if ui
                .button("1:1 pixel")
                .on_hover_text("zoom tot 1 schermpixel = 1 renderpixel op 300 dpi")
                .clicked()
            {
                let ppp = ui.ctx().pixels_per_point();
                let pane = self.viewer.pane_size();
                let (cx, cy) = (
                    (self.viewer.pan.x + pane.x / 2.0 / self.viewer.zoom) as f64,
                    (self.viewer.pan.y + pane.y / 2.0 / self.viewer.zoom) as f64,
                );
                self.viewer.zoom = (300.0 / 72.0) / ppp;
                self.viewer.pan = Vec2::new(
                    cx as f32 - pane.x / 2.0 / self.viewer.zoom,
                    cy as f32 - pane.y / 2.0 / self.viewer.zoom,
                );
                self.viewer.last_change = 0.0;
            }
            if ui
                .button("Uitlijning...")
                .on_hover_text("gevonden uitlijning bekijken en handmatig bijstellen")
                .clicked()
            {
                if let Some(r) = self.cur().and_then(|s| s.registration.clone()) {
                    self.align = Some(AlignEdit {
                        rot: r.rotation,
                        scale: r.scale,
                        dx: r.dx_mm,
                        dy: r.dy_mm,
                    });
                }
            }
            legend(ui);
        });
        self.align_window(ui.ctx());
        ui.separator();
        // knipperen
        let now = ui.input(|i| i.time);
        if self.viewer.mode == Mode::Blink && self.viewer.blink_auto {
            if now - self.viewer.last_blink > 0.6 {
                self.viewer.blink_b = !self.viewer.blink_b;
                self.viewer.last_blink = now;
            }
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        let diff = self.cur().cloned();
        let marks = self.comment_marks();
        let (_, hover) = self.viewer.show(ui, diff.as_ref(), self.sel_region, &marks);
        self.hover_mm = hover.map(viewer::Viewer::hover_mm);
        // detaillaag aanvragen als we verder zijn ingezoomd dan de basislaag en de weergave
        // even stilstaat
        let ppp = ui.ctx().pixels_per_point();
        if let Some(base) = &self.viewer.base {
            let sdpi = self.viewer.screen_dpi(ppp);
            let quiet = now - self.viewer.last_change > 0.25;
            if sdpi > base.dpi * 1.15 && quiet {
                let vis = self.viewer.visible_pt();
                let dpi = sdpi.min(1200.0);
                let need = match &self.viewer.detail_requested {
                    Some((r, d)) => {
                        (*d - dpi).abs() > 1.0
                            || (r.x0 - vis.x0).abs() > 1.0
                            || (r.y0 - vis.y0).abs() > 1.0
                            || (r.x1 - vis.x1).abs() > 1.0
                            || (r.y1 - vis.y1).abs() > 1.0
                    }
                    None => true,
                };
                if need && vis.area() > 1.0 {
                    self.render.detail(vis, dpi);
                    self.viewer.detail_requested = Some((vis, dpi));
                }
            } else if !quiet {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(300));
            }
        }
    }

    fn right_panel(&mut self, ui: &mut egui::Ui) {
        match self.screen {
            Screen::View | Screen::Diffs => {
                let Some(s) = self.cur().cloned() else { return };
                egui::Panel::right("verschillen")
                    .default_size(330.0)
                    .resizable(true)
                    .show(ui, |ui| {
                        let rr = self.real_regions();
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!("Verschillen ({})", rr.len()))
                                    .strong()
                                    .color(theme::PETROL),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .button("volgende")
                                        .on_hover_text("volgende verschil (N)")
                                        .clicked()
                                    {
                                        self.step_region(1);
                                    }
                                    if ui
                                        .button("vorige")
                                        .on_hover_text("vorige verschil (P)")
                                        .clicked()
                                    {
                                        self.step_region(-1);
                                    }
                                },
                            );
                        });
                        for r in &s.reasons {
                            ui.label(RichText::new(r).size(11.5).color(theme::OKER));
                        }
                        if let Some(v) = &s.vector {
                            ui.label(
                                RichText::new(format!(
                                    "vector: {} / {} paden, {} weg, {} nieuw, {} stijl",
                                    v.paths_a,
                                    v.paths_b,
                                    v.removed_segs,
                                    v.added_segs,
                                    v.style_paths
                                ))
                                .size(11.0)
                                .color(theme::GRIJS),
                            );
                        }
                        ui.separator();
                        egui::ScrollArea::vertical()
                            .auto_shrink([false; 2])
                            .show(ui, |ui| {
                                let mut go = None;
                                for (k, &i) in rr.iter().enumerate() {
                                    let r = &s.regions[i];
                                    let sel = self.sel_region == Some(i);
                                    let resp = ui
                                        .vertical(|ui| {
                                            ui.horizontal(|ui| {
                                                ui.label(
                                                    RichText::new(format!("{}", k + 1)).strong(),
                                                );
                                                ui.label(
                                                    RichText::new(format!(
                                                        "{:.0}, {:.0} mm",
                                                        r.bbox.x0 * MM_PER_PT,
                                                        r.bbox.y0 * MM_PER_PT
                                                    ))
                                                    .size(11.0)
                                                    .color(theme::GRIJS),
                                                );
                                                if r.moved_mm.is_some() {
                                                    chip(
                                                        ui,
                                                        "verplaatst",
                                                        theme::verdict_colors(Verdict::MovedOnly),
                                                    );
                                                }
                                                if r.by_vector {
                                                    chip(
                                                        ui,
                                                        "vector",
                                                        theme::verdict_colors(Verdict::Identical),
                                                    );
                                                }
                                                ui.label(
                                                    RichText::new(format!("-{}", r.removed_px))
                                                        .color(theme::C_REMOVED)
                                                        .size(11.0),
                                                );
                                                ui.label(
                                                    RichText::new(format!("+{}", r.added_px))
                                                        .color(Color32::from_rgb(0, 124, 147))
                                                        .size(11.0),
                                                );
                                            });
                                            let texts: Vec<String> = r
                                                .text
                                                .iter()
                                                .filter_map(|&t| s.text.get(t))
                                                .filter(|t| t.kind != TextChangeKind::Stamp)
                                                .map(text_label)
                                                .collect();
                                            if texts.is_empty() {
                                                ui.label(
                                                    RichText::new("alleen grafisch")
                                                        .size(11.5)
                                                        .color(theme::GRIJS),
                                                );
                                            } else {
                                                for t in texts.iter().take(4) {
                                                    ui.label(RichText::new(t).size(12.0));
                                                }
                                            }
                                        })
                                        .response
                                        .interact(egui::Sense::click());
                                    if sel {
                                        ui.painter().rect_stroke(
                                            resp.rect.expand(2.0),
                                            2.0,
                                            egui::Stroke::new(2.0, theme::TEAL),
                                            egui::StrokeKind::Outside,
                                        );
                                    }
                                    if resp.clicked() {
                                        go = Some(i);
                                    }
                                    ui.separator();
                                }
                                if let Some(i) = go {
                                    self.screen = Screen::View;
                                    self.goto_region(i);
                                }
                            });
                    });
            }
            Screen::Comments => self.comments_panel(ui),
            _ => {}
        }
    }

    fn comments_panel(&mut self, ui: &mut egui::Ui) {
        let Some(r) = self.comments.clone() else {
            return;
        };
        egui::Panel::right("opmerkingen")
            .default_size(380.0)
            .resizable(true)
            .show(ui, |ui| {
                ui.label(
                    RichText::new(format!("Opmerkingen ({})", r.comments.len()))
                        .strong()
                        .color(theme::PETROL),
                );
                ui.horizontal(|ui| {
                    use comments::Proposal::*;
                    for p in [LikelyDone, NotDone, Unclear, Info] {
                        let n = r.comments.iter().filter(|c| c.proposal == p).count();
                        chip(ui, &format!("{} {n}", p.label()), theme::proposal_colors(p));
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        let mut select = None;
                        let mut change: Option<(String, Status, String)> = None;
                        for (i, c) in r.comments.iter().enumerate() {
                            let e = self.review.entry(&c.id);
                            let sel = self.sel_comment == Some(i);
                            let frame = egui::Frame::new()
                                .fill(if sel { theme::ACTIEF } else { Color32::WHITE })
                                .inner_margin(egui::Margin::same(6));
                            let resp = frame
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new(format!("#{}", c.number)).strong());
                                        ui.label(
                                            RichText::new(
                                                c.sheet
                                                    .clone()
                                                    .unwrap_or_else(|| "algemeen".into()),
                                            )
                                            .size(11.0)
                                            .color(theme::GRIJS),
                                        );
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                chip(
                                                    ui,
                                                    c.proposal.label(),
                                                    theme::proposal_colors(c.proposal),
                                                );
                                            },
                                        );
                                    });
                                    ui.label(RichText::new(&c.text).size(12.5));
                                    ui.label(
                                        RichText::new(format!(
                                            "{} - {} - {}",
                                            c.author, c.subject, c.reason
                                        ))
                                        .size(11.0)
                                        .color(theme::GRIJS),
                                    );
                                    if !c.own_marks.is_empty() {
                                        ui.label(
                                            RichText::new(format!(
                                                "in PDF: {}",
                                                c.own_marks.join(", ")
                                            ))
                                            .size(11.0)
                                            .color(theme::PETROL),
                                        );
                                    }
                                    if !c.text_changes.is_empty() {
                                        ui.label(
                                            RichText::new(c.text_changes.join("; "))
                                                .size(11.0)
                                                .color(theme::GRIJS),
                                        );
                                    }
                                    ui.horizontal(|ui| {
                                        ui.label("Status:");
                                        let mut st = e.status;
                                        egui::ComboBox::from_id_salt(("st", i))
                                            .selected_text(if st == Status::Open {
                                                "open"
                                            } else {
                                                st.label()
                                            })
                                            .show_ui(ui, |ui| {
                                                for s in Status::ALL {
                                                    ui.selectable_value(
                                                        &mut st,
                                                        s,
                                                        if s == Status::Open {
                                                            "open"
                                                        } else {
                                                            s.label()
                                                        },
                                                    );
                                                }
                                            });
                                        if st != e.status {
                                            change = Some((c.id.clone(), st, e.note.clone()));
                                        }
                                    });
                                    let mut note = e.note.clone();
                                    if ui
                                        .add(
                                            egui::TextEdit::multiline(&mut note)
                                                .desired_rows(1)
                                                .hint_text("notitie")
                                                .desired_width(f32::INFINITY),
                                        )
                                        .changed()
                                    {
                                        change = Some((c.id.clone(), e.status, note));
                                    }
                                })
                                .response
                                .interact(egui::Sense::click());
                            if resp.clicked() {
                                select = Some(i);
                            }
                            ui.separator();
                        }
                        if let Some((id, st, n)) = change {
                            self.set_status(&id, st, n);
                        }
                        if let Some(i) = select {
                            self.select_comment(i);
                        }
                    });
            });
    }

    fn screen_diffs(&mut self, ui: &mut egui::Ui) {
        let Some(s) = self.cur().cloned() else {
            ui.label(RichText::new("Kies een blad.").color(theme::GRIJS));
            return;
        };
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            ui.add_space(10.0);
            ui.label(RichText::new(format!("Verschillenlijst - {} {}", s.key, s.title)).size(18.0).strong().color(theme::PETROL));
            ui.label(RichText::new("Elke regio is een navigatiepunt; de weergave blijft de pixel. N / P = volgende / vorige.").color(theme::GRIJS));
            let mut go = None;
            egui::Grid::new("diffs").striped(true).num_columns(6).spacing([14.0, 6.0]).show(ui, |ui| {
                for h in ["#", "Plek (mm)", "Gevonden door", "weg px", "nieuw px", "Tekst oud -> nieuw"] {
                    ui.label(RichText::new(h).strong().size(11.5).color(theme::GRIJS));
                }
                ui.end_row();
                for (k, i) in self.real_regions().into_iter().enumerate() {
                    let r = &s.regions[i];
                    if ui.link((k + 1).to_string()).clicked() {
                        go = Some(i);
                    }
                    ui.label(format!("{:.0}, {:.0}", r.bbox.x0 * MM_PER_PT, r.bbox.y0 * MM_PER_PT));
                    ui.horizontal(|ui| {
                        chip(ui, "raster", theme::verdict_colors(Verdict::Identical));
                        if r.by_vector {
                            chip(ui, "vector", theme::verdict_colors(Verdict::Identical));
                        }
                        if r.moved_mm.is_some() {
                            chip(ui, "verplaatst", theme::verdict_colors(Verdict::MovedOnly));
                        }
                    });
                    ui.label(r.removed_px.to_string());
                    ui.label(r.added_px.to_string());
                    let texts: Vec<String> = r.text.iter().filter_map(|&t| s.text.get(t)).filter(|t| t.kind != TextChangeKind::Stamp).map(text_label).collect();
                    ui.label(texts.join("\n"));
                    ui.end_row();
                }
            });
            if let Some(v) = &s.vector {
                let only: Vec<_> = v.clusters.iter().filter(|c| !c.in_region).collect();
                if !only.is_empty() {
                    ui.add_space(10.0);
                    ui.label(RichText::new(format!("Alleen door vector gezien ({})", only.len())).strong().color(theme::OKER));
                    for c in only.iter().take(50) {
                        ui.label(format!(
                            "{:?} bij {:.0}, {:.0} mm - {:.1} mm lijn {}",
                            c.kind, c.bbox.x0 * MM_PER_PT, c.bbox.y0 * MM_PER_PT, c.length_mm, c.note
                        ));
                    }
                }
            }
            if let Some(i) = go {
                self.screen = Screen::View;
                self.goto_region(i);
            }
        });
    }

    fn screen_comments(&mut self, ui: &mut egui::Ui) {
        let Some(r) = self.comments.clone() else {
            ui.add_space(12.0);
            ui.label(RichText::new("Geen opmerkingen-PDF geladen. Kies er een op het startscherm en vergelijk opnieuw.").color(theme::GRIJS));
            return;
        };
        ui.add_space(10.0);
        ui.label(
            RichText::new("Opmerkingen aannemer")
                .size(18.0)
                .strong()
                .color(theme::PETROL),
        );
        ui.label(RichText::new("Elke opmerking is uitgelijnd op A en op B; het voorstel komt uit het verschil A->B in de opmerkingszone. Het is een voorstel, geen oordeel: de status zet je zelf.").color(theme::GRIJS));
        let Some(i) = self.sel_comment else {
            ui.label(RichText::new("Kies rechts een opmerking.").color(theme::GRIJS));
            return;
        };
        let Some(c) = r.comments.get(i) else { return };
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!(
                    "#{} {}",
                    c.number,
                    c.sheet.clone().unwrap_or_else(|| "algemeen".into())
                ))
                .strong(),
            );
            ui.label(&c.text);
            chip(ui, c.proposal.label(), theme::proposal_colors(c.proposal));
        });
        if c.zone_a.is_none() {
            note(
                ui,
                &format!(
                    "Deze opmerking heeft geen plek op een tekening ({}). Beoordeel zelf.",
                    c.reason
                ),
            );
            return;
        }
        match &self.zone {
            Some((id, tex, ct)) if *id == i => {
                let w = (ui.available_width() - 30.0) / 4.0;
                let h = w * tex.a_ink.size()[1] as f32 / tex.a_ink.size()[0].max(1) as f32;
                ui.horizontal(|ui| {
                    let tile =
                        |ui: &mut egui::Ui, title: &str, f: &dyn Fn(&egui::Painter, egui::Rect)| {
                            ui.vertical(|ui| {
                                ui.label(RichText::new(title).size(11.5).color(theme::GRIJS));
                                let (rect, _) =
                                    ui.allocate_exact_size(Vec2::new(w, h), egui::Sense::hover());
                                let p = ui.painter_at(rect);
                                p.rect_filled(rect, 0.0, Color32::WHITE);
                                f(&p, rect);
                                p.rect_stroke(
                                    rect,
                                    0.0,
                                    egui::Stroke::new(1.0, theme::LIJN),
                                    egui::StrokeKind::Inside,
                                );
                            });
                        };
                    let uv = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(1.0, 1.0));
                    tile(ui, "Opmerkingen-PDF (aannemer)", &|p, r| {
                        if let Some(t) = ct {
                            p.image(t.id(), r, uv, Color32::WHITE);
                        }
                    });
                    tile(ui, "A", &|p, r| {
                        p.image(tex.a_ink.id(), r, uv, theme::INKT);
                    });
                    tile(ui, "B", &|p, r| {
                        p.image(tex.b_ink.id(), r, uv, theme::INKT);
                    });
                    tile(ui, "Verschil A -> B", &|p, r| {
                        p.image(tex.diff.id(), r, uv, Color32::WHITE);
                    });
                });
            }
            _ => match &self.zone_error {
                Some(e) => {
                    ui.label(
                        RichText::new(format!("Uitsnede niet te maken: {e}")).color(theme::ROOD),
                    );
                }
                None => {
                    ui.add(egui::Spinner::new());
                }
            },
        }
        note(
            ui,
            "De bladweergave staat op deze plek (tab b) voor A, B en de verschillaag in detail.",
        );
    }

    fn screen_export(&mut self, ui: &mut egui::Ui) {
        ui.add_space(12.0);
        ui.label(
            RichText::new("Export")
                .size(20.0)
                .strong()
                .color(theme::PETROL),
        );
        // tijdelijk uitnemen i.p.v. per frame de hele vergelijking te kopieren
        let Some(d) = self.diff.take() else {
            ui.label(RichText::new("Nog niets te exporteren.").color(theme::GRIJS));
            return;
        };
        ui.columns(2, |cols| {
            cols[0].group(|ui| {
                ui.label(RichText::new("Verschillen").strong().color(theme::PETROL));
                if ui.button("Rapport (markdown)").clicked() {
                    if let Some(p) = rfd::FileDialog::new()
                        .set_file_name("vergelijking.md")
                        .save_file()
                    {
                        self.result(
                            std::fs::write(&p, report::markdown(&d)).map_err(Into::into),
                            &p,
                        );
                    }
                }
                if ui.button("Lijst per blad (xlsx)").clicked() {
                    if let Some(p) = rfd::FileDialog::new()
                        .set_file_name("vergelijking.xlsx")
                        .save_file()
                    {
                        self.result(export::diff_xlsx(&d, &p), &p);
                    }
                }
                if ui.button("Lijst per blad (csv)").clicked() {
                    if let Some(p) = rfd::FileDialog::new()
                        .set_file_name("vergelijking.csv")
                        .save_file()
                    {
                        self.result(report::write_all(&d, None, None, Some(&p)), &p);
                    }
                }
                if ui.button("Revisiewolken op kopie van B (PDF)").clicked() {
                    if let (Some(dir), Some(ib)) =
                        (rfd::FileDialog::new().pick_folder(), &self.issue_b)
                    {
                        let r = export::clouds_on_b(&d, ib, &dir);
                        self.result_files(r);
                    }
                }
            });
            cols[1].group(|ui| {
                ui.label(RichText::new("Opmerkingen").strong().color(theme::PETROL));
                let Some(r) = self.comments.clone() else {
                    ui.label(RichText::new("geen opmerkingen geladen").color(theme::GRIJS));
                    return;
                };
                if ui.button("Opmerkingenlijst (xlsx)").clicked() {
                    if let Some(p) = rfd::FileDialog::new()
                        .set_file_name("opmerkingen.xlsx")
                        .save_file()
                    {
                        self.result(export::comments_xlsx(&r, &self.review, &p), &p);
                    }
                }
                if ui.button("Opmerkingenlijst (csv)").clicked() {
                    if let Some(p) = rfd::FileDialog::new()
                        .set_file_name("opmerkingen.csv")
                        .save_file()
                    {
                        self.result(
                            std::fs::write(&p, export::comments_csv(&r, &self.review))
                                .map_err(Into::into),
                            &p,
                        );
                    }
                }
                if ui.button("PDF van B met opmerkingen en status").clicked() {
                    if let (Some(dir), Some(ib)) =
                        (rfd::FileDialog::new().pick_folder(), &self.issue_b)
                    {
                        let missing = export::comments_without_place(&r);
                        let res = export::comments_on_b(&r, &self.review, ib, &dir);
                                                self.result_files(res);
                        if !missing.is_empty() {
                            if let Some((m, _)) = &mut self.message {
                                m.push_str(&format!(" - let op: opmerkingen zonder plek op B staan niet in de PDF (wel in xlsx/csv): {missing:?}"));
                            }
                        }
                    }
                }
                if let Some(p) = &self.review_path {
                    ui.label(
                        RichText::new(format!(
                            "Status wordt automatisch opgeslagen in {}",
                            p.display()
                        ))
                        .size(11.0)
                        .color(theme::GRIJS),
                    );
                }
            });
        });
        if let Some(r) = &self.comments {
            ui.add_space(10.0);
            ui.label(
                RichText::new("Voorbeeld opmerkingenlijst")
                    .strong()
                    .color(theme::PETROL),
            );
            egui::ScrollArea::vertical().show(ui, |ui| {
                egui::Grid::new("exp")
                    .striped(true)
                    .num_columns(6)
                    .show(ui, |ui| {
                        for h in [
                            "Nr",
                            "Blad",
                            "Tekst aannemer",
                            "Voorstel",
                            "Status",
                            "Notitie",
                        ] {
                            ui.label(RichText::new(h).strong().size(11.5).color(theme::GRIJS));
                        }
                        ui.end_row();
                        for c in &r.comments {
                            let e = self.review.entry(&c.id);
                            ui.label(c.number.to_string());
                            ui.label(c.sheet.clone().unwrap_or_else(|| "algemeen".into()));
                            ui.label(c.text.replace('\n', " "));
                            chip(ui, c.proposal.label(), theme::proposal_colors(c.proposal));
                            ui.label(e.status.label());
                            ui.label(e.note);
                            ui.end_row();
                        }
                    });
            });
        }
        self.diff = Some(d);
    }

    fn result(&mut self, r: anyhow::Result<()>, p: &std::path::Path) {
        match r {
            Ok(()) => self.message = Some((format!("geschreven: {}", p.display()), false)),
            Err(e) => self.message = Some((format!("{e:#}"), true)),
        }
    }
    fn result_files(&mut self, r: anyhow::Result<Vec<PathBuf>>) {
        match r {
            Ok(v) => {
                self.message = Some((
                    format!(
                        "geschreven: {}",
                        v.iter()
                            .map(|p| p.display().to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    false,
                ))
            }
            Err(e) => self.message = Some((format!("{e:#}"), true)),
        }
    }

    fn keys(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let (space, n, p, pgdn, pgup, f) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::Space),
                i.key_pressed(egui::Key::N),
                i.key_pressed(egui::Key::P),
                i.key_pressed(egui::Key::PageDown),
                i.key_pressed(egui::Key::PageUp),
                i.key_pressed(egui::Key::F),
            )
        });
        if self.screen == Screen::View {
            if space {
                if self.viewer.mode != Mode::Blink {
                    self.viewer.mode = Mode::Blink;
                    self.viewer.blink_auto = false;
                }
                self.viewer.blink_b = !self.viewer.blink_b;
            }
            if f {
                self.viewer.fit_pending = true;
            }
        }
        if matches!(self.screen, Screen::View | Screen::Diffs) {
            if n {
                self.step_region(1);
            }
            if p {
                self.step_region(-1);
            }
        }
        if pgdn || pgup {
            if let (Some(d), Some(cur)) = (&self.diff, self.sel_sheet) {
                let n = d.sheets.len();
                let next = if pgdn {
                    (cur + 1).min(n - 1)
                } else {
                    cur.saturating_sub(1)
                };
                self.open_sheet(next);
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_jobs();
        self.poll_render(&ctx);
        self.poll_sheet_job();
        self.flush_status(&ctx);
        self.keys(&ctx);
        self.auto_step(&ctx);
        self.titlebar(ui);
        self.statusbar(ui);
        self.left_panel(ui);
        self.right_panel(ui);
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(Color32::WHITE)
                    .inner_margin(egui::Margin::symmetric(14, 6)),
            )
            .show(ui, |ui| {
                if let Some((m, err)) = self.message.clone() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&m).color(if err {
                            theme::ROOD
                        } else {
                            theme::PETROL
                        }));
                        if ui.small_button("x").clicked() {
                            self.message = None;
                        }
                    });
                }
                match self.screen {
                    Screen::Start => self.screen_start(ui),
                    Screen::View => self.screen_view(ui),
                    Screen::Diffs => self.screen_diffs(ui),
                    Screen::Comments => self.screen_comments(ui),
                    Screen::Export => self.screen_export(ui),
                }
            });
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(
            storage,
            "sheetdiff",
            &Saved {
                a: self.path_a.clone(),
                b: self.path_b.clone(),
                c: self.path_c.clone(),
                cfg: Some(self.cfg.clone()),
                workers: self.workers,
            },
        );
    }
}

fn status_color(s: Status, p: comments::Proposal) -> Color32 {
    match s {
        Status::Done => theme::PETROL,
        Status::NotDone => theme::ROOD,
        Status::Question => theme::OKER,
        Status::NotApplicable => theme::GRIJS,
        Status::Open => theme::proposal_colors(p).1,
    }
}

fn text_label(t: &sheetdiff_core::textdiff::TextChange) -> String {
    match t.kind {
        TextChangeKind::Changed => format!(
            "{} -> {}",
            t.old.as_deref().unwrap_or(""),
            t.new.as_deref().unwrap_or("")
        ),
        TextChangeKind::Added => format!("+ {}", t.new.as_deref().unwrap_or("")),
        TextChangeKind::Removed => format!("- {}", t.old.as_deref().unwrap_or("")),
        TextChangeKind::Moved => format!(
            "{} (verplaatst {:.0} mm)",
            t.old.as_deref().unwrap_or(""),
            t.moved_mm.unwrap_or(0.0)
        ),
        TextChangeKind::Stamp => String::new(),
    }
}

fn note(ui: &mut egui::Ui, text: &str) {
    egui::Frame::new()
        .fill(theme::VLAK)
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_max_width(900.0);
            ui.label(RichText::new(text).color(theme::INKT).size(12.0));
        });
}

fn legend(ui: &mut egui::Ui) {
    for (c, l) in [
        (theme::C_REMOVED, "alleen A"),
        (theme::C_ADDED, "alleen B"),
        (Color32::from_rgb(70, 70, 70), "gelijk"),
    ] {
        let (r, _) = ui.allocate_exact_size(Vec2::new(10.0, 10.0), egui::Sense::hover());
        ui.painter().rect_filled(r, 2.0, c);
        ui.label(RichText::new(l).size(11.5).color(theme::GRIJS));
    }
}

#[allow(dead_code)]
fn unused(_: RectPt) {}

/// UTC-tijd als "JJJJ-MM-DDTUU:MM:SSZ" zonder extra afhankelijkheid.
fn now_iso() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // civil_from_days (H. Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}
