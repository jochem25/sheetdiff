# STATUS

Laatst bijgewerkt: 2026-10-10

| Mijlpaal | Stand |
|---|---|
| Mockup (HTML) | akkoord Jochem 09-10 |
| M1 render + uitlijning + pixeldiff (CLI) | klaar: volledige uitgifte 260615->260624 (167 bladen, 63 A0-paren) in 82 s met 8 werkerprocessen |
| M2 viewer met overlay | klaar: egui-app (5 schermen), overlay/knipperen/naast elkaar/pixel/vector, 1:1-pixelzoom via detaillaag; screenshots buiten de repo |
| M3 opmerkingen aannemer | klaar: in app en CLI; op 71 door Jochem afgevinkte opmerkingen van een aannemer 77 % 'verwerkt', 10 % 'niet', 13 % 'onduidelijk' (docs/acceptatie-v1.md) |
| M4 export + acceptatie | klaar: docs/acceptatie-v1.md; 31-08-referentie niet toetsbaar (print weg) |
| Release v1.0.0 | gepubliceerd 10-10 na akkoord Jochem ('erg goed, heel bruikbaar'): GitHub-release v1.0.0, zie onder |

## Release v1.0.0 (10-10-2026)

- Tag `v1.0.0`; assets: sheetdiff.exe, sheetdiff-cli.exe, pdfium.dll (PDFium BSD-3, chromium/8086),
  LICENSE-pdfium.txt, LEESMIJ.txt en een zip met alles plus `licenties/` (meegebouwde bibliotheken).
- Geen testdata of tekeningen in de release. LEESMIJ staat ook in `docs/release/LEESMIJ.txt`.
- Gebouwd met `cargo build --release` op de tagcommit; tests groen (31 + 4 golden ignored); smoketest
  van de losse map (CLI `page` op twee gegenereerde PDF's, exit 1 'gewijzigd').
- Volgende stap: vragen/opmerkingen van Jochem (week 12-10) verzamelen; zie TODO 'Open na v1'.

## Bevindingen

- PDFium 157.0.8086: renderen naar een `FPDFBitmap_Gray` met matrix breekt het proces af
  (exit 3). Altijd via BGRA.
- PDFium past de matrix van `FPDF_RenderPageBitmapWithMatrix` toe op weergavepunten (na /Rotate).
- Bluebeam-opmerkingen: pagina's kunnen 90 gr gedraaid zijn (/Rotate) en er kan een voorblad bij.
- Planner 09-10: eerst mockup, pas na akkoord Jochem de app bouwen. Spike (render/uitlijnen/vector) mag door.
- TO-202n/TO-213z: tekening verschoven binnen vast kader (0,34 / 5 mm); uitlijning per viewport nodig.
- Vector-spike: 155k paden per A0-blad in 0,14 s gelezen, dekkingsvergelijking 0,64 s. Op TO-204n
  vindt vector de nieuwe peillijnen, tekst vindt `-87`, raster ziet daarnaast snippers < 0,15 mm.
- Valse verschillen: 260624 gevels vs gevels-noord (zelfde print): 4/4 gelijk. Plattegronden-zuid
  (24-06) vs plattegronden (26-06): 4 bladen alleen printstempel, de rest echte kleine wijzigingen
  (gecontroleerd op TO-108z: symbool verwijderd).
- pdfium-render zet standaard FPDF_REVERSE_BYTE_ORDER: kleurbuffers zijn RGBA. Eerst als BGRA gelezen
  waardoor rood/blauw verwisseld waren (rode markeringen onvindbaar, wolken blauw in de mockup).
- Tests: 25 unit + 5 registratie (zelfgemaakte PDF's, met negatieve controle) + 4 golden (referentieproject, --ignored).
- Review 09-10 (Sonnet + Ollama): 15 bevindingen getoetst en verwerkt, zie docs/review-2026-10-09.md.
- Dubbele tekeningnummers met andere inhoud binnen een uitgifte (referentieproject: 15 bladen) worden nu gemeld; nieuwste wint.
- Viewer-spike (tiles.rs, examples/tilebench): een tegel van 512 px kost 370-500 ms op een zware
  plattegrond, ONAFHANKELIJK van de zoom (PDFium loopt per aanroep alle paden door). Volledige
  pagina: 75 dpi 0,37 s, 150 dpi 0,50 s, 200 dpi 0,63 s. Ontwerp viewer: basislaag per blad op
  150 dpi in geheugen (direct pannen/zoomen), daarboven zichtbaar beeld in een aanroep op de
  exacte zoom (~0,4 s, met opgeschaalde basislaag als voorlopig beeld), buurbladen vooraf in
  werkerprocessen.
- UI: automatische screenshotmodus `SHEETDIFF_AUTOSHOT=<map>` (+ SHEETDIFF_A/B/C/SHEET) loopt alle schermen langs.
