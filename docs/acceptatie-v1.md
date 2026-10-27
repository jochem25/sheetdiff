# Acceptatie sheetdiff v1 (09-10-2026)

Toetsing tegen de acceptatiecriteria van de opdracht voor v1.
Uitvoer staat buiten de repo (bevat klantbeelden).

| # | Criterium | Uitkomst | Bewijs |
|---|---|---|---|
| 1a | Referentieproject end-to-end, zelfde verschillenbeeld als de Python-analyse | **Deels.** De referentieprint van 31-08 en de Python-rapportage bestaan niet meer. Wat er wel is: het gevels-noord-paar (24-06 vs 31-08), daar vindt de tool `-87` op TO-201n t/m TO-204n en niets vervallen, zoals de Python-analyse. De overige punten uit plan §8 (GO-oppervlakten, RD-regels, SL18/RW30) zijn zonder de print niet te toetsen. | `tests/golden.rs` `golden_gevels_noord_87` |
| 1b | 0 valse verschillen door printstempels of anti-aliasing op gelijke bladen | **Ja.** Zelfde print in twee boekjes: 4/4 gelijk. Prints 24-06 vs 26-06: bladen met alleen een nieuw printstempel krijgen 'alleen printstempel'; de rest zijn echte wijzigingen (steekproef TO-108z: symbool verwijderd). | `golden_same_print_no_false_diffs`; STATUS.md |
| 1c | Volledige uitgifte | 260615 -> 260624: 167 bladen (63 A0-paren + 104 detailboekpagina's zonder tegenhanger in B), 0 niet vergelijkbaar, ~3,3 min met 8 werkers (300 dpi, raster + tekst + vector). Ontdekt: 15 bladen staan in B in twee boekjes met verschillende inhoud; gemeld, nieuwste vergeleken. | `acceptatie/260615_vs_260624.*`, `acceptatie/wolken/` |
| 2 | Overlay en pixelweergave op A0 tot 1:1-pixel, vloeiend | **Ja.** Basislaag per blad (~90 dpi) direct pannen/zoomen; daarboven detaillaag op exacte zoom (~0,4 s na stilstaan), scherp op 300 dpi. | `screenshots/M2/06_pixelzoom_1op1.png`, `07_knipperen_B.png` |
| 3 | Opmerkingentest: elke opmerking een plek op B en een voorstel; voorstellen kloppen; export xlsx/csv + PDF met status | **Ja voor het testbestand, deels voor de echte opmerkingen** (zie hieronder). Export werkt: xlsx, csv, PDF-kopie van B met opmerkingen en status (verwijderbare annotaties). | `golden_synthetic_comments` (4/4); `acceptatie/opmerkingen/` |
| 4 | Tests, clippy, review ander model | **Ja.** 22 unit + 5 registratie (bekende verschuiving/schaal/rotatie, met negatieve controle) + 4 insta-snapshots + 4 golden + werker-faalpadtest. Clippy schoon (kern, CLI, UI). Twee reviewrondes (Sonnet + Ollama): `docs/review-2026-10-09.md`, `docs/review-ui-2026-10-09.md`. | `cargo test`, `cargo clippy` |

## Opmerkingen van de aannemer tegen Jochems afvinking

Jochem heeft in de "gedaan"-PDF's met potlood (Ink) afgevinkt wat hij verwerkt heeft. Dat is de
beste beschikbare grondwaarheid, maar geen volledige: een vink kan ook "gezien" betekenen.

| Opmerkingen-PDF | Afgevinkt (annotaties) | voorstel verwerkt | niet verwerkt | onduidelijk |
|---|---|---|---|---|
| gevels | 3 | 3 | 0 | 0 |
| plattegronden | 19 (+38 verwijzingsnummers apart) | 16 | 2 | 1 |
| fragmenten | 41 | 30 | 5 | 6 |
| doorsneden | 8 | 6 | 0 | 2 |
| **totaal** | **71** | **55 (77 %)** | **7 (10 %)** | **9 (13 %)** |

Van de niet-afgevinkte opmerkingen kregen er 11 'waarschijnlijk verwerkt' (er is verschil in de zone; mogelijk verwerkt maar niet afgevinkt) en 0 'niet verwerkt'.

Onderweg gevonden en hersteld: Bluebeam koppelt een wolk aan zijn tekstvak met `/IRT` + `/RT /Group`;
die werd eerst als antwoord overgeslagen (plattegronden: 11 -> 16 verwerkt na herstel).

Waar het voorstel afwijkt van de vink zijn er drie bekende oorzaken:
1. **Verwerkt op een andere plek** dan de wolk. Voorbeeld: "symbool toevoegen in de legenda". De
   wijziging staat in de legenda, niet bij de wolk.
2. **Informatieve opmerkingen** ("ter info: dit onderdeel is afgerond"), waar niets hoeft te veranderen.
3. **Verwijzingsnummers** ("3", "12"), die naar een lijst elders wijzen. Die krijgen 'onduidelijk'
   met die reden.

Het voorstel is bedoeld als sortering: de status zet Jochem zelf.

## Niet gehaald / open

- Volledige 63-bladen-referentie van 31-08 (print ontbreekt).
- Scan van een uitgifte met veel dubbele bladen duurt ~1 min (tolerante vectorvergelijking per dubbeling).
- GitHub-release: gepubliceerd 10-10 als v1.0.0 na akkoord Jochem.
