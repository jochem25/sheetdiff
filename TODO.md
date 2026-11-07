# TODO

Stand 07-11-2026: v1.1.0 gereleased (GitHub, een samengevoegde commit op origin; volledige history lokaal). Vragen en opmerkingen van Jochem (week 12-10): input volgt via de planner.

## Open na v1

- [x] Push en release 1.1.0 (07-11, route B; installer niet ondertekend)
- [ ] Bureaubladsnelkoppeling naar 1.0.1-dev vervangen: de installer gebruikt dezelfde naam (Jochem)
- [ ] Bladenlijst sorteert op sleutel: p.10 staat voor p.2 bij bladen zonder nummer
- [ ] Besluit: hoe de lokale commits publiek gaan. Negen ervan bevatten projectgegevens (O-2026-11-06-01)
- [ ] Onleesbaar keuzebestand breekt vergelijking en CLI af met een fout: bewust, of waarschuwen en doorgaan?
- [ ] Onthouden pad op onbereikbare netwerkschijf: exists() bij opstarten kan blokkeren en wist het pad (niet gemeten)
- [ ] Review, nice-to-have: uitlijnpaneel blijft open na ontkoppelen als de regel gelijk blijft; sheetdiff-buur-<pid> blijft in %TEMP% (4 kB per start)
- [ ] Review, nice-to-have: keuzebestand valideert handmatige uitlijning niet (scale 0 uit met de hand bewerkte JSON)
- [ ] Review, niet gemeten: scan opent een boekje per 4 pagina's opnieuw; Worker::finish wacht zonder tijdslimiet; inktcache zonder formaatversie
- [ ] Review, nice-to-have: build.rs stopt zonder rc.exe; icoon best-effort maken voor bouwers zonder Windows SDK
- [ ] Vectorvergelijking van zware gevelparen (tot ca. 20 s per paar) bepaalt nu de scantijd bij dubbele bladen
- [ ] Opmerkingen 'verwerkt elders': ook symbolen en andere bladen; knop naar de plek in de app
- [ ] Vragen/opmerkingen Jochem (week 12-10) verwerken
- [ ] Koppelen op inhoud: eerste keer nog boven doel (166 x 78: 7,3 s, 380 x 430: 18,4 s); herhaling 0-0,6 s

## Ideeen QC-koppeling (04-11-2026, oppakken met Jochem 05-11 middag)

Volgorde voorstel: 1 > 2 > 8 > 7 > 3 > 4; 5 en 6 alleen bij vraag. Revit-kant (1-3) in een apart
pyRevit-onderdeel, niet in deze publieke repo; sheetdiff leest/schrijft alleen een open JSON
(wijzigingsgebieden in mm op het blad). Per gekozen punt eerst een opdracht (OPDRACHTEN.md) en go.

- [ ] 1. Sidecar-JSON bij PDF-export uit Revit: per blad viewports, model->blad-transform
      (Viewport.GetProjectionToSheetTransform x View.GetModelToProjectionTransforms), element-id's
      per view. Op printmoment vastleggen, achteraf niet betrouwbaar. Basis voor 2, 3, 5 en 6.
- [ ] 2. Spring naar wijziging in Revit: klik op verschil > blad openen, inzoomen op het gebied,
      elementen in het gebied selecteren. Koppeling tekeningnummer = Sheet Number.
- [ ] 3. Revisiewolken + Revision-regel automatisch, alleen voor door Jochem aangevinkte wijzigingen.
- [ ] 4. Opmerkingen-retour in de PDF van de aannemer: status (verwerkt/niet/toelichting) als
      annotatie-status + reply, leesbaar in Bluebeam. Testen in Bluebeam en een gewone reader.
- [ ] 5. BCF 2.1-export van geselecteerde opmerkingen: snapshot = overlay-uitsnede, titel, status,
      commentaar; met 1 erbij ook camera + IfcGUID's. Eerst nagaan welke partijen BCF gebruiken.
