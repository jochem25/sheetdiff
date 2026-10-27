//! KBA-huisstijl voor de app. Bron: KBA-huisstijl (kba-app-theme.css)
//! (kleuren overgenomen als constanten; namen gelijk aan de --theme-* tokens).

use eframe::egui::{self, Color32, CornerRadius, Stroke, Visuals};

pub const PETROL: Color32 = Color32::from_rgb(0x0D, 0x68, 0x62);
pub const TEAL: Color32 = Color32::from_rgb(0x0F, 0xA0, 0x8D);
#[allow(dead_code)] // palet compleet houden
pub const MINT: Color32 = Color32::from_rgb(0xB0, 0xEA, 0xE2);
pub const INKT: Color32 = Color32::from_rgb(0x0F, 0x30, 0x2C);
pub const GRIJS: Color32 = Color32::from_rgb(0x6B, 0x79, 0x75);
pub const ROOD: Color32 = Color32::from_rgb(0xA6, 0x34, 0x2B);
pub const OKER: Color32 = Color32::from_rgb(0x8F, 0x6A, 0x1F);
pub const LIJN: Color32 = Color32::from_rgb(0xDF, 0xE7, 0xE4);
pub const VLAK: Color32 = Color32::from_rgb(0xF4, 0xF8, 0xF7);
pub const TITEL: Color32 = Color32::from_rgb(0xE7, 0xF2, 0xEF);
pub const STATUS_TEKST: Color32 = Color32::from_rgb(0xD8, 0xEE, 0xE9);
pub const HOVER: Color32 = Color32::from_rgb(0xDF, 0xF5, 0xF1); // mint 40 % op wit
pub const ACTIEF: Color32 = Color32::from_rgb(0xD8, 0xF4, 0xF0); // mint 50 % op wit
pub const CANVAS: Color32 = Color32::from_rgb(0xEE, 0xF3, 0xF1);

// diffkleuren (overlay): alleen A = rood, alleen B = cyaan
pub const C_REMOVED: Color32 = Color32::from_rgb(220, 30, 30);
pub const C_ADDED: Color32 = Color32::from_rgb(0, 170, 200);

pub fn apply(ctx: &egui::Context) {
    let mut v = Visuals::light();
    v.panel_fill = Color32::WHITE;
    v.window_fill = Color32::WHITE;
    v.extreme_bg_color = Color32::WHITE;
    v.faint_bg_color = VLAK;
    v.override_text_color = Some(INKT);
    v.hyperlink_color = TEAL;
    v.selection.bg_fill = ACTIEF;
    v.selection.stroke = Stroke::new(1.0, PETROL);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LIJN);
    // bg_fill is o.a. de rail van een schuifregelaar en de achtergrond van een vinkje:
    // niet wit, anders onzichtbaar op wit
    v.widgets.inactive.bg_fill = LIJN;
    v.slider_trailing_fill = true;
    v.widgets.inactive.weak_bg_fill = Color32::WHITE;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, LIJN);
    v.widgets.hovered.bg_fill = HOVER;
    v.widgets.hovered.weak_bg_fill = HOVER;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, TEAL);
    v.widgets.active.bg_fill = ACTIEF;
    v.widgets.active.weak_bg_fill = ACTIEF;
    v.widgets.active.bg_stroke = Stroke::new(1.0, PETROL);
    for w in [
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
    ] {
        w.corner_radius = CornerRadius::same(3);
    }
    ctx.set_visuals_of(egui::Theme::Light, v);
    ctx.set_theme(egui::Theme::Light);
    ctx.style_mut_of(egui::Theme::Light, |s| {
        s.spacing.item_spacing = egui::vec2(6.0, 5.0);
        s.spacing.button_padding = egui::vec2(10.0, 4.0);
    });
}

/// Chip-kleuren (achtergrond, tekst) per oordeel.
pub fn verdict_colors(v: sheetdiff_core::compare::Verdict) -> (Color32, Color32) {
    use sheetdiff_core::compare::Verdict::*;
    match v {
        Changed => (Color32::from_rgb(0xFC, 0xE9, 0xE7), ROOD),
        StampOnly => (VLAK, GRIJS),
        Identical => (Color32::from_rgb(0xE8, 0xF6, 0xF3), PETROL),
        NotComparable => (OKER, Color32::WHITE),
        AddedInB | RemovedFromB => (
            Color32::from_rgb(0xE6, 0xF6, 0xFA),
            Color32::from_rgb(0x00, 0x7C, 0x93),
        ),
        MovedOnly => (Color32::from_rgb(0xF6, 0xEF, 0xDF), OKER),
    }
}

pub fn proposal_colors(p: sheetdiff_core::comments::Proposal) -> (Color32, Color32) {
    use sheetdiff_core::comments::Proposal::*;
    match p {
        LikelyDone => (Color32::from_rgb(0xE8, 0xF6, 0xF3), PETROL),
        NotDone => (Color32::from_rgb(0xFC, 0xE9, 0xE7), ROOD),
        Unclear => (Color32::from_rgb(0xF6, 0xEF, 0xDF), OKER),
        Info => (VLAK, GRIJS),
    }
}

/// Kleine afgeronde chip met tekst.
pub fn chip(ui: &mut egui::Ui, text: &str, (bg, fg): (Color32, Color32)) -> egui::Response {
    let galley =
        ui.painter()
            .layout_no_wrap(text.to_string(), egui::FontId::proportional(11.0), fg);
    let size = galley.size() + egui::vec2(12.0, 4.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(8), bg);
    ui.painter()
        .galley(rect.min + egui::vec2(6.0, 2.0), galley, fg);
    resp
}
