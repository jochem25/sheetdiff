//! Golden-tests op echte projectdata van een referentieproject. Die data staat buiten de repo;
//! de paden komen uit omgevingsvariabelen (zie docs/testdata.md), bijvoorbeeld via een lokale,
//! niet-gecommitte `.cargo/config.toml` met een `[env]`-sectie.
//! Draaien met `cargo test --release -- --ignored`. Ontbrekende data is een FOUT, geen stille
//! groene test.

use sheetdiff_core::comments::{self, Proposal};
use sheetdiff_core::compare::{self, Verdict};
use sheetdiff_core::issue;
use sheetdiff_core::Config;
use std::path::PathBuf;

/// Pad uit een verplichte omgevingsvariabele.
fn env_path(key: &str) -> PathBuf {
    let v = std::env::var(key)
        .unwrap_or_else(|_| panic!("omgevingsvariabele {key} ontbreekt (zie docs/testdata.md)"));
    PathBuf::from(v)
}

fn need(p: &std::path::Path) -> PathBuf {
    assert!(
        p.exists(),
        "testdata ontbreekt: {} (zie docs/testdata.md)",
        p.display()
    );
    p.to_path_buf()
}

/// Uitgifte A (map met boekjes).
fn issue_a() -> PathBuf {
    env_path("SHEETDIFF_GOLDEN_ISSUE_A")
}

/// Uitgifte B (map met boekjes).
fn issue_b() -> PathBuf {
    env_path("SHEETDIFF_GOLDEN_ISSUE_B")
}

/// Gevels-paar oud/nieuw: `-87` toegevoegd op TO-201n t/m TO-204n, niets vervallen.
#[test]
#[ignore]
fn golden_gevels_noord_87() {
    let a = need(&env_path("SHEETDIFF_GOLDEN_PAIR_OLD"));
    let b = need(&env_path("SHEETDIFF_GOLDEN_PAIR_NEW"));
    let cfg = Config::default();
    let (ia, ib) = (
        issue::scan_issue(&a, &cfg).unwrap(),
        issue::scan_issue(&b, &cfg).unwrap(),
    );
    let d = compare::compare_issues(&ia, &ib, &cfg, |_| {}).unwrap();
    for nr in ["TO-201n", "TO-202n", "TO-203n", "TO-204n"] {
        let s = d
            .sheets
            .iter()
            .find(|s| s.key == nr)
            .unwrap_or_else(|| panic!("{nr} ontbreekt"));
        assert_eq!(s.verdict, Verdict::Changed, "{nr}");
        assert!(
            s.text.iter().any(|t| t.new.as_deref() == Some("-87")),
            "{nr}: '-87' niet als nieuwe tekst gevonden"
        );
    }
    assert_eq!(d.count(Verdict::RemovedFromB), 0);
    assert_eq!(d.count(Verdict::NotComparable), 0);
}

/// Zelfde print, ander boekje: alles gelijk (0 valse verschillen).
#[test]
#[ignore]
fn golden_same_print_no_false_diffs() {
    let b = issue_b();
    let a = need(&b.join("TO+_wijzA_gevels.pdf"));
    let n = need(&b.join("TO+_wijzA_gevels-noord.pdf"));
    let cfg = Config::default();
    let (ia, ib) = (
        issue::scan_issue(&a, &cfg).unwrap(),
        issue::scan_issue(&n, &cfg).unwrap(),
    );
    let d = compare::compare_issues(&ia, &ib, &cfg, |_| {}).unwrap();
    let paired: Vec<_> = d
        .sheets
        .iter()
        .filter(|s| s.a.is_some() && s.b.is_some())
        .collect();
    assert_eq!(paired.len(), 4);
    for s in paired {
        assert_eq!(s.verdict, Verdict::Identical, "{}: {:?}", s.key, s.reasons);
    }
}

/// Synthetisch opmerkingenbestand met bekende uitkomst (docs/testdata.md).
#[test]
#[ignore]
fn golden_synthetic_comments() {
    let c = need(&env_path("SHEETDIFF_TESTDATA").join("synthetisch_opm_TO-201n.pdf"));
    let a = need(&issue_a().join("TO+_wijzA_gevels.pdf"));
    let b = need(&issue_b().join("TO+_wijzA_gevels.pdf"));
    let cfg = Config::default();
    let (ia, ib) = (
        issue::scan_issue(&a, &cfg).unwrap(),
        issue::scan_issue(&b, &cfg).unwrap(),
    );
    let r = comments::analyze(&c, &ia, &ib, &cfg).unwrap();
    let find = |t: &str| {
        r.comments
            .iter()
            .find(|c| c.text.contains(t))
            .unwrap_or_else(|| panic!("'{t}' niet gevonden"))
    };
    assert_eq!(find("detail aanpassen").proposal, Proposal::LikelyDone);
    assert_eq!(find("kozijnmaat").proposal, Proposal::NotDone);
    assert_eq!(find("maat vervalt").proposal, Proposal::LikelyDone);
    assert_eq!(find("hier wijzigen").proposal, Proposal::NotDone);
    assert_eq!(r.comments.len(), 4);
}

/// Echte opmerkingen van een aannemer: de drie afgevinkte opmerkingen krijgen
/// "waarschijnlijk verwerkt"; drie bladen zijn door de aannemer gedraaid.
#[test]
#[ignore]
fn golden_real_comments_gevels() {
    let c = need(&env_path("SHEETDIFF_GOLDEN_COMMENTS"));
    let a = need(&issue_a().join("TO+_wijzA_gevels.pdf"));
    let b = need(&issue_b().join("TO+_wijzA_gevels.pdf"));
    let cfg = Config::default();
    let (ia, ib) = (
        issue::scan_issue(&a, &cfg).unwrap(),
        issue::scan_issue(&b, &cfg).unwrap(),
    );
    let r = comments::analyze(&c, &ia, &ib, &cfg).unwrap();
    let ticked: Vec<_> = r
        .comments
        .iter()
        .filter(|c| !c.own_marks.is_empty())
        .collect();
    assert_eq!(ticked.len(), 3);
    assert!(ticked.iter().all(|c| c.proposal == Proposal::LikelyDone));
    let rotated = r
        .page_regs
        .values()
        .filter(|(_, reg)| reg.rotation.degrees() != 0)
        .count();
    assert_eq!(rotated, 3);
    assert!(
        r.comments
            .iter()
            .all(|c| c.kind != comments::CommentKind::ContentMarkup),
        "valse rode markering"
    );
}