- [ ] 6. Modeloverlay: IFC/Revit-link van derden geprojecteerd op de bladen (zoals keukencontrole
      referentieproject, 04-11). Eerst als losse CLI-modus.
- [ ] 7. Bladhistorie over het archief: een blad/gebied volgen over alle uitgiftes; vingerafdruk per
      gebied bij de scan bewaren zodat N-1 vergelijkingen niet nodig zijn.
- [ ] 8. Ruismaskers per project (printstempel, datumveld, sleutelplan), zichtbaar in het rapport.
- Eerst afronden uit 'Open na v1': handmatige koppelingen/uitlijning bewaren en het niet-uitgelijnde
  verkoopblad (55,7 %). Gedaan 05-11, O-2026-11-05-04 a t/m c.

## Gedaan

- [x] Icoon: exe's, venster, installer en snelkoppelingen (O-2026-11-05-05 m)
- [x] Windows-installer (Inno Setup, per gebruiker), versie 1.1.0 (O-2026-11-05-05 l)
- [x] Engelse OS-fouten naar korte Nederlandse meldingen met pad (O-2026-11-05-05 k5)
- [x] Meldingen: niet bestaande onthouden paden stil weg, geen oude foutbalk, map/bestand correct (O-2026-11-05-05 k4)
- [x] Bladenlijst: naam in brede kolom, status en herkenning in eigen kolommen, tooltip (O-2026-11-05-05 k3)
- [x] KBA-nummers uit het veld Tekeningnr/BLAD van de titelhoek (O-2026-11-05-05 k2)
- [x] Weergavenaam: geen interne sleutels meer in UI, rapport en export (O-2026-11-05-05 k1)
- [x] Opmerkingen: voorstel 'verwerkt elders (controleer)' met verwijzing, op tekst (O-2026-11-05-04 j)
- [x] Titelhoek-tekst bij verschoven kader (TO-213z) en titelherkenning (O-2026-11-05-04 i)
- [x] Werkertimeout per blad: hangend blad NIET VERGELIJKBAAR, rest gaat door (O-2026-11-05-04 h)
- [x] Scan parallel: uitgifte 72 naar 26 s, verkoopset 105 naar 10 s (O-2026-11-05-04 g)
- [x] UI: buurbladen vooraf in werkerprocessen, bladwisselen 14-69 ms (O-2026-11-05-04 f)
- [x] Koppelen op inhoud: kwartslag gedraaide bladen (O-2026-11-05-04 d)
- [x] Verkoopblad 55,7 %: blad echt anders (plattegrond ontbreekt in de print), vastgelegd in STATUS;
      melding verbeterd (O-2026-11-05-04 c)
- [x] Handmatige uitlijning: bewaard, vrije rotatie, in de CLI (O-2026-11-05-04 b)
- [x] Handmatige bladkoppelingen en ontkoppelingen bewaren, opmerkingen daarna opnieuw analyseren
      (O-2026-11-05-04 a)
- [x] Bladen zonder herkend nummer of met andere bestandsnaam: koppelen op inhoud, knop en vlag op
      paginavolgorde, ontkoppelen in de UI (O-2026-11-05-01, 05-11)
- [x] Mockup (docs/mockup/) en akkoord Jochem (09-10)
- [x] M1: render, uitlijning, pixeldiff, volledige uitgifte, parallel over processen
- [x] Vector-spike en vectorvergelijking (`sheetdiff-cli vec`)
- [x] Verplaatst-categorie (viewport verschoven in vast kader)
- [x] M2: egui-viewer
- [x] M3: opmerkingen-PDF, voorstel per opmerking, status-json
- [x] M4: export xlsx/csv + PDF van B met opmerkingen, acceptatie (docs/acceptatie-v1.md), review ander model
- [x] Release v1.0.0 (10-10)
- [x] Bladen koppelen zonder tekeningnummer: bouwnummer, varianten, restkoppeling, handmatig (04-11)
