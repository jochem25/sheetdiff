//! insta-snapshots van de rapportvormen en de opmerkingengroepering. Deterministische invoer,
//! geen PDF's nodig. Bij een bewuste wijziging: `cargo insta review` (of INSTA_UPDATE=always).

use sheetdiff_core::comments::{group, RawAnnot};
use sheetdiff_core::compare::{IssueDiff, Region, SheetDiff, SheetLoc, Verdict};
use sheetdiff_core::geom::RectPt;
use sheetdiff_core::register::Registration;
use sheetdiff_core::report;
use sheetdiff_core::textdiff::{TextChange, TextChangeKind};
use sheetdiff_core::Config;
use std::path::PathBuf;

fn sheet(key: &str, verdict: Verdict) -> SheetDiff {
    SheetDiff {
        key: key.into(),
        title: format!("titel {key}"),
        verdict,
        reasons: vec![],
        a: Some(SheetLoc {
            file: PathBuf::from("A/boekje.pdf"),
            page: 0,
            booklets: vec!["gevels".into()],
        }),
        b: Some(SheetLoc {
            file: PathBuf::from("B/boekje.pdf"),
            page: 0,
            booklets: vec!["gevels".into()],
        }),
        registration: Some(Registration::identity_with_score(1.0)),
        dpi: 300.0,
        removed_px: 0,
        added_px: 0,
        pixel_pct: 0.0,
        regions: vec![],
        text: vec![],
        seconds: 1.0,
        vector: None,
        mask: None,
    }
}

fn sample() -> IssueDiff {
    let mut changed = sheet("TO-201n", Verdict::Changed);
    changed.removed_px = 120;
    changed.added_px = 3400;
    changed.pixel_pct = 0.0123;
    changed.regions = vec![Region {
        bbox: RectPt::new(100.0, 200.0, 180.0, 220.0),
        removed_px: 120,
        added_px: 3400,
        stamp: false,
        moved_mm: None,
        by_vector: true,
        text: vec![0, 1],
    }];
    changed.text = vec![
        TextChange {
            kind: TextChangeKind::Changed,
            old: Some("62,4 m²".into()),
            new: Some("61,8 m²".into()),
            bbox: RectPt::new(100.0, 200.0, 130.0, 208.0),
            moved_mm: None,
        },
        TextChange {
            kind: TextChangeKind::Added,
            old: None,
            new: Some("-87".into()),
            bbox: RectPt::new(150.0, 210.0, 170.0, 218.0),
            moved_mm: None,
        },
        TextChange {
            kind: TextChangeKind::Stamp,
            old: Some("24-6-2026 10:32:33".into()),
            new: Some("31-8-2026 9:01:02".into()),
            bbox: RectPt::new(3000.0, 2300.0, 3100.0, 2310.0),
            moved_mm: None,
        },
    ];
    let mut nc = sheet("TO-213z", Verdict::NotComparable);
    nc.reasons = vec![
        "UITLIJNING MISLUKT: overlap 41.0 % < 60 %. Vergelijking op dit blad is niet betrouwbaar."
            .into(),
    ];
    IssueDiff {
        a_root: PathBuf::from("A"),
        b_root: PathBuf::from("B"),
        sheets: vec![
            nc,
            changed,
            sheet("TO-214z", Verdict::StampOnly),
            sheet("TO-215z", Verdict::Identical),
        ],
        booklet_changes: vec![("fragmenten".into(), vec![], vec!["TO-523".into()])],
        warnings: vec!["TO-100z: twee versies met ANDERE inhoud".into()],
        config: Config::default(),
    }
}

#[test]
fn report_markdown() {
    insta::assert_snapshot!(report::markdown(&sample()));
}

#[test]
fn report_csv() {
    insta::assert_snapshot!(report::csv(&sample()));
}

#[test]
fn exit_code_not_comparable_wins() {
    assert_eq!(sample().exit_code(), 2);
}

#[test]
fn comment_grouping() {
    let ann = |page,
               sub: &str,
               r: (f64, f64, f64, f64),
               text: &str,
               subj: &str,
               author: &str,
               nm: &str| RawAnnot {
        page,
        subtype: sub.into(),
        rect: RectPt::new(r.0, r.1, r.2, r.3),
        contents: text.into(),
        subject: subj.into(),
        author: author.into(),
        date: "D:20260617085132+02'00'".into(),
        nm: nm.into(),
        in_reply_to: None,
        reply_type: None,
        state: None,
        object: (0, 0),
    };
    let annots = vec![
        ann(
            0,
            "FreeText",
            (40.0, 166.0, 699.0, 271.0),
            "algemene opmerkingen",
            "tekstvak",
            "A. Aannemer",
            "A1",
        ),
        ann(
            1,
            "Square",
            (2780.0, 1268.0, 3304.0, 1473.0),
            "",
            "Rechthoek",
            "A. Aannemer",
            "A2",
        ),
        ann(
            1,
            "PolyLine",
            (2497.0, 338.0, 2784.0, 1305.0),
            "",
            "Polylijn",
            "A. Aannemer",
            "A3",
        ),
        ann(
            1,
            "FreeText",
            (2515.0, 343.0, 2625.0, 387.0),
            "tekst opnemen in algemeen renvooi",
            "tekstvak blauw",
            "A. Aannemer",
            "A4",
        ),
        ann(
            1,
            "Ink",
            (2490.0, 259.0, 3058.0, 1487.0),
            "",
            "Pencil",
            "Jochem",
            "A5",
        ),
        ann(
            7,
            "PolyLine",
            (1032.0, 2348.0, 1081.0, 2387.0),
            "",
            "check akkoord",
            "A. Aannemer",
            "A6",
        ),
    ];
    let g = group(&annots, &Config::default());
    let view: Vec<_> = g
        .iter()
        .map(|c| {
            format!(
                "#{} p{} {:?} '{}' delen={} eigen={:?}",
                c.number,
                c.c_page + 1,
                c.kind,
                c.text,
                c.parts.len(),
                c.own_marks
            )
        })
        .collect();
    insta::assert_snapshot!(view.join("\n"));
}
