# sheetdiff

Windows-desktoptool (Rust) die twee uitgiftes van bouwtekeningen (PDF) pixelprecies vergelijkt,
met overlay, en opmerkingen van de aannemer (derde PDF) uitlijnt en per opmerking voorstelt of
hij in de nieuwe revisie verwerkt is. Gemaakt door Kolthof Bouwadvies (KBA).

- `crates/sheetdiff-core` - logica: PDFium-render, registratie, pixeldiff, tekstdiff, vectordiff, opmerkingen, rapport
- `crates/sheetdiff-cli` - headless (`sheetdiff-cli compare A B --report r.md --csv r.csv`)
- `crates/sheetdiff-ui` - egui-viewer (overlay, knipperen, naast elkaar, pixel, vector)

Kant-en-klare Windows-build: zie [Releases](../../releases) (exe's + `pdfium.dll` in een zip).

Installer bouwen (Inno Setup 6, per gebruiker, zonder beheerdersrechten):
`powershell -ExecutionPolicy Bypass -File installer\build-installer.ps1`. De installatie gaat naar
`%LOCALAPPDATA%\Programs\sheetdiff`; een nieuwere versie installeert over de oude.

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
                      [--pair-by-order]                      pagina i tegen pagina i
                      [--choices f | --no-choices]           bewaarde keuzes (ook bij pairs en comments)
sheetdiff-cli pairs <A> <B> [--csv s.csv] [--list l.tsv] [--no-names]   koppeling en scores bekijken
sheetdiff-cli comments <opm.pdf> <A> <B> --xlsx o.xlsx --csv o.csv --pdf-dir map [--state s.json]
sheetdiff-cli page <a.pdf> <p> <b.pdf> <p> [--align rot,schaal,dx,dy | --align-file f]   twee losse pagina's
sheetdiff-cli overlay <a.pdf> <p> <b.pdf> <p> --dpi 300 --crop x0,y0,x1,y1 --out o.png [--align ...]
sheetdiff-cli vec <a.pdf> <p> <b.pdf> <p>                    vectorvergelijking
sheetdiff-cli profile > profiel.toml                         alle instellingen, aanpasbaar
```

Exitcodes `compare`: 0 geen verschillen, 1 verschillen, 2 niet vergelijkbaar of fout.

## Bladen koppelen

Eerst op tekeningnummer of bouwnummer. Wat overblijft, wordt op inhoud gekoppeld: lijnwerk en
tekst, bij gelijk bladformaat, globaal optimaal over alle bladen. Een ingevoegd of verwijderd blad
eindigt als nieuw of vervallen. Drempels staan in het profiel (`pair.min_score`, `pair.doubt_score`).

Per paar staat in rapport en app hoe het gekoppeld is:

| Label | Betekenis |
|---|---|
| `op naam` | zelfde nummer of zelfde bestandsnaam en pagina |
| `op inhoud (87 %)` | gekoppeld op lijnwerk en tekst |
| `twijfel (62 %)` | gekoppeld, maar controleren; in de app oranje |
| `op paginavolgorde` | pagina i tegen pagina i (`--pair-by-order`, of de knop in de app) |
| `handmatig` | door de gebruiker gekoppeld |

Bladen die een kwartslag gedraaid zijn (/Rotate of gedraaide inhoud) worden ook op inhoud gekoppeld.
Inktkaarten blijven bewaard in `%TEMP%\sheetdiff-inkt`, zodat een volgende vergelijking sneller is
(`pair.ink_cache`). Hangt een blad langer dan `compare.sheet_timeout_s`, dan wordt alleen dat blad
`NIET VERGELIJKBAAR`.

Bij een los blad staat in het rapport de beste kandidaat met zijn score. In de app kan elk paar
worden ontkoppeld en een los blad handmatig gekoppeld; de opmerkingen worden dan opnieuw
geanalyseerd.

Die keuzes staan in `<B> vs <A>.sheetdiff.json`, in de map van B of naast het bestand B, en gelden
ook bij de volgende vergelijking, in app en CLI. Een ander bestand: `--choices`; negeren:
`--no-choices`. Ook een handmatige uitlijning (vrije rotatie, schaal, verschuiving) staat daar;
`--align-file` is hetzelfde als `--choices`. Is een blad veranderd of weg, dan vervalt de keuze met een waarschuwing.

## Uitkomsten

Per blad: `gelijk`, `alleen printstempel`, `gewijzigd`, `nieuw in B`, `vervallen in B`,
`NIET VERGELIJKBAAR`. Een mislukte uitlijning of onleesbaar bestand geeft altijd
`NIET VERGELIJKBAAR` (exitcode 2), nooit "gelijk".

Per opmerking: `waarschijnlijk verwerkt`, `verwerkt elders (controleer)`, `niet verwerkt`,
`onduidelijk`, `informatief`. 'Verwerkt elders' betekent: op de plek zelf niets, maar elders op het
blad een nieuwe tekst die erover gaat; de reden noemt die tekst en de plek.

## Licentie

Naar keuze [MIT](LICENSE-MIT) of [Apache-2.0](LICENSE-APACHE).
`pdfium.dll` valt onder de eigen licentie van PDFium (BSD-3-Clause).
