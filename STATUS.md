# STATUS

Laatst bijgewerkt: 2026-11-07

| Mijlpaal | Stand |
|---|---|
| Mockup (HTML) | akkoord Jochem 09-10 |
| M1 render + uitlijning + pixeldiff (CLI) | klaar: volledige uitgifte 260615->260624 (167 bladen, 63 A0-paren) in 82 s met 8 werkerprocessen |
| M2 viewer met overlay | klaar: egui-app (5 schermen), overlay/knipperen/naast elkaar/pixel/vector, 1:1-pixelzoom via detaillaag; screenshots buiten de repo |
| M3 opmerkingen aannemer | klaar: in app en CLI; op 71 door Jochem afgevinkte opmerkingen van een aannemer 77 % 'verwerkt', 10 % 'niet', 13 % 'onduidelijk' (docs/acceptatie-v1.md) |
| M4 export + acceptatie | klaar: docs/acceptatie-v1.md; 31-08-referentie niet toetsbaar (print weg) |
| Release 1.1.0 | gepubliceerd 07-11 (O-2026-11-07-01): tag v1.1.0, een samengevoegde commit op origin, zie onder |
| Release v1.0.0 | gepubliceerd 10-10 na akkoord Jochem ('erg goed, heel bruikbaar'): GitHub-release v1.0.0, zie onder |

## Release 1.1.0 (07-11-2026, O-2026-11-06-01 en O-2026-11-07-01)

- Dual review per crate (qwen3.6:35b en Sonnet): ship with fixes. Twee bugs bevestigd en gefixt (eceb05a, f3de53e).
- Testdata neutraal (O-2026-11-07-01): geen projectnaam of tekenaarsnaam meer in de testfixtures en geen
  projectspecifieke tekst in deze STATUS. 111 tests groen, golden 4/4. Installer opnieuw gebouwd, versie 1.1.0.
- Gepubliceerd via route B (besluit Jochem 06-11): op origin/main staat EEN samengevoegde commit, kind van
  dfd41d6 (1.0.0), met precies de boom van de lokale commit die deze sectie invoerde
  ('STATUS: 1.1.0 uitgebracht'). De volledige history blijft alleen lokaal; controle:
  `git diff <die lokale commit> origin/main` is leeg. Tag `v1.1.0` staat op de publieke commit.
- Assets: sheetdiff-setup-1.1.0.exe (niet ondertekend, SmartScreen) en een portable zip.
- Remote prive niet bijgewerkt: deelt geen history met origin; main pushen zou force vragen.

## Release v1.0.0 (10-10-2026)

- Tag `v1.0.0`; assets: sheetdiff.exe, sheetdiff-cli.exe, pdfium.dll (PDFium BSD-3, chromium/8086),
  LICENSE-pdfium.txt, LEESMIJ.txt en een zip met alles plus `licenties/` (meegebouwde bibliotheken).
