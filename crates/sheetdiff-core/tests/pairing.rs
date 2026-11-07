//! Koppelen op inhoud met zelfgemaakte PDF's: elk "blad" heeft eigen lijnwerk en tekst in een
//! gedeeld kader, zoals bladen uit een boekje. Geen projecttekeningen.

use pdfium_render::prelude::*;
use sheetdiff_core::choices::Choices;
use sheetdiff_core::compare::{self, Runner, Verdict};
use sheetdiff_core::geom::PT_PER_MM;
use sheetdiff_core::issue::{self, Issue, LinkMethod};
use sheetdiff_core::pairing::{pair_by_order, pair_sheets, pair_with_choices};
use sheetdiff_core::{pdf, Config};
use std::path::{Path, PathBuf};

const W_MM: f64 = 420.0;
const H_MM: f64 = 297.0;

/// Maakt een PDF met een pagina per zaad; hetzelfde zaad geeft hetzelfde blad.
fn make(path: &Path, seeds: &[u32]) {
    let p = pdf::pdfium().expect("pdfium.dll");
    let mut doc = p.create_new_pdf().unwrap();
    let font = doc.fonts_mut().helvetica();
    for &seed in seeds {
        let hpt = H_MM * PT_PER_MM;
        let mut page = doc
            .pages_mut()
            .create_page_at_end(PdfPagePaperSize::Custom(
                PdfPoints::new((W_MM * PT_PER_MM) as f32),
                PdfPoints::new(hpt as f32),
            ))
            .unwrap();
        let at = |x: f64, y: f64| {
            (
                PdfPoints::new((x * PT_PER_MM) as f32),
                PdfPoints::new((hpt - y * PT_PER_MM) as f32),
            )
        };
        let line = |page: &mut PdfPage, x0, y0, x1, y1| {
            let ((a, b), (c, d)) = (at(x0, y0), at(x1, y1));
            page.objects_mut()
                .create_path_object_line(a, b, c, d, PdfColor::BLACK, PdfPoints::new(0.5))
                .unwrap();
        };
        // gedeeld kader en titelhoek
        for (x0, y0, x1, y1) in [
            (10.0, 10.0, 410.0, 10.0),
            (410.0, 10.0, 410.0, 287.0),
            (410.0, 287.0, 10.0, 287.0),
            (10.0, 287.0, 10.0, 10.0),
            (300.0, 250.0, 410.0, 250.0),
            (300.0, 250.0, 300.0, 287.0),
        ] {
            line(&mut page, x0, y0, x1, y1);
        }
        // eigen lijnwerk: een onregelmatig patroon uit het zaad
        let mut r = seed.wrapping_mul(2654435761).wrapping_add(12345);
        let mut next = |m: u32| {
            r = r.wrapping_mul(1664525).wrapping_add(1013904223);
            ((r >> 8) % m) as f64
        };
        for _ in 0..120 {
            let (x, y) = (20.0 + next(360), 20.0 + next(220));
            let (dx, dy) = (next(80), next(80));
            if next(2) == 0.0 {
                line(&mut page, x, y, (x + dx).min(400.0), y);
            } else {
                line(&mut page, x, y, x, (y + dy).min(245.0));
            }
        }
        // eigen tekst plus de vaste tekst van de titelhoek
        let mut text = |t: String, x: f64, y: f64| {
            let (a, b) = at(x, y);
            page.objects_mut()
                .create_text_object(a, b, t, font, PdfPoints::new(9.0))
                .unwrap();
        };
        text("Projectnaam Testgebouw".into(), 305.0, 262.0);
        for i in 0..12u32 {
            text(
                format!("onderdeel{seed}x{i} maat{}", seed * 37 + i),
                30.0,
                30.0 + i as f64 * 15.0,
            );
        }
    }
    doc.save_to_file(path).unwrap();
}

/// Tests draaien parallel, maar PDF's maken met tekst naast andere PDFium-aanroepen breekt het
/// proces af (bad_variant_access). Daarom een test tegelijk; de app maakt zelf geen PDF's zo.
static PDFIUM_TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Dir(
    PathBuf,
    #[allow(dead_code)] std::sync::MutexGuard<'static, ()>,
);

