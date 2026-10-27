//! Renderthread voor de viewer. Bezit de PDF-documenten (PDFium-objecten zijn niet Send) en
//! levert per blad:
//! - een basislaag (hele blad op ~90 dpi: A, B uitgelijnd, pixelklasse) -> direct pannen/zoomen;
//! - een detaillaag (alleen het zichtbare deel, op de exacte zoom) -> scherp tot 1:1-pixel;
//! - de vectorwijzigingen (paden) voor de vectormodus.
//!
//! Nieuwe aanvragen verdringen oude: bij zoomen telt alleen het laatste beeld.

use sheetdiff_core::geom::{Affine, RectPt};
use sheetdiff_core::pdf::{self, Doc, Gray};
use sheetdiff_core::raster::{self, DiffParams};
use sheetdiff_core::vector::{self, VChange};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

#[derive(Debug, Clone, PartialEq)]
pub struct SheetSrc {
    pub a: Option<(PathBuf, usize)>,
    pub b: Option<(PathBuf, usize)>,
    pub b_to_a: Affine,
    /// Afmeting van het blad in A-weergavepunten.
    pub size_pt: (f64, f64),
}

pub struct Layer {
    pub rect_pt: RectPt,
    pub dpi: f64,
    pub w: usize,
    pub h: usize,
    pub a: Gray,
    pub b: Gray,
    pub class: Vec<u8>,
}

/// Uitsnede rond een opmerking: A/B/verschil plus de opmerkingen-PDF (met annotaties).
pub struct ZoneReq {
    pub id: usize,
    pub src: SheetSrc,
    /// Opmerkingen-PDF, pagina, en transformatie opmerkingenpagina -> A.
    pub c: Option<(PathBuf, usize, Affine)>,
    pub rect: RectPt,
    pub dpi: f64,
    pub params: DiffParams,
}

enum Req {
    Zone(ZoneReq),
    /// generatie, blad, parameters, ook vectorwijzigingen berekenen
    Sheet(u64, SheetSrc, DiffParams, bool),
    Detail(u64, RectPt, f64),
    Params(u64, DiffParams),
}

pub enum Resp {
    Zone(usize, Layer, Option<pdf::Rgba>),
    ZoneError(usize, String),
    Base(u64, Layer),
    Detail(u64, Layer),
    Vector(u64, Vec<VChange>),
    Error(u64, String),
}

pub struct RenderService {
    tx: Sender<Req>,
    pub rx: Receiver<Resp>,
    pub gen: u64,
}

impl RenderService {
    pub fn start(ctx: eframe::egui::Context) -> Self {
        let (tx, rx_req) = std::sync::mpsc::channel();
        let (tx_resp, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("sheetdiff-render".into())
            .spawn(move || {
                // een paniek in PDFium-code mag de viewer niet stil laten bevriezen: melden en
                // de thread opnieuw laten draaien met dezelfde kanalen
                let rx = std::sync::Arc::new(std::sync::Mutex::new(rx_req));
                loop {
                    let (rx2, tx2, ctx2) = (rx.clone(), tx_resp.clone(), ctx.clone());
                    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                        let guard = rx2.lock().expect("render-ontvanger");
                        worker(&guard, tx2, ctx2)
                    }));
                    match r {
                        Ok(()) => return, // UI is weg
                        Err(e) => {
                            let msg = e
                                .downcast_ref::<String>()
                                .cloned()
                                .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                                .unwrap_or_else(|| "onbekende fout".into());
                            let _ =
                                tx_resp.send(Resp::Error(0, format!("renderfout (paniek): {msg}")));
                            ctx.request_repaint();
                        }
                    }
                }
            })
            .expect("renderthread");
        Self { tx, rx, gen: 0 }
    }
    /// Nieuw blad: basislaag en vectorwijzigingen worden gemaakt.
    pub fn open(&mut self, src: SheetSrc, params: DiffParams) -> u64 {
        self.open_with(src, params, true)
    }
    /// Zonder vectorwijzigingen (bijv. live voorbeeld bij handmatig uitlijnen).
    pub fn open_with(&mut self, src: SheetSrc, params: DiffParams, vector: bool) -> u64 {
        self.gen += 1;
        let _ = self.tx.send(Req::Sheet(self.gen, src, params, vector));
        self.gen
    }
    pub fn zone(&self, z: ZoneReq) {
        let _ = self.tx.send(Req::Zone(z));
    }
    pub fn detail(&self, rect: RectPt, dpi: f64) {
        let _ = self.tx.send(Req::Detail(self.gen, rect, dpi));
    }
    /// Tolerantie gewijzigd: basislaag opnieuw.
    pub fn params(&mut self, params: DiffParams) {
        let _ = self.tx.send(Req::Params(self.gen, params));
    }
}

