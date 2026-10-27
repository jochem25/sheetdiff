"""Bouwt de zelfstandige HTML-mockup uit template.html + lokaal gegenereerde data.

Gebruik: python -I build_mockup.py <mockupmap> [<opmerkingen.pdf>]
De mockupmap bevat data/gevels.json, img/manifest.json en img/*.png (gegenereerd met
`cargo run --release --example mockup_assets` en `sheetdiff-cli compare --json`).
De uitvoer bevat klantbeelden en hoort NIET in de repo.
"""
import base64, datetime, io, json, sys
from pathlib import Path
from PIL import Image

root = Path(sys.argv[1])
here = Path(__file__).resolve().parent
tpl = (here / "template.html").read_text(encoding="utf-8")
diff = json.loads((root / "data" / "gevels.json").read_text(encoding="utf-8"))
man = json.loads((root / "img" / "manifest.json").read_text(encoding="utf-8"))

def b64(path, jpeg=False):
    if jpeg:
        im = Image.open(path).convert("RGB")
        buf = io.BytesIO(); im.save(buf, "JPEG", quality=86, optimize=True)
        return "data:image/jpeg;base64," + base64.b64encode(buf.getvalue()).decode()
    return "data:image/png;base64," + base64.b64encode(path.read_bytes()).decode()

imgs = {}
for p in sorted((root / "img").glob("*.png")):
    imgs[p.stem] = b64(p, jpeg=p.stem.startswith("opm"))

sheets = []
for s in diff["sheets"]:
    s = dict(s); s.pop("a", None); s.pop("b", None)
    sheets.append(s)
sheets.sort(key=lambda s: s["key"])
data = {"hero": man["hero"], "sheets": sheets, "comments": man["comments"]}
logo = b64(here.parents[1] / "crates" / "sheetdiff-ui" / "assets" / "beeldmerk-96.png")
html = (tpl.replace("__DATA__", json.dumps(data, ensure_ascii=False))
           .replace("__IMG__", json.dumps(imgs))
           .replace("__LOGO__", logo)
           .replace("__PATH_A__", diff["a_root"]).replace("__PATH_B__", diff["b_root"])
           .replace("__PATH_C__", (sys.argv[2] if len(sys.argv) > 2 else ""))
           .replace("__DATE__", datetime.date.today().strftime("%d-%m-%Y")))
out = root / "sheetdiff_mockup.html"
out.write_text(html, encoding="utf-8")
print(out, round(out.stat().st_size / 1e6, 1), "MB")