- Geen testdata of tekeningen in de release. LEESMIJ staat ook in `docs/release/LEESMIJ.txt`.
- Gebouwd met `cargo build --release` op de tagcommit; tests groen (31 + 4 golden ignored); smoketest
  van de losse map (CLI `page` op twee gegenereerde PDF's, exit 1 'gewijzigd').
- Volgende stap: vragen/opmerkingen van Jochem (week 12-10) verzamelen; zie TODO 'Open na v1'.

## Bladen koppelen zonder tekeningnummer (O-2026-11-04-01, 04-11-2026)

- Bouwnummer als tweede herkenning (`identify.building_regex`, `building_file_regex`). Volgorde:
  tekeningnummer in titelhoek > bouwnummer in titelhoek > tekeningnummer elders in de tekstlaag >
  bestandsnaam. In de bestandsnaam gaat het bouwnummer voor het tekeningnummer.
- Varianten: Basis, OptieA en zwart dragen hetzelfde bouwnummer maar zijn andere bladen. De variant
  uit de bestandsnaam (`identify.variant_regex`) komt in de sleutel: `Z.09.09_A1_09 [OptieA_zwart]`.
- Restkoppeling: vervangen door koppelen op inhoud (O-2026-11-05-01, hieronder).
- Handmatig koppelen in de UI: bij een vervallen of nieuw blad, rechterpaneel. Geldt tot de volgende
  vergelijking; opmerkingen worden niet opnieuw geanalyseerd.
- Acceptatie op het referentieproject: twee uitgiftes van een verkoopblad: 1 blad 'gewijzigd', zelfde sleutel,
  tekst 1,1-1,4 mm verplaatst. Twee volledige verkoopsets: 77 bladen gekoppeld, 76 'gewijzigd' en 1
  'NIET VERGELIJKBAAR' (uitlijning 55,7 % overlap).

## Koppelen op inhoud en op paginavolgorde (O-2026-11-05-01, 05-11-2026)

- Na koppelen op sleutel scoort `pairing::pair_sheets` alle overgebleven bladen A x B en wijst
  globaal optimaal toe (Hongaars). Score = 0,6 x IoU van de inkt (40 dpi, gedilateerd, beste van 'zoals
  het ligt' en 'op de profielen gelegd') + 0,4 x Jaccard op woorden (zonder datums en stempels).
  Bladformaat is een harde voorwaarde (1,5 %).
- Een naam zonder nummer (`?bestand#pagina`) bewijst niets en doet ook mee: een los bestand met
  dezelfde naam kaapt het juiste paar niet weg. Bij gelijke score beslissen naam en paginavolgorde.
- Twee verschillende nummers op dezelfde inhoud worden altijd 'twijfel': hernummerd of de verdieping
  erboven is niet te zien.
- Inktkaarten worden per blad een keer gemaakt, alleen voor bladen die op inhoud moeten koppelen, in
  werkerprocessen. Een uitgifte die volledig op nummer koppelt kost dus niets extra. Afwijking van de
  opdracht ('tijdens de scan'): de scan is serieel en zou 0,4 s per blad duurder worden voor elk blad.
- `compare --pair-by-order` en de knop 'Vergelijk op paginavolgorde': pagina i tegen pagina i, per
  paar een waarschuwing onder `pair.min_score`.
- UI: ontkoppelen van elk paar. Handmatige keuzes bewaren is niet klein (herkenning over twee
  scans heen) en blijft TODO.
- Gedragswijziging: twee losse bladen die onder `min_score` scoren worden niet meer gekoppeld. Twee
  willekeurige pagina's naast elkaar leggen kan met `page`, of met de knop op paginavolgorde.

### Kalibratie

`pair.min_score` = 0,50, `pair.doubt_score` = 0,70. Gemeten met `sheetdiff-cli pairs --csv`.

| set | juiste paren: n, min, mediaan | onjuiste paren: n, mediaan, p99, max |
|---|---|---|
| project K, volledige uitgifte | 21, 0,817, 0,956 | 222, 0,312, 0,562, 0,588 |
| referentieproject, uitgifte, blind | 62, 0,704, 0,965 | 1833, 0,344, 0,907, 0,941 |
| referentieproject, verkoopset (bladen zonder nummer in A of B) | 32, 0,571, 0,926 | 1024, 0,158, 0,756, 0,814 |
| project K tegen referentieproject, formaatvoorwaarde uit | - | 6505 paren in 3 sets, max 0,312 |

- `min_score`: boven alles van een ander project (0,312), onder het zwakste juiste paar (0,571: blad
  echt anders ingedeeld, lijnen 0,33, tekst 0,92).
- `doubt_score`: boven de onjuiste paren van verschillende bladsoorten (tot 0,63), onder de juiste
  paren op een na (de volgende is 0,704).
- Grens: herhaalde verdiepingen zijn verschillende bladen die tot 0,94 scoren, hoger dan sommige juiste
  paren. Geen drempel scheidt die; dat doet alleen de toewijzing, zolang het juiste blad er is.

### Acceptatie

| toets | uitkomst |
|---|---|
| project K, boekje 16 bladen, andere bestandsnaam | 16 gekoppeld i op i, 15 gewijzigd + 1 gelijk, per blad gelijk aan het referentierapport |
| project K, volledige uitgifte (6 pdf, 21 bladen) | 21 gekoppeld, 20 gewijzigd, 1 gelijk, 0 los; op paginavolgorde dezelfde telling, 0 waarschuwingen |
| los bestand met dezelfde naam in B | vervanger: 1 blad uit het boekje als los bestand; juiste paar blijft, los blad wordt nieuw. Het originele bestand was niet meer beschikbaar |
| referentieproject blind, verkoopset 110 x 231 | 109 van 109 paren gelijk aan de koppeling op naam |
| referentieproject blind, uitgifte 166 x 78 | 62 paren, 49 zelfde pagina, 12 een andere versie van hetzelfde nummer, 1 fout (verdieping) |
| referentieproject blind, set zonder nummers 380 x 430 | 146 gelijk; 234 wisselen tussen bladen met dezelfde score (binnen 0,002). Op naam geen afwijking: daar beslist de naam |
| negatief: boekje tegen gevelblad, en project tegen project | 0 paren, ook met formaatvoorwaarde uit |
| verkoopsets op naam (O-2026-11-04-01: 77 gekoppeld) | 109 gekoppeld: 32 extra op inhoud, waarvan 1 twijfel; 1 blad zonder tegenhanger |

Tijd bij oplevering O-2026-11-05-01, zie punt e voor de stand nu (32 logische kernen, 16 werkers): 166 x 166 bladen 10,0 s extra, waarvan scoren en toewijzen
0,02 s; de rest is renderen. 380 x 430 bladen: 28 s. Doel was < 10 s bij 167 x 167: net niet gehaald.
Anti-aliasing uit scheelde niets (9,0 tegen 9,4 s).

## Open punten na v1 (O-2026-11-05-04, 05-11-2026)

### a. Handmatige koppelingen bewaren

- Keuzebestand per vergelijking: `<B> vs <A>.sheetdiff.json`, in de map van B of naast het bestand B.
  CLI: `--choices <bestand>`, `--no-choices`. Bevat koppelingen en ontkoppelingen.
- Waarom daar: een koppeling geldt voor precies twee uitgiftes, dus niet in een profiel (dat is voor
  instellingen en geldt voor elk paar). Bij B reist hij mee met de uitgifte, en het is hetzelfde patroon
  als het statusbestand naast de opmerkingen-PDF. Is de map alleen-lezen, dan meldt de app dat en geldt
  de keuze alleen voor deze vergelijking.
- Herkenning over scans heen: bestandsnaam + pagina, bewaakt met de inhoudshandtekening. Sleutels
  kunnen niet: die veranderen door het koppelen. Hernoemd bestand: gevonden als enig blad met die
  handtekening. Blad gewijzigd of weg: keuze niet toegepast, waarschuwing in rapport en log.
- De handtekening is nu FNV-1a; `DefaultHasher` belooft niets over builds heen.
- Ontkoppelen is een verbod op dat paar: naam en inhoud koppelen het niet opnieuw. De bladen zelf
  mogen wel aan een ander blad koppelen.
- Opmerkingen worden na koppelen en ontkoppelen automatisch opnieuw geanalyseerd; statussen blijven
  staan. `comments::analyze` volgt nu de koppeling: een opmerkingenpagina met nummer X vindt het blad
  ook als dat na koppelen de sleutel van B draagt.
- Toets: OVK_400 met 1 koppeling (13 op 12) en 2 ontkoppelingen, CLI `pairs`: koppeling 'handmatig',
  14 paren op inhoud ongewijzigd, A 12 en B 13 los. Opmerkingen referentieproject met A TO-217z op B TO-216z:
  opmerkingenpagina 12 (TO-217z) gaat naar het gekoppelde blad, voorstellen ongewijzigd: op die twee
  bladen staan geen opmerkingen. De heranalyse op een blad met opmerkingen is dus niet aangetoond op echte
  data, wel de sleutelvertaling (unit-test).
- Niet getoetst: de knoppen in de app zelf (geen UI-test); de onderliggende functies wel.

### b. Handmatige uitlijning afmaken

- Bewaard in hetzelfde keuzebestand (`aligns`), per bladpaar. Toegepast na het koppelen, want een
  uitlijning hoort bij een paar; is het paar weg, dan waarschuwing en automatisch uitlijnen.
- Vrije rotatie: elke hoek, kwartslagen plus een rest om het midden van het gedraaide blad.
- CLI: `compare`, `pairs`, `comments` lezen hem via `--align-file` (= `--choices`) of het standaardbestand;
  `page` en `overlay` via `--align rot,schaal,dx,dy` of `--align-file`. Opmerkingen gebruiken hem ook.
- Oude fout opgelost: een handmatige registratie had overlap NaN. Dat wordt `null` in JSON en een werker
  had die regel niet kunnen afleveren. De overlap wordt nu echt gemeten en getoond.
- Toets: blad uit OVK_400 in de PDF 2 graden gedraaid. Automatisch: NIET VERGELIJKBAAR (56,2 %). Met
  `--align=2,1,0,0`: overlap 100,0 %, 0 regio's op 300 dpi; ook via `compare` met werkers en `overlay`.
- Grens: het oordeel blijft dan 'gewijzigd', want PDFium knipt schuine tekst anders in stukken dan rechte
  (10 tekstwijzigingen, 0 pixels). De melding 'tekst gewijzigd zonder zichtbaar pixelverschil' staat erbij.
- Niet getoetst: het venster in de app.

### c. Verkoopblad dat niet uitlijnt (55,7 %): blad echt anders, geen fout in de uitlijning

- Blad Z.02.09_F3_02, 20-05 tegen 02-09. Het blad van 20-05 mist de plattegrond: 1269 tekenobjecten,
  tegen 9610 in de print van 21-05 en 9705 op 02-09. Daarnaast zijn renvooi, sleutelplan en
  gevelbenamingen anders. Kader, logo en gevels liggen wel op elkaar: de gevonden uitlijning (identiteit) klopt.
- De melding blijft dus NIET VERGELIJKBAAR. Een lagere drempel kan niet: dit blad scoort 0,557 en onjuiste
  paren halen tot 1,0 (herhaalde verdiepingen). Per richting scheidt ook niet: 66 % en 46 % hier, onjuiste
  paren tot 79,5 % (160 onjuiste paren, 96 juiste, 25 van een ander project; 1 juist paar onder 0,6).
- Wel verbeterd: de melding geeft beide richtingen en wijst op handmatig uitlijnen. Met `--align=0,1,0,0`
  (of de knop in de app, bewaard in het keuzebestand) wordt het 'gewijzigd': 3,0 %, 181 regio's.
- Ook G2_opties (58,4 %) en F3_opties (36,2 %, twijfelpaar) in 21-05 tegen 02-09 zijn NIET VERGELIJKBAAR;
  die zijn niet per blad onderzocht.

### d. Bladen die een kwartslag gedraaid zijn

- Formaat is nu 'gelijk, of gelijk na een kwartslag'. Past het pas na een kwartslag, dan scoort het lijnwerk
  de beste van de twee draairichtingen; de tekst telt gewoon mee. Draaikaarten worden per blad een keer
  gemaakt en alleen als aan de overkant een blad op zijn kant bestaat.
- Dezelfde drempels gelden. De uitlijning daarna kende kwartslagen al.
- 180 graden blijft buiten beeld: het formaat verraadt het niet en `register.try_180` staat uit.
- Toets tweede project, hele uitgifte met B op zijn kant (/Rotate 90 en 270, andere bestandsnamen): 21 van 21
  gekoppeld, zelfde bladen, 20 gewijzigd en 1 gelijk als rechtop. 58 verkoopbladen tegen dezelfde bladen
  gedraaid: 58 paren; onjuiste paren mediaan 0,26, zelfde verdeling als rechtop (0,27).
- Regressie uitgifte en beide verkoopsets: per blad gelijk aan de nulmeting. Gedraaide inhoud zonder /Rotate
  is via een unit-test op de inktkaart gedekt, niet op een echt blad.

### e. Koppelen op inhoud sneller

| set (blind, `pairs --no-names`) | was | eerste keer | daarna (cache) | doel |
|---|---|---|---|---|
| 166 x 78 bladen | 10,1 s | 7,3 s | 0,0 s | < 5 s |
| 380 x 430 bladen | 27,9 s | 18,4 s | 0,6 s | < 15 s |

- Doel eerste keer niet gehaald, herhaling wel. Grens: het renderwerk zelf is ca. 85 s CPU op de eerste set,
  en dat zit in het parsen van de paden, niet in de pixels: 20 dpi scheelt maar 24 %, en zou de
  gekalibreerde scores raken.
- Wachtrij in plaats van vaste verdeling: elke werker haalt de volgende taak, zwaarste bladen eerst, tot 32
  werkers. Aaneengesloten stukken per bestand waren juist trager (16,3 s): zware bladen zitten bij elkaar.
- Kaarten binair van werker naar ouder in plaats van JSON.
- Kaartencache in `%TEMP%\sheetdiff-inkt`, sleutel pad + grootte + wijzigingstijd + pagina + instellingen,
  grens 1 GiB, oudste eerst weg. Uit met `pair.ink_cache = false`. In de praktijk wordt uitgifte A vaak nog
  eens vergeleken, dus dat scheelt ook de eerste keer van de volgende vergelijking.
- Onderzocht, niet gedaan: tekst vooraf laten beslissen. Op de uitgifte 39 van 62 bladen foutloos, op de
  verkoopset 2 van 380 (herhaalde woningen), dus geen winst waar het traag is.
- Vergelijken gebruikt dezelfde wachtrij: uitgifte 190 naar 156 s. Regressie per blad gelijk aan de nulmeting
  (OVK_400, tweede project, uitgifte, verkoopset 21-05); koppeling blind gelijk aan die van voor de wijziging.

### f. Buurbladen vooraf renderen

- Drie werkerprocessen maken de basislaag van de bladen +1, -1, +2 en -2. Tien lagen blijven in het
  geheugen. Is een laag nog in de maak, dan wacht de renderthread erop (tot 5 s) in plaats van opnieuw te
  beginnen. Basislaag en `SheetSrc` verhuisd naar core (`layer.rs`) zodat een werker ze kan maken.
- Gemeten in de app (`SHEETDIFF_TIMING`, `SHEETDIFF_WALK`), van openen tot textuur, 2 s per blad:

| set | zonder | met |
|---|---|---|
| tweede project | ca. 550 ms, uitschieters 1150-1700 ms | 14-61 ms |
| uitgifte A0 | niet gemeten | 42-69 ms; eerste blad 1622 ms |

- Dat is rustig bladeren. Wie sneller bladert dan een laag kost (A0 ca. 1,5 s), loopt de werkers voorbij en
  wacht alsnog. `SHEETDIFF_NO_PREFETCH` zet het uit.
- De lagen zijn gemaakt met de tolerantie van het profiel; bij een andere tolerantie wordt gewoon gerenderd.

### g. Scan sneller

| set | scan was | nu |
|---|---|---|
| uitgifte 166 x 78 (A + B) | 72 s | 26 s |
| verkoopset 380 x 430 (A + B) | 105 s | 10 s |
| uitgifte hele `compare` | 190 s (na punt e 156 s) | 110 s |

- Lezen in stukken van 4 pagina's over de wachtrij; samenvoegen tot bladen blijft in de ouder, in vaste
  volgorde, dus dezelfde uitkomst. De 64 dubbele bladen worden vooraf bepaald en gaan parallel.
- De rest van de 26 s zijn een paar zware gevelparen (tot ca. 20 s per paar in de vectorvergelijking).
- Geprobeerd en teruggedraaid: pixels vooraf vergelijken. Die gaf 83 in plaats van 78 bladen, want een
  printstempel verandert pixels zonder dat het blad anders is. Bit-gelijk komt om dezelfde reden niet voor.
- `sheetdiff-cli scan` blijft in een proces; `compare`, `pairs`, `comments` en de app gebruiken de werkers.
- Regressie per blad en alle waarschuwingen gelijk aan de nulmeting (uitgifte, verkoopset 21-05).

### h. Werkertimeout

- De timeout is per blad (`compare.sheet_timeout_s`, standaard 600 s), niet per werker. Hangt een blad, dan
  stopt alleen die werker, wordt dat blad NIET VERGELIJKBAAR met de melding 'vastgelopen', en krijgt de
  volgende taak een nieuwe werker. Voor de wachtrij verloor een vastgelopen werker al zijn resterende bladen.
- Geldt ook voor crashes: een blad dat PDFium laat vallen raakt alleen zichzelf.
- Een vastgelopen inktkaart wordt niet opnieuw in de app zelf geprobeerd; dat blad koppelt op tekst.
- Tests: `tests/pool.rs` met een nepwerker (hangen, wegvallen, niet te starten). Op echte bladen: 4 A0-gevels met timeout 1 s: alle 4 NIET
  VERGELIJKBAAR met de melding, exit 2, klaar in 3,6 s. Een mix van hangend en goed op echte data is niet gedraaid; die zit in de nepwerkertest.

### i. Titelhoek bij verschoven kader en titelherkenning

- Is een deel van de titelhoek verschoven, dan telt nu het hele titelvak als kader (12 mm rond die delen,
  binnen de titelhoek). Een regel die echt wijzigde, is zelf geen verschoven regio en werd eerst onder de
  transformatie van de tekening vergeleken, met een verkeerde buur als gevolg.
- TO-213z: was 'aanvullingen X' -> '24-06-2026'. Nu '15-06-2026' -> '24-06-2026' en 'aanvullingen X' ->
  'aanvulling x'; de printstempel telt als stempel. Tekstwijzigingen 282 naar 247.
- Titel: `identify.title_skip_regex` slaat windrichtingen, veldnamen (eindigend op ':'), adressen, 'geprint',
  'ongeschikt voor', 'definitief' en 'concept' over. Op de uitgifte: 'west' (6x) en 'Tekeningnr:' (2x) weg.
  Zonder echte titel in het titelvak blijft er een gok over (bijv. opdrachtgever); dat is niet opgelost.
- Regressie per blad gelijk aan de nulmeting: titels raken de sleutels dus niet op deze sets.

### j. Opmerkingen: verwerkt op een andere plek

- Nieuw voorstel 'verwerkt elders (controleer)'. Alleen bij 'niet verwerkt' of 'onduidelijk': staat er buiten de
  zone op hetzelfde blad een nieuwe of gewijzigde tekst met minstens 2 onderwerpwoorden uit de opmerking,
  of een woord dat een plek aanwijst (renvooi, legenda), dan wordt het dit voorstel. De reden noemt de tekst en
  de plek in mm. Woordlijsten en drempels staan in het profiel (`comments.elsewhere_*`).
- Alleen tekst: een icoon of symbool in een renvooi ziet dit niet, en andere bladen worden niet doorzocht.
- Toets op de vier opmerkingen-PDF's (116 opmerkingen): 1 verschuiving, 'icoon toevoegen in het renvooi' op TO-100n
  van niet verwerkt naar elders, verwijzing 'Renvooi Glas'. Die opmerking is door Jochem afgevinkt. De overige
  voorstellen zijn gelijk aan de nulmeting. Eén treffer zegt weinig over de betrouwbaarheid; de verwijzing is
  hier een andere regel in hetzelfde renvooi, dus de koppeling is grof.
- Niet gedaan: knop in de app om naar de plek te springen (`Comment.elsewhere` is er wel).

### Eindstand O-2026-11-05-04

- Tests: 68 unit, 16 koppelen, 3 wachtrij, 5 registratie, 4 snapshots, 4 golden; clippy schoon.
- Acceptatie per blad gelijk aan de nulmeting met exe 1.0.2-dev: OVK_400 15 + 1, tweede project 20 + 1, uitgifte 62 gewijzigd,
  verkoopset 21-05 107 + 2 niet vergelijkbaar (109 gekoppeld), verkoopset 29-05 380 gewijzigd + 50 nieuw.
- `compare`: uitgifte 190 naar 108 s, verkoopset 29-05 608 naar 360 s, verkoopset 21-05 73 naar 64 s.
- Exe's: `D:\dev\shared\uitvoer\sheetdiff\release-1.0.3-dev`. Versienummer in Cargo.toml nog 1.0.0.

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
- PDF's met tekst maken in parallelle tests breekt PDFium af (bad_variant_access), ondanks de mutex
  van pdfium-render; `tests/pairing.rs` draait daarom een test tegelijk.
- UI: automatische screenshotmodus `SHEETDIFF_AUTOSHOT=<map>` (+ SHEETDIFF_A/B/C/SHEET/WALK/TIMING) loopt alle schermen langs.

## Weergave, KBA-nummers, installer en icoon (O-2026-11-05-05, 06-11-2026)
- k1 Weergavenaam: `SheetDiff::name()` = nummer + titel, anders bestandsnaam zonder extensie, ' p.n' alleen bij een bestand met meer pagina's. Herkenning als klein label ('op naam', 'op inhoud', 'geen nummer'). Sleutels intern ongewijzigd; rapport, csv, xlsx en wolken tonen de korte naam. Meldingen zonder paginatelling tonen altijd ' p.n'.
- k2 KBA-nummers: `identify.field_label_regex` + `field_regex`; alleen een nummer naast of onder de veldnaam telt, zodat een boekjesnummer elders niet meedoet. Over elkaar geprinte tekst ('OVK_099OVK_099') wordt een keer gelezen. Titel: A1/A2-kader ja, A3-detailkader heeft geen titelveld (titel leeg). Gemeten: tweede project 28-10 en wijzA, alle tekeningen herkend behalve OVK_000 (geen nummer in de tekstlaag), titels goed; referentieproject 260624 scan voor/na gelijk (94 regels). De app bewaart voortaan alleen de vier instelbare waarden, anders hield een oude sessie nieuwe patronen tegen.
- k3 Bladenlijst: drie kolommen (blad, status, herkend); paneel start op 500 en is versleepbaar, de naamkolom groeit mee. Tooltip met volledige naam en bestand + pagina per kant.
- k4 Melding 'map niet te lezen': oorzaak: de melding bleef staan na een mislukte poging met een onthouden pad, want een nieuwe vergelijking wiste hem niet. Fix: wissen bij start; onthouden paden die niet meer bestaan gaan bij het opstarten uit de velden met een zachte melding; mislukte opmerkingen na een geslaagde vergelijking zijn een waarschuwing; 'bestand' of 'map' volgens het pad. Testruns (SHEETDIFF_AUTOSHOT) schrijven de bewaarde paden niet meer weg: daar stond een pad uit een agenttest in.
- k5 Foutteksten: `errors::nl` zet de foutketen om (niet gevonden, geen toegang, in gebruik, schijf vol, netwerk, anders 'systeemfout n'); PDFium-weigeringen idem. Geldt voor app, CLI en werkers.
- l Installer: `installer/sheetdiff.iss` + `build-installer.ps1`, versie uit Cargo.toml (1.1.0), niet ondertekend. ISCC stond al op deze pc (per gebruiker), winget was niet nodig. Uitvoer buiten de repo: uitvoer/sheetdiff/installer/sheetdiff-setup-1.1.0.exe (13 MB). Getest (stil, 06-11): installeren zonder beheerdersrechten naar %LOCALAPPDATA%/Programs/sheetdiff, start via startmenu, OVK_400 28-10 tegen wijzA (16 bladen), overinstalleren (1 uninstaller, 1 Apps-regel), de-installeren: map, startmenu en Apps-regel weg. Blijft staan: %APPDATA%/sheetdiff/data/app.ron (instellingen), %TEMP%/sheetdiff-inkt (cache), keuzebestanden bij uitgifte B, statusbestanden bij de opmerkingen-PDF. Bureaublad-optie staat standaard aan; getest onder de testnaam sheetdiff-test om de bestaande snelkoppeling niet te raken. LET OP: die heet ook sheetdiff.lnk, dus een installatie met de optie aan overschrijft hem en de-installeren haalt hem weg. SmartScreen: de setup is niet ondertekend; bij een gedownload bestand is 'Windows heeft uw pc beveiligd' te verwachten (Meer informatie > Toch uitvoeren). Niet gemeten: de lokale testkopie heeft geen downloadmarkering.
- m Icoon: `assets/icon/` (ico, png 256, svg; eigen werk), exe-icoon via build.rs (winresource) voor app en cli, venstericoon uit de png, installer met SetupIconFile. Oude `crates/sheetdiff-ui/assets/icon-256.png` weg. Gecontroleerd op de geinstalleerde 1.1.0 met schermafdrukken in uitvoer/sheetdiff/screenshots/m-icoon: verkenner, taakbalk met draaiende app, startmenu. Na de test weer gede-installeerd.