/// Basis-dpi: hoogstens 100, en de langste zijde hoogstens 4096 px (textuurgrens, geheugen).
pub fn base_dpi(size_pt: (f64, f64)) -> f64 {
    (4096.0 * 72.0 / size_pt.0.max(size_pt.1)).min(100.0)
}

fn render_layer(
    docs: &HashMap<PathBuf, Doc>,
    src: &SheetSrc,
    rect: RectPt,
    dpi: f64,
    params: &DiffParams,
) -> anyhow::Result<Layer> {
    let s = dpi / 72.0;
    let w = ((rect.width() * s).round() as usize).max(1);
    let h = ((rect.height() * s).round() as usize).max(1);
    let m = params.radius_px + 1;
    let to_px = Affine::translate(-rect.x0, -rect.y0)
        .then(&Affine::scale(s))
        .then(&Affine::translate(m as f64, m as f64));
    let (pw, ph) = (w + 2 * m, h + 2 * m);
    // Een kant die er hoort te zijn maar niet te laden is, is een FOUT, geen wit blad:
    // anders toont de viewer "alles erbij" of "alles weg" (review 09-10 #1).
    let page =
        |p: &Option<(PathBuf, usize)>, side: &str| -> anyhow::Result<Option<pdf::PdfPage<'_>>> {
            match p {
                None => Ok(None),
                Some((f, i)) => {
                    let d = docs
                        .get(f)
                        .ok_or_else(|| anyhow::anyhow!("{side} niet te openen: {}", f.display()))?;
                    Ok(Some(d.page(*i)?))
                }
            }
        };
    let ga = match page(&src.a, "A")? {
        Some(p) => pdf::render_gray(&p, &to_px, pw, ph)?,
        None => Gray::new(pw, ph, 255),
    };
    let gb = match page(&src.b, "B")? {
        Some(p) => pdf::render_gray(&p, &src.b_to_a.then(&to_px), pw, ph)?,
        None => Gray::new(pw, ph, 255),
    };
    let d = raster::diff(&ga, &gb, params);
    let crop = |data: &[u8]| -> Vec<u8> {
        let mut v = Vec::with_capacity(w * h);
        for y in m..m + h {
            v.extend_from_slice(&data[y * pw + m..y * pw + m + w]);
        }
        v
    };
    Ok(Layer {
        rect_pt: rect,
        dpi,
        w,
        h,
        a: Gray {
            w,
            h,
            data: crop(&ga.data),
        },
        b: Gray {
            w,
            h,
            data: crop(&gb.data),
        },
        class: crop(&d.class),
    })
}

