//! Rapporten: markdown (mens), json (machine), csv (Excel).

use crate::compare::{IssueDiff, SheetDiff, Verdict};
use crate::geom::MM_PER_PT;
use crate::textdiff::TextChangeKind;
use anyhow::Result;
use std::fmt::Write as _;
use std::path::Path;

fn kind_label(k: TextChangeKind) -> &'static str {
    match k {
        TextChangeKind::Changed => "gewijzigd",
        TextChangeKind::Added => "toegevoegd",
        TextChangeKind::Removed => "verwijderd",
        TextChangeKind::Moved => "verplaatst",
        TextChangeKind::Stamp => "printstempel",
    }
}

pub fn sheet_markdown(s: &SheetDiff, out: &mut String) {
    let _ = writeln!(
        out,
        "### {} {} - **{}**",
        s.key,
        if s.title.is_empty() {
            String::new()
        } else {
            format!("({})", s.title)
        },
        s.verdict.label()
    );
    for r in &s.reasons {
        let _ = writeln!(out, "- {r}");
    }
    if matches!(s.verdict, Verdict::Changed | Verdict::StampOnly) {
        let _ = writeln!(
            out,
            "- pixels: {:.4} % ({} weg, {} erbij) op {} dpi, {} regio's ({} op printstempel)",
            s.pixel_pct,
            s.removed_px,
            s.added_px,
            s.dpi,
            s.regions.len(),
            s.regions.iter().filter(|r| r.stamp).count()
        );
    }
    let mut by_kind: std::collections::BTreeMap<TextChangeKind, Vec<String>> = Default::default();
    for t in &s.text {
        let txt = match (t.kind, &t.old, &t.new) {
            (TextChangeKind::Changed | TextChangeKind::Stamp, Some(o), Some(n)) => {
                format!("`{o}` -> `{n}`")
            }
            (TextChangeKind::Moved, Some(o), _) => {
                format!("`{o}` ({:.1} mm)", t.moved_mm.unwrap_or(0.0))
            }
            (_, Some(o), None) => format!("`{o}`"),
            (_, None, Some(n)) => format!("`{n}`"),
            _ => String::new(),
        };
        by_kind.entry(t.kind).or_default().push(txt);
    }
    for (k, v) in by_kind {
        if k == TextChangeKind::Stamp {
            let _ = writeln!(out, "- printstempel: {} regel(s)", v.len());
            continue;
        }
        let shown: Vec<&String> = v.iter().take(12).collect();
        let more = if v.len() > 12 {
            format!(" ... (+{})", v.len() - 12)
        } else {
            String::new()
        };
        let _ = writeln!(
            out,
            "- tekst {} ({}): {}{}",
            kind_label(k),
            v.len(),
            shown
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            more
        );
    }
    let _ = writeln!(out);
}

pub fn markdown(d: &IssueDiff) -> String {
    let mut o = String::new();
    let _ = writeln!(o, "# Vergelijking tekeninguitgiftes\n");
    let _ = writeln!(o, "- A: `{}`", d.a_root.display());
    let _ = writeln!(o, "- B: `{}`", d.b_root.display());
    let _ = writeln!(
        o,
        "- analyse: {} dpi, tolerantie {} grijswaarden, lijnafstand {} px\n",
        d.config.compare.dpi, d.config.compare.diff.tolerance, d.config.compare.diff.radius_px
    );
    let _ = writeln!(o, "| oordeel | aantal |\n|---|---|");
    for v in [
        Verdict::NotComparable,
        Verdict::Changed,
        Verdict::AddedInB,
        Verdict::RemovedFromB,
        Verdict::MovedOnly,
        Verdict::StampOnly,
        Verdict::Identical,
    ] {
        let _ = writeln!(o, "| {} | {} |", v.label(), d.count(v));
    }
    let _ = writeln!(o);
    if d.count(Verdict::NotComparable) > 0 {
        let _ = writeln!(
            o,
            "> **LET OP:** {} blad(en) konden niet betrouwbaar vergeleken worden. \
            Die bladen zijn NIET gecontroleerd.\n",
            d.count(Verdict::NotComparable)
        );
    }
    if !d.warnings.is_empty() {
        let _ = writeln!(o, "## Waarschuwingen\n");
        for w in &d.warnings {
            let _ = writeln!(o, "- {w}");
        }
        let _ = writeln!(o);
    }
    if !d.booklet_changes.is_empty() {
        let _ = writeln!(o, "## Boekjessamenstelling\n");
        for (b, oa, ob) in &d.booklet_changes {
            let _ = writeln!(o, "- {b}: alleen in A {:?}, alleen in B {:?}", oa, ob);
        }
        let _ = writeln!(o);
    }
    let _ = writeln!(o, "## Per blad\n");
    for s in d.sheets.iter().filter(|s| s.verdict != Verdict::Identical) {
        sheet_markdown(s, &mut o);
    }
    let ident: Vec<&str> = d
        .sheets
        .iter()
        .filter(|s| s.verdict == Verdict::Identical)
        .map(|s| s.key.as_str())
        .collect();
    if !ident.is_empty() {
        let _ = writeln!(o, "### Gelijk ({})\n\n{}\n", ident.len(), ident.join(", "));
    }
    o
}

fn csv_field(s: &str) -> String {
    if s.contains([';', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// CSV met puntkomma (Nederlandse Excel), een regel per blad.
pub fn csv(d: &IssueDiff) -> String {
    let mut o = String::from(
        "blad;titel;oordeel;pixel_pct;regios;tekst_gewijzigd;tekst_toegevoegd;tekst_verwijderd;tekst_verplaatst;uitlijning;boekjes;redenen\n",
    );
    for s in &d.sheets {
        let n = |k| s.text.iter().filter(|t| t.kind == k).count();
        let booklets =
            s.b.as_ref()
                .or(s.a.as_ref())
                .map(|l| l.booklets.join(", "))
                .unwrap_or_default();
        let reg = s
            .registration
            .as_ref()
            .map(|r| r.note.clone())
            .unwrap_or_default();
        let _ = writeln!(
            o,
            "{};{};{};{};{};{};{};{};{};{};{};{}",
            csv_field(&s.key),
            csv_field(&s.title),
            s.verdict.label(),
            format!("{:.4}", s.pixel_pct).replace('.', ","),
            s.regions.iter().filter(|r| !r.stamp).count(),
            n(TextChangeKind::Changed),
            n(TextChangeKind::Added),
            n(TextChangeKind::Removed),
            n(TextChangeKind::Moved),
            csv_field(&reg),
            csv_field(&booklets),
            csv_field(&s.reasons.join(" | ")),
        );
    }
    o
}

pub fn write_all(
    d: &IssueDiff,
    md: Option<&Path>,
    json: Option<&Path>,
    csv_p: Option<&Path>,
) -> Result<()> {
    if let Some(p) = md {
        std::fs::write(p, markdown(d))?;
    }
    if let Some(p) = json {
        std::fs::write(p, serde_json::to_string_pretty(d)?)?;
    }
    if let Some(p) = csv_p {
        // BOM zodat Excel UTF-8 herkent
        let mut s = String::from("\u{feff}");
        s.push_str(&csv(d));
        std::fs::write(p, s)?;
    }
    Ok(())
}

pub fn mm(v: f64) -> f64 {
    v * MM_PER_PT
}
