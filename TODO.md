# TODO

Stand 10-10-2026: v1.0.0 gereleased (GitHub, public sinds 27-10). Jochem komt week 12-10 met vragen en
opmerkingen; die eerst verzamelen en prioriteren voor er nieuw werk start.

## Open na v1

- [ ] UI: buurbladen vooraf renderen in werkerprocessen (bladwisselen nu ~0,5 s)
- [ ] UI: handmatige uitlijning afmaken - staat er (rotatie in kwartslagen, schaal, dx/dy, per
      geopend blad), maar wordt niet bewaard, geen vrije rotatie, en de CLI kent hem niet
- [ ] Scan sneller bij veel dubbele bladen (tolerante vectorvergelijking ~0,6 s per dubbeling,
      ~1 min op het referentieproject)
- [ ] Werkertimeout in parallel.rs
- [ ] Titelhoek-tekst bij verschoven kader (restgeval TO-213z datumregel)
- [ ] Titelherkenning verbeteren (nu soms 'noordwest' uit sleutelplan)
- [ ] Opmerkingen: 'verwerkt op een andere plek' (legenda/renvooi) herkennen
- [ ] Vragen/opmerkingen Jochem (week 12-10) verwerken

## Gedaan

- [x] Mockup (docs/mockup/) en akkoord Jochem (09-10)
- [x] M1: render, uitlijning, pixeldiff, volledige uitgifte, parallel over processen
- [x] Vector-spike en vectorvergelijking (`sheetdiff-cli vec`)
- [x] Verplaatst-categorie (viewport verschoven in vast kader)
- [x] M2: egui-viewer
- [x] M3: opmerkingen-PDF, voorstel per opmerking, status-json
- [x] M4: export xlsx/csv + PDF van B met opmerkingen, acceptatie (docs/acceptatie-v1.md), review ander model
- [x] Release v1.0.0 (10-10)