fn worker(rx: &Receiver<Req>, out: Sender<Resp>, ctx: eframe::egui::Context) {
    let mut docs: HashMap<PathBuf, Doc> = HashMap::new();
    let mut cur: Option<(u64, SheetSrc, DiffParams)> = None;
    let mut want_base = false;
    let mut want_vector = false;
    let mut want_detail: Option<(RectPt, f64)> = None;
    let mut want_zone: Option<ZoneReq> = None;
    let send = |r: Resp| {
        let ok = out.send(r).is_ok();
        ctx.request_repaint();
        ok
    };
    loop {
        // wachten als er niets te doen is, daarna alles wat klaarstaat ophalen
        let idle = !want_base && !want_vector && want_detail.is_none() && want_zone.is_none();
        let mut msgs = Vec::new();
        if idle {
            match rx.recv() {
                Ok(m) => msgs.push(m),
                Err(_) => return,
            }
        }
        while let Ok(m) = rx.try_recv() {
            msgs.push(m);
        }
        for m in msgs {
            match m {
                Req::Zone(z) => {
                    for f in z
                        .src
                        .a
                        .iter()
                        .chain(z.src.b.iter())
                        .map(|x| &x.0)
                        .chain(z.c.iter().map(|x| &x.0))
                    {
                        if !docs.contains_key(f) {
                            match Doc::open(f) {
                                Ok(d) => {
                                    docs.insert(f.clone(), d);
                                }
                                Err(e) => {
                                    send(Resp::ZoneError(z.id, format!("{e:#}")));
                                }
                            }
                        }
                    }
                    want_zone = Some(z);
                }
                Req::Sheet(g, src, p, vec) => {
                    // open documenten begrenzen: alleen die van het nieuwe blad houden als er
                    // veel open staan (uitgiftes met een PDF per blad)
                    if docs.len() > 24 {
                        let keep: Vec<PathBuf> = src
                            .a
                            .iter()
                            .chain(src.b.iter())
                            .map(|x| x.0.clone())
                            .collect();
                        docs.retain(|k, _| keep.contains(k));
                    }
                    for (f, _) in src.a.iter().chain(src.b.iter()) {
                        if !docs.contains_key(f) {
                            match Doc::open(f) {
                                Ok(d) => {
                                    docs.insert(f.clone(), d);
                                }
                                Err(e) => {
                                    send(Resp::Error(g, format!("{e:#}")));
                                }
                            }
                        }
                    }
                    cur = Some((g, src, p));
                    want_base = true;
                    want_vector = vec;
                    want_detail = None;
                }
                Req::Detail(g, r, dpi) => {
                    if cur.as_ref().is_some_and(|c| c.0 == g) {
                        want_detail = Some((r, dpi));
                    }
                }
                Req::Params(g, p) => {
                    if let Some(c) = cur.as_mut().filter(|c| c.0 == g) {
                        c.2 = p;
                        want_base = true;
                    }
                }
            }
        }
        if let Some(z) = want_zone.take() {
            let resp = match render_layer(&docs, &z.src, z.rect, z.dpi, &z.params) {
                Ok(l) => {
                    let c = z.c.as_ref().and_then(|(f, i, c_to_a)| {
                        let page = docs.get(f)?.page(*i).ok()?;
                        let s = z.dpi / 72.0;
                        let to_px = c_to_a
                            .then(&Affine::translate(-z.rect.x0, -z.rect.y0))
                            .then(&Affine::scale(s));
                        pdf::render_rgba(&page, &to_px, l.w, l.h, true).ok()
                    });
                    Resp::Zone(z.id, l, c)
                }
                Err(e) => Resp::ZoneError(z.id, format!("uitsnede: {e:#}")),
            };
            if !send(resp) {
                return;
            }
            continue;
        }
        let Some((g, src, p)) = cur.clone() else {
            want_base = false;
            want_vector = false;
            want_detail = None;
            continue;
        };
        if want_base {
            want_base = false;
            let dpi = base_dpi(src.size_pt);
            let rect = RectPt::new(0.0, 0.0, src.size_pt.0, src.size_pt.1);
            let r = match render_layer(&docs, &src, rect, dpi, &p) {
                Ok(l) => Resp::Base(g, l),
                Err(e) => Resp::Error(g, format!("{e:#}")),
            };
            if !send(r) {
                return;
            }
            continue; // eerst kijken of er al iets nieuws is
        }
        if let Some((r, dpi)) = want_detail.take() {
            let resp = match render_layer(&docs, &src, r, dpi, &p) {
                Ok(l) => Resp::Detail(g, l),
                Err(e) => Resp::Error(g, format!("{e:#}")),
            };
            if !send(resp) {
                return;
            }
            continue;
        }
        if want_vector {
            want_vector = false;
            let run = || -> anyhow::Result<Vec<VChange>> {
                let page = |p: &Option<(PathBuf, usize)>| {
                    p.as_ref()
                        .and_then(|(f, i)| docs.get(f).and_then(|d| d.page(*i).ok()))
                };
                let (Some(pa), Some(pb)) = (page(&src.a), page(&src.b)) else {
                    anyhow::bail!("blad komt maar in een uitgifte voor (of is niet te laden): geen vectorvergelijking");
                };
                let (va, _) = vector::extract(&pa, &pdf::page_geom(&pa)?)?;
                let (vb, _) = vector::extract(&pb, &pdf::page_geom(&pb)?)?;
                Ok(vector::compare(
                    &va,
                    &vb,
                    &src.b_to_a,
                    &vector::VParams::default(),
                ))
            };
            let resp = match run() {
                Ok(v) => Resp::Vector(g, v),
                Err(e) => Resp::Error(g, format!("vector: {e:#}")),
            };
            if !send(resp) {
                return;
            }
        }
    }
}
