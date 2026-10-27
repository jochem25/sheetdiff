//! Bladweergave: overlay, knipperen, naast elkaar, pixelverschil, vectorverschil.
//! Coordinaten: A-weergavepunten (pt). Scherm = (pt - pan) * zoom + oorsprong paneel.

use crate::render::Layer;
use crate::theme;
use eframe::egui::{
    self, Color32, ColorImage, Pos2, Rect, Sense, Stroke, StrokeKind, TextureHandle,
    TextureOptions, Vec2,
};
use rayon::prelude::*;
use sheetdiff_core::compare::SheetDiff;
use sheetdiff_core::geom::{RectPt, MM_PER_PT};
use sheetdiff_core::raster::{ADDED, REMOVED};
use sheetdiff_core::textdiff::TextChangeKind;
use sheetdiff_core::vector::{VChange, VChangeKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Overlay,
    Blink,
    Side,
    Pixel,
    Vector,
}

pub struct LayerTex {
    pub rect_pt: RectPt,
    pub dpi: f64,
    /// Brongegevens, voor het opnieuw samenstellen van de overlay bij een andere dekking.
    pub layer: std::sync::Arc<Layer>,
    pub overlay: Option<(f32, TextureHandle)>,
    pub a_ink: TextureHandle,
    pub b_ink: TextureHandle,
    pub diff: TextureHandle,
}

/// Een gemarkeerde opmerking op het blad.
pub struct CommentMark {
    pub number: usize,
    pub zone: RectPt,
    pub color: Color32,
    pub selected: bool,
}

pub struct Viewer {
    pub mode: Mode,
    pub mix: f32,
    pub blink_b: bool,
    pub show_regions: bool,
    pub show_comments: bool,
    pub zoom: f32, // schermpixels per pt
    pub pan: Vec2, // pt in linkerbovenhoek
    pub base: Option<LayerTex>,
    pub detail: Option<LayerTex>,
    pub vector: Option<Vec<VChange>>,
    pub size_pt: (f64, f64),
    /// Renderfout voor dit blad: wordt groot over het blad getoond, nooit stil.
    pub error: Option<String>,
    pub fit_pending: bool,
    pub last_change: f64,
    pub detail_requested: Option<(RectPt, f64)>,
    pub panel_rect: Rect,
    pub last_blink: f64,
    pub blink_auto: bool,
}

impl Default for Viewer {
    fn default() -> Self {
        Self {
            mode: Mode::Overlay,
            mix: 0.5,
            blink_b: false,
            show_regions: true,
            show_comments: true,
            zoom: 0.3,
            pan: Vec2::ZERO,
            base: None,
            detail: None,
            vector: None,
            size_pt: (1.0, 1.0),
            error: None,
            fit_pending: true,
            last_change: 0.0,
            detail_requested: None,
            panel_rect: Rect::NOTHING,
            last_blink: 0.0,
            blink_auto: false,
        }
    }
}

/// Bouwt de drie texturen van een laag: inkt A, inkt B (wit met alfa = inkt) en het
/// pixelverschil (rood/cyaan/grijs).
pub fn make_tex(ctx: &egui::Context, name: &str, l: Layer) -> LayerTex {
    let l = std::sync::Arc::new(l);
    let n = l.w * l.h;
    let ink = |g: &[u8]| -> Vec<u8> {
        let mut v = vec![0u8; n * 4];
        v.par_chunks_mut(4).zip(g.par_iter()).for_each(|(px, &gv)| {
            px.copy_from_slice(&[255, 255, 255, 255 - gv]);
        });
        v
    };
    let mut diff = vec![0u8; n * 4];
    diff.par_chunks_mut(4).enumerate().for_each(|(i, px)| {
        let (va, vb, c) = (l.a.data[i], l.b.data[i], l.class[i]);
        let lerp = |ink: u8, col: Color32, k: f32| -> [u8; 4] {
            let t = (255 - ink) as f32 / 255.0 * k;
            [
                (255.0 - t * (255.0 - col.r() as f32)) as u8,
                (255.0 - t * (255.0 - col.g() as f32)) as u8,
                (255.0 - t * (255.0 - col.b() as f32)) as u8,
                255,
            ]
        };
        let p = if c & REMOVED != 0 && (c & ADDED == 0 || va <= vb) {
            lerp(va, theme::C_REMOVED, 1.0)
        } else if c & ADDED != 0 {
            lerp(vb, theme::C_ADDED, 1.0)
        } else {
            lerp(va.min(vb), Color32::from_rgb(60, 60, 60), 0.55)
        };
        px.copy_from_slice(&p);
    });
    let opt = TextureOptions {
        magnification: egui::TextureFilter::Nearest,
        minification: egui::TextureFilter::Linear,
        ..Default::default()
    };
    let mk = |suffix: &str, data: &[u8]| {
        ctx.load_texture(
            format!("{name}-{suffix}"),
            ColorImage::from_rgba_unmultiplied([l.w, l.h], data),
            opt,
        )
    };
    LayerTex {
        overlay: None,
        rect_pt: l.rect_pt,
        dpi: l.dpi,
        a_ink: mk("a", &ink(&l.a.data)),
        b_ink: mk("b", &ink(&l.b.data)),
        diff: mk("diff", &diff),
        layer: l,
    }
}

