//! Keuzes van de gebruiker bij een vergelijking: koppelingen, ontkoppelingen, uitlijningen. Ze
//! staan in een eigen bestand per vergelijking, bij uitgifte B, zodat ze een nieuwe scan overleven
//! en met de uitgifte meereizen. Een blad wordt herkend aan bestandsnaam + pagina; de
//! inhoudshandtekening bewaakt dat het nog hetzelfde blad is. Sleutels zijn daarvoor niet
//! geschikt: die veranderen door het koppelen zelf.

use crate::issue::{self, Issue, LinkMethod, SheetLink, SheetRef};
use crate::register::ManualAlign;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const VERSION: u32 = 1;

/// Een blad in het keuzebestand. `key` is alleen leesbaarheid. `signature` 0 = niet controleren,
/// zodat een bestand ook met de hand te schrijven is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SheetId {
    pub file: String,
    /// 1-based, zoals in de CLI.
    pub page: usize,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub signature: u64,
}

fn file_name(s: &SheetRef) -> String {
    s.file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

impl SheetId {
    pub fn of(s: &SheetRef) -> SheetId {
        SheetId {
            file: file_name(s),
            page: s.page + 1,
            key: s.key.clone(),
            signature: s.signature,
        }
    }

    fn same_place(&self, o: &SheetId) -> bool {
        self.file == o.file && self.page == o.page
    }

    fn label(&self) -> String {
        format!("{} p.{}", self.file, self.page)
    }

    /// Huidige sleutel van dit blad. Staat het niet meer op zijn plek of is de inhoud anders,
    /// dan telt alleen een blad dat als enige dezelfde handtekening heeft.
    fn find(&self, i: &Issue) -> Option<String> {
        let at = i
            .sheets
            .values()
            .find(|s| file_name(s) == self.file && s.page + 1 == self.page);
        if let Some(s) = at {
            if self.signature == 0 || s.signature == self.signature {
                return Some(s.key.clone());
            }
        }
        if self.signature == 0 {
            return None;
        }
        let mut same = i
            .sheets
            .values()
            .filter(|s| s.signature == self.signature && s.text_runs > 0);
        match (same.next(), same.next()) {
            (Some(s), None) => Some(s.key.clone()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairChoice {
    pub a: SheetId,
    pub b: SheetId,
}

/// Handmatige uitlijning van een bladpaar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlignChoice {
    pub a: SheetId,
    pub b: SheetId,
    pub align: ManualAlign,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Choices {
    pub version: u32,
    pub a: PathBuf,
    pub b: PathBuf,
    /// Deze twee bladen zijn hetzelfde blad.
    #[serde(default)]
    pub links: Vec<PairChoice>,
    /// Deze twee bladen zijn niet hetzelfde blad, wat naam of inhoud ook zeggen.
    #[serde(default)]
    pub unlinks: Vec<PairChoice>,
    /// B wordt op deze manier op A gelegd, in plaats van automatisch.
    #[serde(default)]
    pub aligns: Vec<AlignChoice>,
}

impl Choices {
    pub fn new(a: &Path, b: &Path) -> Choices {
        Choices {
            version: VERSION,
            a: a.into(),
            b: b.into(),
            links: Vec::new(),
            unlinks: Vec::new(),
            aligns: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.links.is_empty() && self.unlinks.is_empty() && self.aligns.is_empty()
    }

    /// Standaardplek: `<B> vs <A>.sheetdiff.json`, in de map van B of naast het bestand B.
    pub fn default_path(a: &Path, b: &Path) -> PathBuf {
        let name = |p: &Path| {
            let n = if p.is_dir() {
                p.file_name()
            } else {
                p.file_stem()
            };
            n.unwrap_or_default().to_string_lossy().into_owned()
        };
        let file = format!("{} vs {}.sheetdiff.json", name(b), name(a));
        if b.is_dir() {
            b.join(file)
        } else {
            b.with_file_name(file)
        }
    }

    /// Leest het keuzebestand; geen bestand = geen keuzes. Een onleesbaar bestand is een fout:
    /// stil overslaan zou de koppelingen van de gebruiker kwijtraken. Staat het bij andere paden
    /// dan nu, dan volgt een waarschuwing en gelden de keuzes wel (de map kan verplaatst zijn).
    pub fn load(path: &Path, a: &Path, b: &Path) -> Result<(Choices, Option<String>)> {
        if !path.exists() {
            return Ok((Choices::new(a, b), None));
        }
        let s = std::fs::read_to_string(path)
            .with_context(|| format!("keuzebestand niet te lezen: {}", path.display()))?;
        let c: Choices = serde_json::from_str(&s)
            .with_context(|| format!("keuzebestand onleesbaar: {}", path.display()))?;
        anyhow::ensure!(
            c.version <= VERSION,
            "keuzebestand {} heeft versie {}; deze sheetdiff kent tot versie {VERSION}",
            path.display(),
            c.version
        );
        let warn = (c.a != a || c.b != b).then(|| {
            format!(
                "keuzebestand {} hoort bij A={} / B={}; keuzes overgenomen",
                path.display(),
                c.a.display(),
                c.b.display()
            )
        });
        Ok((c, warn))
    }

    /// Schrijft atomair (tijdelijk bestand + hernoemen).
    pub fn save(&self, path: &Path) -> Result<()> {
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)
            .with_context(|| format!("keuzebestand niet te schrijven: {}", tmp.display()))?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Legt vast dat `a` en `b` hetzelfde blad zijn. Een blad heeft maar een tegenhanger, dus
    /// eerdere koppelingen van deze bladen vervallen.
    pub fn set_link(&mut self, a: &SheetRef, b: &SheetRef) {
        let (a, b) = (SheetId::of(a), SheetId::of(b));
        self.links
            .retain(|c| !c.a.same_place(&a) && !c.b.same_place(&b));
        self.unlinks
            .retain(|c| !(c.a.same_place(&a) && c.b.same_place(&b)));
        self.links.push(PairChoice { a, b });
    }

    /// Legt vast dat `a` en `b` verschillende bladen zijn.
    pub fn set_unlink(&mut self, a: &SheetRef, b: &SheetRef) {
        let (a, b) = (SheetId::of(a), SheetId::of(b));
        self.links
            .retain(|c| !(c.a.same_place(&a) && c.b.same_place(&b)));
        if !self
            .unlinks
            .iter()
            .any(|c| c.a.same_place(&a) && c.b.same_place(&b))
        {
            self.aligns
                .retain(|c| !(c.a.same_place(&a) && c.b.same_place(&b)));
            self.unlinks.push(PairChoice { a, b });
        }
    }

    /// Legt de uitlijning van dit paar vast; `None` = terug naar automatisch.
    pub fn set_align(&mut self, a: &SheetRef, b: &SheetRef, align: Option<ManualAlign>) {
        let (a, b) = (SheetId::of(a), SheetId::of(b));
        self.aligns
            .retain(|c| !(c.a.same_place(&a) && c.b.same_place(&b)));
        if let Some(align) = align {
            self.aligns.push(AlignChoice { a, b, align });
        }
    }

    /// Uitlijning van twee losse pagina's (bestandsnaam, pagina 1-based), voor `page` en `overlay`.
    pub fn align_for(
        &self,
        a: &Path,
        page_a: usize,
        b: &Path,
        page_b: usize,
    ) -> Option<ManualAlign> {
        let name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned());
        let (na, nb) = (name(a)?, name(b)?);
        self.aligns
            .iter()
            .find(|c| c.a.file == na && c.a.page == page_a && c.b.file == nb && c.b.page == page_b)
            .map(|c| c.align)
    }
}

/// Uitkomst van [`apply`].
#[derive(Debug, Default)]
pub struct Applied {
    pub links: usize,
    pub unlinks: usize,
    pub aligns: usize,
    /// Paren (sleutel A, sleutel B) die het koppelen op inhoud niet mag maken.
    pub forbidden: Vec<(String, String)>,
    /// Keuzes die niet meer passen op de bladen van nu.
    pub warnings: Vec<String>,
}

/// Maakt `key` vrij in `i`: het blad dat hem draagt krijgt een achtervoegsel en blijft los.
fn free_key(i: &mut Issue, other: &[String], key: &str, suffix: &str) {
    if !i.sheets.contains_key(key) {
        return;
    }
    let mut new = format!("{key} {suffix}");
    while i.sheets.contains_key(&new) || other.contains(&new) {
        new.push_str(&format!(" {suffix}"));
    }
    issue::rekey(i, key, &new);
}

/// Past de keuzes toe op twee verse scans. Bij koppelen op naam of inhoud: voor het koppelen; bij
/// paginavolgorde erna, want die koppelt alles op positie. Wat niet meer past, komt als
/// waarschuwing terug en in de waarschuwingen van A; het blad volgt dan het gewone koppelen.
pub fn apply(c: &Choices, a: &mut Issue, b: &mut Issue) -> Applied {
    let mut out = Applied::default();
    let stale = |what: &str, p: &PairChoice| {
        format!(
            "keuzebestand: {what} A '{}' met B '{}' niet toegepast: blad niet gevonden of inhoud gewijzigd",
            p.a.label(),
            p.b.label()
        )
    };
    let mut kept = Vec::new();
    for p in &c.unlinks {
        let (Some(ka), Some(kb)) = (p.a.find(a), p.b.find(b)) else {
            out.warnings.push(stale("ontkoppeling", p));
            continue;
        };
        if ka == kb {
            if let Err(e) = issue::unlink_sheets(a, b, &ka) {
                out.warnings.push(format!("keuzebestand: {e:#}"));
                continue;
            }
        }
        kept.push(p);
    }
    for p in &c.links {
        let (Some(ka), Some(kb)) = (p.a.find(a), p.b.find(b)) else {
            out.warnings.push(stale("koppeling", p));
            continue;
        };
        let manual = |l: &SheetLink, k: &str| l.method == LinkMethod::Manual && l.key == k;
        if a.links.iter().any(|l| manual(l, &ka) || manual(l, &kb)) {
            out.warnings.push(format!(
                "keuzebestand: koppeling A '{}' met B '{}' overgeslagen: een van de bladen is al handmatig gekoppeld",
                p.a.label(),
                p.b.label()
            ));
            continue;
        }
        // een koppeling op paginavolgorde of een blad met dezelfde sleutel maakt plaats
        a.links.retain(|l| l.key != ka && l.key != kb);
        if ka == kb {
            a.links.push(SheetLink {
                key: ka.clone(),
                key_a: ka.clone(),
                key_b: kb,
                method: LinkMethod::Manual,
                score: None,
                low: false,
            });
            out.links += 1;
            continue;
        }
        let keys_b: Vec<String> = b.sheets.keys().cloned().collect();
        free_key(a, &keys_b, &kb, "(A)");
        let keys_a: Vec<String> = a.sheets.keys().cloned().collect();
        free_key(b, &keys_a, &ka, "(B)");
        match issue::link_sheets(a, b, &ka, &kb, LinkMethod::Manual) {
            Ok(_) => out.links += 1,
            Err(e) => out.warnings.push(format!("keuzebestand: {e:#}")),
        }
    }
    for p in kept {
        if let (Some(ka), Some(kb)) = (p.a.find(a), p.b.find(b)) {
            out.forbidden.push((ka, kb));
            out.unlinks += 1;
        }
    }
    a.warnings.extend(out.warnings.iter().cloned());
    out
}

/// Zet de bewaarde uitlijningen op de bladen. Na het koppelen: een uitlijning hoort bij een paar.
/// Is het paar er niet meer, dan volgt een waarschuwing en lijnt het blad automatisch uit.
pub fn apply_aligns(c: &Choices, a: &mut Issue, b: &Issue, out: &mut Applied) {
    for p in &c.aligns {
        let (ka, kb) = (p.a.find(a), p.b.find(b));
        match (ka, kb) {
            (Some(ka), Some(kb)) if ka == kb => {
                if let Some(s) = a.sheets.get_mut(&ka) {
                    s.align = Some(p.align);
                    out.aligns += 1;
                }
            }
            _ => {
                let w = format!(
                    "keuzebestand: uitlijning A '{}' met B '{}' niet toegepast: dit paar bestaat niet meer",
                    p.a.label(),
                    p.b.label()
                );
                a.warnings.push(w.clone());
                out.warnings.push(w);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::{PageGeom, Rot90};
    use crate::identify::IdMethod;

    fn sheet(key: &str, file: &str, page: usize, sig: u64) -> SheetRef {
        SheetRef {
            key: key.into(),
            number: None,
            title: String::new(),
            file: PathBuf::from(file),
            page,
            geom: PageGeom {
                crop: [0.0, 0.0, 100.0, 100.0],
                rotate: Rot90::R0,
            },
            method: IdMethod::None,
            text_runs: 10,
            booklets: vec![],
            signature: sig,
            tokens: Default::default(),
            ink: None,
            align: None,
            file_pages: 1,
        }
    }

    fn issue(sheets: Vec<SheetRef>) -> Issue {
        Issue {
            root: "x".into(),
            sheets: sheets.into_iter().map(|s| (s.key.clone(), s)).collect(),
            booklets: Default::default(),
            list_pages: vec![],
            warnings: vec![],
            links: vec![],
            notes: Default::default(),
        }
    }

    fn pair() -> (Issue, Issue) {
        (
            issue(vec![
                sheet("?a#1", "d/a.pdf", 0, 11),
                sheet("?a#2", "d/a.pdf", 1, 12),
            ]),
            issue(vec![
                sheet("?b#1", "d/b.pdf", 0, 21),
                sheet("?b#2", "d/b.pdf", 1, 22),
            ]),
        )
    }

    fn choices() -> Choices {
        Choices::new(Path::new("A"), Path::new("B"))
    }

    #[test]
    fn a_link_survives_a_new_scan() {
        let (a, b) = pair();
        let mut c = choices();
        c.set_link(&a.sheets["?a#1"], &b.sheets["?b#2"]);
        let (mut a2, mut b2) = pair();
        let r = apply(&c, &mut a2, &mut b2);
        assert_eq!((r.links, r.warnings.len()), (1, 0));
        assert_eq!(a2.links[0].method, LinkMethod::Manual);
        assert_eq!(a2.sheets["?b#2"].page, 0);
        assert!(b2.sheets.contains_key("?b#2"));
    }

    #[test]
    fn a_renamed_file_is_found_by_its_content() {
        let (a, b) = pair();
        let mut c = choices();
        c.set_link(&a.sheets["?a#1"], &b.sheets["?b#2"]);
        let (mut a2, _) = pair();
        let mut b2 = issue(vec![sheet("?n#7", "d/nieuw.pdf", 6, 22)]);
        let r = apply(&c, &mut a2, &mut b2);
        assert_eq!(r.links, 1);
        assert_eq!(a2.sheets["?n#7"].page, 0);
    }

    #[test]
    fn a_changed_sheet_is_not_linked_blindly_but_reported() {
        let (a, b) = pair();
        let mut c = choices();
        c.set_link(&a.sheets["?a#1"], &b.sheets["?b#2"]);
        let (mut a2, _) = pair();
        let mut b2 = issue(vec![sheet("?b#2", "d/b.pdf", 1, 99)]);
        let r = apply(&c, &mut a2, &mut b2);
        assert_eq!((r.links, r.warnings.len()), (0, 1));
        assert!(a2.links.is_empty());
        assert!(a2.warnings[0].contains("niet toegepast"));
    }

    #[test]
    fn a_hand_written_choice_needs_no_signature() {
        let mut c = choices();
        let id = |f: &str, p| SheetId {
            file: f.into(),
            page: p,
            key: String::new(),
            signature: 0,
        };
        c.links.push(PairChoice {
            a: id("a.pdf", 2),
            b: id("b.pdf", 1),
        });
        let (mut a, mut b) = pair();
        assert_eq!(apply(&c, &mut a, &mut b).links, 1);
        assert_eq!(a.sheets["?b#1"].page, 1);
    }

    #[test]
    fn an_unlinked_pair_on_name_stays_apart_and_is_forbidden() {
        let mk = || {
            (
                issue(vec![sheet("TO-1", "a.pdf", 0, 1)]),
                issue(vec![sheet("TO-1", "b.pdf", 0, 2)]),
            )
        };
        let (a, b) = mk();
        let mut c = choices();
        c.set_unlink(&a.sheets["TO-1"], &b.sheets["TO-1"]);
        let (mut a2, mut b2) = mk();
        let r = apply(&c, &mut a2, &mut b2);
        assert_eq!(r.unlinks, 1);
        let (ka, kb) = &r.forbidden[0];
        assert_ne!(ka, kb);
        assert!(a2.sheets.contains_key(ka) && b2.sheets.contains_key(kb));
    }

    #[test]
    fn a_link_takes_the_key_from_a_sheet_that_carries_it_on_the_other_side() {
        // A heeft ook een blad met de sleutel van B: dat paar gold alleen op naam
        let mk = || {
            (
                issue(vec![
                    sheet("?x#1", "x.pdf", 0, 1),
                    sheet("?y#1", "y.pdf", 0, 2),
                ]),
                issue(vec![sheet("?y#1", "yb.pdf", 0, 3)]),
            )
        };
        let (a, b) = mk();
        let mut c = choices();
        c.set_link(&a.sheets["?x#1"], &b.sheets["?y#1"]);
        let (mut a2, mut b2) = mk();
        assert_eq!(apply(&c, &mut a2, &mut b2).links, 1);
        assert_eq!(a2.sheets["?y#1"].file, PathBuf::from("x.pdf"));
        assert_eq!(a2.sheets["?y#1 (A)"].file, PathBuf::from("y.pdf"));
    }

    #[test]
    fn linking_after_unlinking_keeps_one_truth() {
        let (a, b) = pair();
        let (sa, sb) = (&a.sheets["?a#1"], &b.sheets["?b#1"]);
        let mut c = choices();
        c.set_unlink(sa, sb);
        c.set_link(sa, sb);
        assert!(c.unlinks.is_empty());
        c.set_unlink(sa, sb);
        assert!(c.links.is_empty() && c.unlinks.len() == 1);
        // een nieuwe koppeling vervangt de oude van hetzelfde blad
        c.set_link(sa, sb);
        c.set_link(sa, &b.sheets["?b#2"]);
        assert_eq!(c.links.len(), 1);
        assert_eq!(c.links[0].b.page, 2);
    }

    #[test]
    fn file_round_trip_and_a_broken_file_is_an_error() {
        let dir = std::env::temp_dir().join(format!("sheetdiff-keuzes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("k.sheetdiff.json");
        let (a, b) = pair();
        let mut c = choices();
        c.set_link(&a.sheets["?a#1"], &b.sheets["?b#2"]);
        c.save(&p).unwrap();
        let (r, warn) = Choices::load(&p, Path::new("A"), Path::new("B")).unwrap();
        assert_eq!(r.links, c.links);
        assert!(warn.is_none());
        let (_, warn) = Choices::load(&p, Path::new("A2"), Path::new("B")).unwrap();
        assert!(warn.is_some());
        std::fs::write(&p, "{ kapot").unwrap();
        assert!(Choices::load(&p, Path::new("A"), Path::new("B")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn comments_find_a_sheet_that_carries_the_key_of_b_after_linking() {
        let mut a = issue(vec![sheet("TO-1", "a.pdf", 0, 1)]);
        let mut b = issue(vec![sheet("TO-2", "b.pdf", 0, 2)]);
        a.sheets.get_mut("TO-1").unwrap().number = Some("TO-1".into());
        b.sheets.get_mut("TO-2").unwrap().number = Some("TO-2".into());
        issue::link_sheets(&mut a, &mut b, "TO-1", "TO-2", LinkMethod::Manual).unwrap();
        let s = crate::comments::sheet_of_number(&a, "TO-1").unwrap();
        assert_eq!(s.key, "TO-2");
    }

    #[test]
    fn an_alignment_follows_the_pair_and_lapses_with_it() {
        let m = ManualAlign {
            rotation_deg: 1.5,
            scale: 1.0,
            dx_mm: 3.0,
            dy_mm: 0.0,
        };
        let (a, b) = pair();
        let mut c = choices();
        c.set_link(&a.sheets["?a#1"], &b.sheets["?b#2"]);
        c.set_align(&a.sheets["?a#1"], &b.sheets["?b#2"], Some(m));
        let (mut a2, mut b2) = pair();
        let mut r = apply(&c, &mut a2, &mut b2);
        apply_aligns(&c, &mut a2, &b2, &mut r);
        assert_eq!(r.aligns, 1);
        assert_eq!(a2.sheets["?b#2"].align, Some(m));
        // zonder de koppeling is er geen paar
        c.links.clear();
        let (mut a3, mut b3) = pair();
        let mut r = apply(&c, &mut a3, &mut b3);
        apply_aligns(&c, &mut a3, &b3, &mut r);
        assert_eq!((r.aligns, r.warnings.len()), (0, 1));
        assert!(a3.sheets.values().all(|s| s.align.is_none()));
        // ontkoppelen en terug naar automatisch ruimen de uitlijning op
        c.set_align(&a.sheets["?a#1"], &b.sheets["?b#2"], None);
        assert!(c.aligns.is_empty());
        c.set_align(&a.sheets["?a#1"], &b.sheets["?b#2"], Some(m));
        c.set_unlink(&a.sheets["?a#1"], &b.sheets["?b#2"]);
        assert!(c.aligns.is_empty());
    }

    #[test]
    fn page_alignment_is_found_on_file_and_page() {
        let m = ManualAlign {
            rotation_deg: 0.0,
            scale: 1.0,
            dx_mm: 1.0,
            dy_mm: 0.0,
        };
        let (a, b) = pair();
        let mut c = choices();
        c.set_align(&a.sheets["?a#2"], &b.sheets["?b#1"], Some(m));
        let f = |n: &str| PathBuf::from(format!("elders/{n}"));
        assert_eq!(c.align_for(&f("a.pdf"), 2, &f("b.pdf"), 1), Some(m));
        assert_eq!(c.align_for(&f("a.pdf"), 1, &f("b.pdf"), 1), None);
    }

    #[test]
    fn default_path_lies_with_b_and_names_both_issues() {
        let p = Choices::default_path(Path::new("x/oud.pdf"), Path::new("y/nieuw.pdf"));
        assert_eq!(p, Path::new("y/nieuw vs oud.sheetdiff.json"));
    }
}
