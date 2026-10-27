# Testdata (buiten de repo)

De golden-tests draaien op echte tekeningen van een referentieproject. Die tekeningen horen nooit in
de repo. De paden komen uit omgevingsvariabelen; het handigst in een lokale `.cargo/config.toml`
(staat in `.gitignore`):

```toml
[env]
SHEETDIFF_TESTDATA        = "C:/pad/naar/testdata"            # synthetisch opmerkingenbestand
SHEETDIFF_GOLDEN_ISSUE_A  = "C:/pad/naar/uitgifte-A"          # map met boekjes, oude uitgifte
SHEETDIFF_GOLDEN_ISSUE_B  = "C:/pad/naar/uitgifte-B"          # map met boekjes, nieuwe uitgifte
SHEETDIFF_GOLDEN_PAIR_OLD = "C:/pad/naar/gevels_oud.pdf"      # gevels, 4 A0-bladen
SHEETDIFF_GOLDEN_PAIR_NEW = "C:/pad/naar/gevels_nieuw.pdf"    # zelfde, met '-87' op TO-201n..204n
SHEETDIFF_GOLDEN_COMMENTS = "C:/pad/naar/opmerkingen.pdf"     # opmerkingen aannemer (Bluebeam)
```

| Rol | Variabele | Opmerking |
|---|---|---|
| Uitgifte A | `SHEETDIFF_GOLDEN_ISSUE_A` | basis waarop de aannemer opmerkingen maakte; bevat `TO+_wijzA_gevels.pdf` |
| Uitgifte B | `SHEETDIFF_GOLDEN_ISSUE_B` | volgende uitgifte; bevat `TO+_wijzA_gevels.pdf` en `TO+_wijzA_gevels-noord.pdf` (zelfde print, ander boekje) |
| Opmerkingen aannemer | `SHEETDIFF_GOLDEN_COMMENTS` | Bluebeam: Polygon-wolk + FreeText; eigen afvinking als Ink (potlood) |
| Gevels-paar | `SHEETDIFF_GOLDEN_PAIR_OLD` / `_NEW` | 4 bladen A0, verwacht `-87` op TO-201n..204n |
| Synthetisch opmerkingenbestand | `SHEETDIFF_TESTDATA` | `synthetisch_opm_TO-201n.pdf`: TO-201n van A met 2 annotaties en 2 rode inhoud-markeringen, telkens 1 op een in B gewijzigde en 1 op een ongewijzigde plek |

Golden-tests: `cargo test --release -p sheetdiff-core --test golden -- --ignored` (ca. 3 min).
Ontbrekende data of een ontbrekende variabele laat de test falen, niet stil slagen.
