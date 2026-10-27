# sheetdiff

Windows-desktoptool (Rust) die twee uitgiftes van bouwtekeningen (PDF) pixelprecies vergelijkt,
met overlay, en opmerkingen van de aannemer (derde PDF) uitlijnt en per opmerking voorstelt of
hij in de nieuwe revisie verwerkt is. Gemaakt door Kolthof Bouwadvies (KBA).

- `crates/sheetdiff-core` - logica: PDFium-render, registratie, pixeldiff, tekstdiff, vectordiff, opmerkingen, rapport
- `crates/sheetdiff-cli` - headless (`sheetdiff-cli compare A B --report r.md --csv r.csv`)
- `crates/sheetdiff-ui` - egui-viewer (overlay, knipperen, naast elkaar, pixel, vector)

Kant-en-klare Windows-build: zie [Releases](../../releases) (exe's + `pdfium.dll` in een zip).

## Bouwen

PDFium (BSD-3) komt van [bblanchon/pdfium-binaries](https://github.com/bblanchon/pdfium-binaries),
build chromium/8086 (157.0.8086). Uitpakken naar `vendor/pdfium/` (gitignored); de exe zoekt
`pdfium.dll` naast zichzelf, via `SHEETDIFF_PDFIUM`, of in `vendor/pdfium/bin`.

```
cargo build --release
cargo test
```

## Testdata

Nooit in de repo: de golden-tests draaien op tekeningen van een referentieproject en lezen hun
paden uit omgevingsvariabelen. Zie `docs/testdata.md`.

## CLI

```
sheetdiff-cli scan <map|pdf>                                 bladen op tekeningnummer
sheetdiff-cli compare <A> <B> --report r.md --xlsx r.xlsx --csv r.csv --json r.json
                      [--clouds map] [--workers N] [--only TO-101,TO-102] [--dpi 300]
sheetdiff-cli comments <opm.pdf> <A> <B> --xlsx o.xlsx --csv o.csv --pdf-dir map [--state s.json]
sheetdiff-cli page <a.pdf> <p> <b.pdf> <p>                   twee losse pagina's
sheetdiff-cli overlay <a.pdf> <p> <b.pdf> <p> --dpi 300 --crop x0,y0,x1,y1 --out o.png
sheetdiff-cli vec <a.pdf> <p> <b.pdf> <p>                    vectorvergelijking
sheetdiff-cli profile > profiel.toml                         alle instellingen, aanpasbaar
```

Exitcodes `compare`: 0 geen verschillen, 1 verschillen, 2 niet vergelijkbaar of fout.

## Uitkomsten

Per blad: `gelijk`, `alleen printstempel`, `gewijzigd`, `nieuw in B`, `vervallen in B`,
`NIET VERGELIJKBAAR`. Een mislukte uitlijning of onleesbaar bestand geeft altijd
`NIET VERGELIJKBAAR` (exitcode 2), nooit "gelijk".

## Licentie

Naar keuze [MIT](LICENSE-MIT) of [Apache-2.0](LICENSE-APACHE).
`pdfium.dll` valt onder de eigen licentie van PDFium (BSD-3-Clause).