/// Overlay als echte vermenigvuldiging: A in rood, B in cyaan; waar beide inkt hebben wordt
/// het donker (zoals in de mockup).
fn overlay_tex(ctx: &egui::Context, t: &mut LayerTex, mix: f32) {
    if t.overlay
        .as_ref()
        .is_some_and(|(m, _)| (m - mix).abs() < 0.005)
    {
        return;
    }
    let l = &t.layer;
    let ka = (2.0 * (1.0 - mix)).min(1.0);
    let kb = (2.0 * mix).min(1.0);
    let (ra, rb) = (theme::C_REMOVED, theme::C_ADDED);
    let mut out = vec![0u8; l.w * l.h * 4];
    out.par_chunks_mut(4).enumerate().for_each(|(i, px)| {
        let ta = (255 - l.a.data[i]) as f32 / 255.0 * ka;
        let tb = (255 - l.b.data[i]) as f32 / 255.0 * kb;
        let ch = |ca: u8, cb: u8| {
            let a = 255.0 - ta * (255.0 - ca as f32);
            let b = 255.0 - tb * (255.0 - cb as f32);
            (a * b / 255.0) as u8
        };
        px.copy_from_slice(&[
            ch(ra.r(), rb.r()),
            ch(ra.g(), rb.g()),
            ch(ra.b(), rb.b()),
            255,
        ]);
    });
    let img = ColorImage::from_rgba_unmultiplied([l.w, l.h], &out);
    let opt = TextureOptions {
        magnification: egui::TextureFilter::Nearest,
        minification: egui::TextureFilter::Linear,
        ..Default::default()
    };
    match &mut t.overlay {
        Some((m, h)) => {
            h.set(img, opt);
            *m = mix;
        }
        None => t.overlay = Some((mix, ctx.load_texture("overlay", img, opt))),
    }
}

impl Viewer {
    pub fn reset(&mut self, size_pt: (f64, f64)) {
        self.base = None;
        self.detail = None;
        self.vector = None;
        self.error = None;
        self.size_pt = size_pt;
        self.fit_pending = true;
        self.detail_requested = None;
    }

    fn to_screen(&self, origin: Pos2, x: f64, y: f64) -> Pos2 {
        origin
            + Vec2::new(
                (x as f32 - self.pan.x) * self.zoom,
                (y as f32 - self.pan.y) * self.zoom,
            )
    }
    fn rect_screen(&self, origin: Pos2, r: &RectPt) -> Rect {
        Rect::from_two_pos(
            self.to_screen(origin, r.x0, r.y0),
            self.to_screen(origin, r.x1, r.y1),
        )
    }

    pub fn fit(&mut self, pane: Vec2) {
        let (w, h) = self.size_pt;
        self.zoom = (pane.x / w as f32).min(pane.y / h as f32) * 0.96;
        self.pan = Vec2::new(
            w as f32 / 2.0 - pane.x / 2.0 / self.zoom,
            h as f32 / 2.0 - pane.y / 2.0 / self.zoom,
        );
    }