impl Dir {
    fn new(name: &str) -> Dir {
        let turn = PDFIUM_TURN.lock().unwrap_or_else(|e| e.into_inner());
        let d = std::env::temp_dir().join(format!("sheetdiff-pair-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("a")).unwrap();
        std::fs::create_dir_all(d.join("b")).unwrap();
        Dir(d, turn)
    }
    fn a(&self, file: &str, seeds: &[u32]) {
        make(&self.0.join("a").join(file), seeds);
    }
    fn b(&self, file: &str, seeds: &[u32]) {
        make(&self.0.join("b").join(file), seeds);
    }
    fn scan(&self) -> (Issue, Issue) {
        let cfg = Config::default();
        (
            issue::scan_issue(&self.0.join("a"), &cfg).unwrap(),
            issue::scan_issue(&self.0.join("b"), &cfg).unwrap(),
        )
    }
    fn scan_pages(&self) -> (Issue, Issue) {
        let cfg = Config::default();
        (
            issue::scan_pages(&self.0.join("a"), &cfg).unwrap(),
            issue::scan_pages(&self.0.join("b"), &cfg).unwrap(),
        )
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Paginanummers (1-based) van de gekoppelde bladen: (pagina in A, pagina in B).
fn pairs(a: &Issue, b: &Issue) -> Vec<(usize, usize)> {
    let mut v: Vec<(usize, usize)> = a
        .sheets
        .iter()
        .filter_map(|(k, s)| b.sheets.get(k).map(|o| (s.page + 1, o.page + 1)))
        .collect();
    v.sort();
    v
}

fn only_in(x: &Issue, y: &Issue) -> usize {
    x.sheets
        .keys()
        .filter(|k| !y.sheets.contains_key(*k))
        .count()
}

#[test]
fn same_sheets_under_another_file_name() {
    let d = Dir::new("naam");
    d.a("boekje.pdf", &[1, 2, 3]);
    d.b("boekje_wijzA.pdf", &[1, 2, 3]);
    let (mut a, mut b) = d.scan();
    let cfg = Config::default();
    assert_eq!(
        pair_sheets(&mut a, &mut b, &cfg, &Runner::InProcess).unwrap(),
        3
    );
    assert_eq!(pairs(&a, &b), vec![(1, 1), (2, 2), (3, 3)]);
    assert!(a.links.iter().all(|l| l.method == LinkMethod::Content));
    assert!(a.links.iter().all(|l| l.score.unwrap() > 0.99));
}

#[test]
fn inserted_sheet_is_new_and_the_rest_stays_paired() {
    let d = Dir::new("ingevoegd");
    d.a("boekje.pdf", &[1, 2, 3]);
    d.b("boekje_wijzA.pdf", &[1, 9, 2, 3]);
    let (mut a, mut b) = d.scan();
    pair_sheets(&mut a, &mut b, &Config::default(), &Runner::InProcess).unwrap();
    assert_eq!(pairs(&a, &b), vec![(1, 1), (2, 3), (3, 4)]);
    assert_eq!(only_in(&b, &a), 1);
    assert_eq!(only_in(&a, &b), 0);
    let note = b
        .notes
        .values()
        .next()
        .expect("toelichting bij het losse blad");
    assert!(note.contains("beste kandidaat"), "{note}");
}

#[test]
fn removed_sheet_is_removed_and_the_rest_stays_paired() {
    let d = Dir::new("verwijderd");
    d.a("boekje.pdf", &[1, 2, 3, 4]);
    d.b("boekje_wijzA.pdf", &[1, 3, 4]);
    let (mut a, mut b) = d.scan();
    pair_sheets(&mut a, &mut b, &Config::default(), &Runner::InProcess).unwrap();
    assert_eq!(pairs(&a, &b), vec![(1, 1), (3, 2), (4, 3)]);
    assert_eq!(only_in(&a, &b), 1);
}

#[test]
fn swapped_order_follows_the_content() {
    let d = Dir::new("verwisseld");
    d.a("boekje.pdf", &[1, 2, 3]);
    d.b("boekje_wijzA.pdf", &[3, 1, 2]);
    let (mut a, mut b) = d.scan();
    pair_sheets(&mut a, &mut b, &Config::default(), &Runner::InProcess).unwrap();
    assert_eq!(pairs(&a, &b), vec![(1, 2), (2, 3), (3, 1)]);
}

#[test]
fn a_completely_different_sheet_is_not_paired() {
    let d = Dir::new("anders");
    d.a("boekje.pdf", &[1]);
    d.b("boekje_wijzA.pdf", &[7]);
    let (mut a, mut b) = d.scan();
    let n = pair_sheets(&mut a, &mut b, &Config::default(), &Runner::InProcess).unwrap();
    assert_eq!(n, 0);
    assert!(pairs(&a, &b).is_empty());
    assert_eq!((only_in(&a, &b), only_in(&b, &a)), (1, 1));
}

/// Een los bestand met dezelfde naam als een boekje mag het juiste paar niet wegkapen.
#[test]
fn a_loose_file_with_the_same_name_does_not_take_the_right_pair() {
    let d = Dir::new("losbestand");
    d.a("boekje.pdf", &[1, 2]);
    d.b("boekje.pdf", &[8]); // zelfde sleutel als A pagina 1, andere inhoud
    d.b("boekje_wijzA.pdf", &[1, 2]);
    let (mut a, mut b) = d.scan();
    pair_sheets(&mut a, &mut b, &Config::default(), &Runner::InProcess).unwrap();
    for s in a.sheets.values() {
        let o = &b.sheets[&s.key];
        assert_eq!(s.page, o.page);
        assert!(o.file.ends_with("boekje_wijzA.pdf"), "{}", o.file.display());
    }
    assert_eq!(only_in(&b, &a), 1);
    let d2 = compare::compare_issues(&a, &b, &Config::default(), |_| {}).unwrap();
    assert_eq!(d2.count(Verdict::AddedInB), 1);
    assert_eq!(d2.count(Verdict::Identical), 2);
    assert!(d2
        .sheets
        .iter()
        .all(|s| s.pair_label.starts_with("op inhoud") || s.verdict == Verdict::AddedInB));
}

#[test]
fn same_name_and_same_content_stays_a_pair_on_name() {
    let d = Dir::new("opnaam");
    d.a("boekje.pdf", &[1, 2]);
    d.b("boekje.pdf", &[1, 2]);
    let (mut a, mut b) = d.scan();
    assert_eq!(
        pair_sheets(&mut a, &mut b, &Config::default(), &Runner::InProcess).unwrap(),
        0
    );
    let r = compare::compare_issues(&a, &b, &Config::default(), |_| {}).unwrap();
    assert_eq!(r.sheets.len(), 2);
    assert!(r.sheets.iter().all(|s| s.pair_label == "op naam"));
}

/// Hernummerd of de verdieping erboven? Dat is niet te zien, dus nooit zonder twijfel.
#[test]
fn two_different_numbers_with_the_same_content_stay_a_doubt() {
    let d = Dir::new("nummers");
    d.a("1234-TO-100 plattegrond.pdf", &[1]);
    d.b("1234-TO-200 plattegrond.pdf", &[1]);
    let (mut a, mut b) = d.scan();
    assert!(a.sheets.contains_key("TO-100") && b.sheets.contains_key("TO-200"));
    pair_sheets(&mut a, &mut b, &Config::default(), &Runner::InProcess).unwrap();
    assert_eq!(a.links.len(), 1);
    assert_eq!(a.links[0].method, LinkMethod::Doubt);
    assert_eq!(a.links[0].label(), "twijfel (100 %)");
    assert!(a.links[0].note().is_some());
}

#[test]
fn switched_off_nothing_is_paired() {
    let d = Dir::new("uit");
    d.a("boekje.pdf", &[1]);
    d.b("boekje_wijzA.pdf", &[1]);
    let (mut a, mut b) = d.scan();
    let mut cfg = Config::default();
    cfg.pair.content = false;
    assert_eq!(
        pair_sheets(&mut a, &mut b, &cfg, &Runner::InProcess).unwrap(),
        0
    );
    assert!(pairs(&a, &b).is_empty());
}

#[test]
fn by_order_pairs_page_i_with_page_i_and_warns_when_the_content_differs() {
    let d = Dir::new("volgorde");
    d.a("boekje.pdf", &[1, 2, 3]);
    d.b("iets_anders.pdf", &[1, 9, 3, 4]);
    let (mut a, mut b) = d.scan_pages();
    let cfg = Config::default();
    assert_eq!(
        pair_by_order(&mut a, &mut b, &cfg, &Runner::InProcess).unwrap(),
        3
    );
    assert_eq!(pairs(&a, &b), vec![(1, 1), (2, 2), (3, 3)]);
    assert_eq!(only_in(&b, &a), 1);
    let low: Vec<bool> = a.links.iter().map(|l| l.low).collect();
    assert_eq!(low, vec![false, true, false]);
    let r = compare::compare_issues(&a, &b, &cfg, |_| {}).unwrap();
    assert_eq!(r.count(Verdict::AddedInB), 1);
    assert_eq!(
        r.warnings
            .iter()
            .filter(|w| w.contains("verschoven"))
            .count(),
        1
    );
    assert!(r
        .sheets
        .iter()
        .filter(|s| s.a.is_some() && s.b.is_some())
        .all(|s| s.pair_label == "op paginavolgorde"));
}

#[test]
fn by_order_with_fewer_pages_in_b_reports_the_rest_as_removed() {
    let d = Dir::new("minder");
    d.a("boekje.pdf", &[1, 2, 3]);
    d.b("boekje.pdf", &[1, 2]);
    let (mut a, mut b) = d.scan_pages();
    let cfg = Config::default();
    pair_by_order(&mut a, &mut b, &cfg, &Runner::InProcess).unwrap();
    let r = compare::compare_issues(&a, &b, &cfg, |_| {}).unwrap();
    assert_eq!(r.count(Verdict::RemovedFromB), 1);
    assert_eq!(r.count(Verdict::Identical), 2);
}

#[test]
fn unlinking_an_automatic_pair_gives_a_removed_and_a_new_sheet() {
    let d = Dir::new("ontkoppel");
    d.a("boekje.pdf", &[1, 2]);
    d.b("boekje_wijzA.pdf", &[1, 2]);
    let (mut a, mut b) = d.scan();
    let cfg = Config::default();
    pair_sheets(&mut a, &mut b, &cfg, &Runner::InProcess).unwrap();
    let key = a.links[0].key.clone();
    let (ka, kb) = issue::unlink_sheets(&mut a, &mut b, &key).unwrap();
    assert_eq!(a.links.len(), 1);
    let r = compare::compare_issues(&a, &b, &cfg, |_| {}).unwrap();
    let verdict = |k: &str| r.sheets.iter().find(|s| s.key == k).unwrap().verdict;
    assert_eq!(verdict(&ka), Verdict::RemovedFromB);
    assert_eq!(verdict(&kb), Verdict::AddedInB);
    // en weer handmatig te koppelen
    let key = issue::link_sheets(&mut a, &mut b, &ka, &kb, LinkMethod::Manual).unwrap();
    assert!(a.sheets.contains_key(&key) && b.sheets.contains_key(&key));
}

/// Zet /Rotate op elke pagina: het blad staat dan op zijn kant, zoals na een scan of Bluebeam.
fn turn(path: &Path, rotation: PdfPageRenderRotation) {
    let p = pdf::pdfium().expect("pdfium.dll");
    let doc = p.load_pdf_from_file(path, None).unwrap();
    for mut page in doc.pages().iter() {
        page.set_rotation(rotation);
    }
    let tmp = path.with_extension("tmp");
    doc.save_to_file(&tmp).unwrap();
    drop(doc);
    std::fs::rename(&tmp, path).unwrap();
}

#[test]
fn a_sheet_turned_a_quarter_is_paired_and_compared() {
    for rotation in [
        PdfPageRenderRotation::Degrees90,
        PdfPageRenderRotation::Degrees270,
    ] {
        let d = Dir::new("kwartslag");
        d.a("boekje.pdf", &[1, 2]);
        d.b("boekje_wijzA.pdf", &[2, 1]);
        turn(&d.0.join("b").join("boekje_wijzA.pdf"), rotation);
        let (mut a, mut b) = d.scan();
        let cfg = Config::default();
        pair_sheets(&mut a, &mut b, &cfg, &Runner::InProcess).unwrap();
        assert_eq!(pairs(&a, &b), vec![(1, 2), (2, 1)]);
        assert!(a.links.iter().all(|l| l.method == LinkMethod::Content));
        let r = compare::compare_issues(&a, &b, &cfg, |_| {}).unwrap();
        assert!(r.sheets.iter().all(|s| s.verdict != Verdict::NotComparable));
        drop(d);
    }
}

#[test]
fn a_turned_sheet_with_other_content_is_not_paired() {
    let d = Dir::new("kwartslag-anders");
    d.a("boekje.pdf", &[1]);
    d.b("gevels.pdf", &[7]);
    turn(
        &d.0.join("b").join("gevels.pdf"),
        PdfPageRenderRotation::Degrees90,
    );
    let (mut a, mut b) = d.scan();
    pair_sheets(&mut a, &mut b, &Config::default(), &Runner::InProcess).unwrap();
    assert!(pairs(&a, &b).is_empty());
}

#[test]
fn a_saved_link_and_unlink_hold_after_a_new_scan() {
    let d = Dir::new("keuzes");
    d.a("boekje.pdf", &[1, 2, 3]);
    d.b("boekje_wijzA.pdf", &[1, 2, 4]);
    let cfg = Config::default();
    let (mut a, mut b) = d.scan();
    pair_sheets(&mut a, &mut b, &cfg, &Runner::InProcess).unwrap();
    assert_eq!(pairs(&a, &b), vec![(1, 1), (2, 2)]);
    let page = |i: &Issue, p: usize| i.sheets.values().find(|s| s.page + 1 == p).unwrap().clone();

    // de gebruiker: blad 1 is een ander blad, en blad 3 van A is blad 3 van B
    let mut c = Choices::new(Path::new("A"), Path::new("B"));
    c.set_unlink(&page(&a, 1), &page(&b, 1));
    c.set_link(&page(&a, 3), &page(&b, 3));

    let (mut a2, mut b2) = d.scan();
    let r = pair_with_choices(&mut a2, &mut b2, &cfg, &Runner::InProcess, &c, false).unwrap();
    assert_eq!((r.links, r.unlinks, r.warnings.len()), (1, 1, 0));
    assert_eq!(pairs(&a2, &b2), vec![(2, 2), (3, 3)]);
    let manual = a2.links.iter().find(|l| l.method == LinkMethod::Manual);
    assert_eq!(a2.sheets[&manual.unwrap().key].page, 2);
    assert_eq!((only_in(&a2, &b2), only_in(&b2, &a2)), (1, 1));
}

#[test]
fn saved_choices_also_hold_on_page_order() {
    let d = Dir::new("keuzes-volgorde");
    d.a("boekje.pdf", &[1, 2]);
    d.b("boekje_wijzA.pdf", &[2, 1]);
    let (a, b) = d.scan_pages();
    let page = |i: &Issue, p: usize| i.sheets.values().find(|s| s.page + 1 == p).unwrap().clone();
    let mut c = Choices::new(Path::new("A"), Path::new("B"));
    c.set_link(&page(&a, 1), &page(&b, 2));
    let (mut a2, mut b2) = (a.clone(), b.clone());
    let cfg = Config::default();
    let r = pair_with_choices(&mut a2, &mut b2, &cfg, &Runner::InProcess, &c, true).unwrap();
    assert_eq!(r.links, 1);
    assert!(pairs(&a2, &b2).contains(&(1, 2)));
}