    /// Zoom zo dat `r` (pt) gevuld in beeld staat.
    pub fn zoom_to(&mut self, r: &RectPt, pad_pt: f64) {
        let pane = self.pane_size();
        let rr = r.expand(pad_pt);
        self.zoom = (pane.x / rr.width() as f32)
            .min(pane.y / rr.height() as f32)
            .min(30.0);
        let (cx, cy) = rr.center();
        self.pan = Vec2::new(
            cx as f32 - pane.x / 2.0 / self.zoom,
            cy as f32 - pane.y / 2.0 / self.zoom,
        );
    }

    pub fn pane_size(&self) -> Vec2 {
        let s = self.panel_rect.size();
        if self.mode == Mode::Side {
            Vec2::new(s.x / 2.0, s.y)
        } else {
            s
        }
    }

    /// Zichtbaar deel van het blad in pt (voor de detaillaag).
    pub fn visible_pt(&self) -> RectPt {
        let pane = self.pane_size();
        let r = RectPt::new(
            self.pan.x as f64,
            self.pan.y as f64,
            (self.pan.x + pane.x / self.zoom) as f64,
            (self.pan.y + pane.y / self.zoom) as f64,
        );
        RectPt::new(
            r.x0.max(0.0),
            r.y0.max(0.0),
            r.x1.min(self.size_pt.0),
            r.y1.min(self.size_pt.1),
        )
    }

    /// dpi op het scherm (fysieke pixels).
    pub fn screen_dpi(&self, ppp: f32) -> f64 {
        (self.zoom * ppp * 72.0) as f64
    }

    /// Tekent het blad. Geeft true terug als de weergave veranderde (pan/zoom).
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        diff: Option<&SheetDiff>,
        sel_region: Option<usize>,
        comments: &[CommentMark],
    ) -> (bool, Option<Pos2>) {
        let avail = ui.available_rect_before_wrap();
        let (resp, painter) = ui.allocate_painter(avail.size(), Sense::click_and_drag());
        let rect = resp.rect;
        self.panel_rect = rect;
        painter.rect_filled(rect, 0.0, theme::CANVAS);
        // passend maken tot het blad er is (panelen kunnen de eerste frames nog verspringen)
        if self.fit_pending && rect.width() > 10.0 {
            self.fit(self.pane_size());
            if self.base.is_some() {
                self.fit_pending = false;
            }
        }
        if self.mode == Mode::Overlay {
            let mix = self.mix;
            for t in [self.base.as_mut(), self.detail.as_mut()]
                .into_iter()
                .flatten()
            {
                overlay_tex(ui.ctx(), t, mix);
            }
        }
        let mut changed = false;
        // pannen en zoomen (wiel = zoom rond cursor, slepen = pannen)
        if resp.dragged() {
            self.pan -= resp.drag_delta() / self.zoom;
            changed = true;
        }
        let mut hover_pt = None;
        if let Some(pos) = resp.hover_pos() {
            let pane_origin = if self.mode == Mode::Side && pos.x > rect.center().x {
                Pos2::new(rect.center().x + 1.0, rect.min.y)
            } else {
                rect.min
            };
            let pt = Pos2::new(
                (pos.x - pane_origin.x) / self.zoom + self.pan.x,
                (pos.y - pane_origin.y) / self.zoom + self.pan.y,
            );
            hover_pt = Some(pt);
            let scroll = ui.input(|i| i.smooth_scroll_delta.y + i.zoom_delta().ln() * 300.0);
            if scroll.abs() > 0.1 {
                let f = (scroll * 0.0025).exp();
                let nz = (self.zoom * f).clamp(0.02, 40.0);
                self.pan = Vec2::new(
                    pt.x - (pos.x - pane_origin.x) / nz,
                    pt.y - (pos.y - pane_origin.y) / nz,
                );
                self.zoom = nz;
                changed = true;
            }
        }
        if changed {
            self.last_change = ui.input(|i| i.time);
        }
        let panes: Vec<(Rect, Pos2, u8)> = if self.mode == Mode::Side {
            let c = rect.center().x;
            vec![
                (
                    Rect::from_min_max(rect.min, Pos2::new(c - 1.0, rect.max.y)),
                    rect.min,
                    0,
                ),
                (
                    Rect::from_min_max(Pos2::new(c + 1.0, rect.min.y), rect.max),
                    Pos2::new(c + 1.0, rect.min.y),
                    1,
                ),
            ]
        } else {
            vec![(rect, rect.min, 2)]
        };
        for (pane, origin, which) in panes {
            let p = painter.with_clip_rect(pane);
            let page = self.rect_screen(
                origin,
                &RectPt::new(0.0, 0.0, self.size_pt.0, self.size_pt.1),
            );
            p.rect_filled(page, 0.0, Color32::WHITE);
            p.rect_stroke(
                page,
                0.0,
                Stroke::new(1.0, theme::LIJN),
                StrokeKind::Outside,
            );
            for layer in [self.base.as_ref(), self.detail.as_ref()]
                .into_iter()
                .flatten()
            {
                let r = self.rect_screen(origin, &layer.rect_pt);
                let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                if layer.dpi < 1.0 {
                    continue;
                }
                match (self.mode, which) {
                    (Mode::Side, 0) => {
                        p.rect_filled(r, 0.0, Color32::WHITE);
                        p.image(layer.a_ink.id(), r, uv, theme::INKT);
                    }
                    (Mode::Side, _) => {
                        p.rect_filled(r, 0.0, Color32::WHITE);
                        p.image(layer.b_ink.id(), r, uv, theme::INKT);
                    }
                    (Mode::Blink, _) => {
                        p.rect_filled(r, 0.0, Color32::WHITE);
                        let t = if self.blink_b {
                            &layer.b_ink
                        } else {
                            &layer.a_ink
                        };
                        p.image(t.id(), r, uv, theme::INKT);
                    }
                    (Mode::Overlay, _) => {
                        if let Some((_, t)) = &layer.overlay {
                            p.image(t.id(), r, uv, Color32::WHITE);
                        }
                    }
                    (Mode::Pixel, _) => {
                        p.image(layer.diff.id(), r, uv, Color32::WHITE);
                    }
                    (Mode::Vector, _) => {
                        // eerst wit, zodat de basislaag niet door de detaillaag heen schemert
                        p.rect_filled(r, 0.0, Color32::WHITE);
                        p.image(layer.diff.id(), r, uv, Color32::WHITE.gamma_multiply(0.35));
                    }
                }
            }
            if self.mode == Mode::Vector {
                self.draw_vector(&p, origin, diff);
            }
            if self.show_regions {
                if let Some(d) = diff {
                    for (i, r) in d.regions.iter().enumerate() {
                        let sr = self.rect_screen(origin, &r.bbox.expand(3.0));
                        let (col, w) = if r.stamp {
                            (theme::GRIJS, 1.0)
                        } else if r.moved_mm.is_some() {
                            (theme::OKER, 1.2)
                        } else {
                            (theme::TEAL, 1.4)
                        };
                        let w = if Some(i) == sel_region { 3.0 } else { w };
                        p.rect_stroke(sr, 1.0, Stroke::new(w, col), StrokeKind::Outside);
                    }
                }
            }
            if self.show_comments {
                for c in comments {
                    let sr = self.rect_screen(origin, &c.zone);
                    let st = Stroke::new(if c.selected { 3.0 } else { 1.5 }, c.color);
                    dashed_rect(&p, sr, st);
                    let at = sr.left_top() + Vec2::new(-2.0, -2.0);
                    p.circle_filled(at, 10.0, c.color);
                    p.text(
                        at,
                        egui::Align2::CENTER_CENTER,
                        c.number.to_string(),
                        egui::FontId::proportional(11.0),
                        Color32::WHITE,
                    );
                }
            }
            let label = match (self.mode, which) {
                (Mode::Side, 0) => "A",
                (Mode::Side, _) => "B (uitgelijnd op A)",
                (Mode::Blink, _) if self.blink_b => "B",
                (Mode::Blink, _) => "A",
                (Mode::Overlay, _) => "Overlay: A rood, B cyaan",
                (Mode::Pixel, _) => "Pixelverschil: rood = alleen A, cyaan = alleen B",
                (Mode::Vector, _) => "Vectorverschil: paden en tekst, scherp op elke zoom",
            };
            let lr = Rect::from_min_size(
                pane.min + Vec2::new(8.0, 8.0),
                Vec2::new(8.0 + 7.0 * label.len() as f32, 20.0),
            );
            p.rect_filled(lr, 3.0, Color32::from_white_alpha(230));
            p.text(
                lr.left_center() + Vec2::new(6.0, 0.0),
                egui::Align2::LEFT_CENTER,
                label,
                egui::FontId::proportional(12.0),
                theme::INKT,
            );
        }
        if self.mode == Mode::Side {
            painter.vline(
                rect.center().x,
                rect.y_range(),
                Stroke::new(2.0, theme::LIJN),
            );
        }
        if let Some(e) = &self.error {
            let r = Rect::from_center_size(rect.center(), Vec2::new(rect.width().min(720.0), 90.0));
            painter.rect_filled(r, 4.0, theme::OKER);
            painter.text(
                r.center(),
                egui::Align2::CENTER_CENTER,
                format!("Dit blad kan niet getoond worden:\n{e}"),
                egui::FontId::proportional(14.0),
                Color32::WHITE,
            );
        }
        (changed, hover_pt)
    }

    fn draw_vector(&self, p: &egui::Painter, origin: Pos2, diff: Option<&SheetDiff>) {
        let vis = self.visible_pt().expand(10.0);
        if let Some(v) = &self.vector {
            for c in v {
                if !c.bbox.intersects(&vis) {
                    continue;
                }
                let col = match c.kind {
                    VChangeKind::Removed => theme::C_REMOVED,
                    VChangeKind::Added => theme::C_ADDED,
                    VChangeKind::Style => theme::OKER,
                };
                let st = Stroke::new(2.0, col);
                for s in &c.segs {
                    p.line_segment(
                        [
                            self.to_screen(origin, s.x0 as f64, s.y0 as f64),
                            self.to_screen(origin, s.x1 as f64, s.y1 as f64),
                        ],
                        st,
                    );
                }
            }
        }
        if let Some(d) = diff {
            for t in &d.text {
                if t.kind == TextChangeKind::Stamp || !t.bbox.intersects(&vis) {
                    continue;
                }
                let col = match t.kind {
                    TextChangeKind::Removed => theme::C_REMOVED,
                    TextChangeKind::Added => theme::C_ADDED,
                    TextChangeKind::Moved => theme::OKER,
                    _ => theme::PETROL,
                };
                let r = self.rect_screen(origin, &t.bbox.expand(1.0));
                p.rect_filled(r, 1.0, col.gamma_multiply(0.12));
                p.rect_stroke(r, 1.0, Stroke::new(1.2, col), StrokeKind::Outside);
                let lab = match t.kind {
                    TextChangeKind::Changed => format!(
                        "{} → {}",
                        t.old.as_deref().unwrap_or(""),
                        t.new.as_deref().unwrap_or("")
                    ),
                    TextChangeKind::Added => format!("+ {}", t.new.as_deref().unwrap_or("")),
                    TextChangeKind::Removed => format!("− {}", t.old.as_deref().unwrap_or("")),
                    _ => format!(
                        "⇢ {} ({:.0} mm)",
                        t.old.as_deref().unwrap_or(""),
                        t.moved_mm.unwrap_or(0.0)
                    ),
                };
                // labels pas bij inzoomen, anders onleesbare drukte
                if self.zoom < 0.8 {
                    continue;
                }
                let fs = (7.0 * self.zoom).clamp(9.0, 16.0);
                p.text(
                    r.left_top() - Vec2::new(0.0, 2.0),
                    egui::Align2::LEFT_BOTTOM,
                    lab,
                    egui::FontId::proportional(fs),
                    col,
                );
            }
        }
    }

    pub fn hover_mm(pt: Pos2) -> (f64, f64) {
        (pt.x as f64 * MM_PER_PT, pt.y as f64 * MM_PER_PT)
    }
}

fn dashed_rect(p: &egui::Painter, r: Rect, st: Stroke) {
    let pts = [
        r.left_top(),
        r.right_top(),
        r.right_bottom(),
        r.left_bottom(),
        r.left_top(),
    ];
    for w in pts.windows(2) {
        p.extend(egui::Shape::dashed_line(&[w[0], w[1]], st, 8.0, 5.0));
    }
}
