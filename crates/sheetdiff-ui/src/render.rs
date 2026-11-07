//! Renderthread voor de viewer. Bezit de PDF-documenten (PDFium-objecten zijn niet Send) en
//! levert per blad:
//! - een basislaag (hele blad op ~90 dpi: A, B uitgelijnd, pixelklasse) -> direct pannen/zoomen;
//! - een detaillaag (alleen het zichtbare deel, op de exacte zoom) -> scherp tot 1:1-pixel;
//! - de vectorwijzigingen (paden) voor de vectormodus.
//!
//! Nieuwe aanvragen verdringen oude: bij zoomen telt alleen het laatste beeld.
//!
//! PDFium is per proces geserialiseerd; daarom maken losse werkerprocessen de basislaag van de
//! buurbladen vooraf (zie [`Prefetch`]). Staat die klaar, dan hoeft de renderthread niets te doen.

use sheetdiff_core::geom::{Affine, RectPt};
use sheetdiff_core::layer::{render_base, render_layer};
pub use sheetdiff_core::layer::{Layer, SheetSrc};
use sheetdiff_core::parallel::{Job, Outcome, Task, Worker};
use sheetdiff_core::pdf::{self, Doc};
use sheetdiff_core::raster::DiffParams;
use sheetdiff_core::vector::{self, VChange};
use sheetdiff_core::Config;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// Zoveel werkers maken buurbladen; meer is niet nodig, de gebruiker bladert een blad tegelijk.
const PREFETCH_WORKERS: usize = 3;
/// Zoveel basislagen blijven in het geheugen (A0: ca. 35 MB per stuk).
const PREFETCH_KEEP: usize = 10;
const PREFETCH_TIMEOUT_S: u64 = 60;
/// Zo lang wacht de renderthread op een laag die al in de maak is.
const PREFETCH_WAIT_S: u64 = 5;

type LayerKey = (SheetSrc, DiffParams);

#[derive(Default)]
struct PrefetchState {
    /// Wat nog gemaakt moet worden; nieuwste vraag vervangt de oude.
    queue: VecDeque<LayerKey>,
    busy: Vec<LayerKey>,
    done: VecDeque<(LayerKey, Layer)>,
}

/// Maakt basislagen vooraf in eigen werkerprocessen.
#[derive(Clone)]
pub struct Prefetch(Arc<(Mutex<PrefetchState>, Condvar)>);

impl Prefetch {
    fn start() -> Prefetch {
        let p = Prefetch(Default::default());
        // zonder werkers werkt de viewer gewoon, alleen zonder voorsprong
        let Some((exe, dir, job)) = prefetch_job() else {
            return p;
        };
        for i in 0..PREFETCH_WORKERS {
            let (p, exe, job, dir) = (p.clone(), exe.clone(), job.clone(), dir.clone());
            let _ = std::thread::Builder::new()
                .name(format!("sheetdiff-buurblad-{i}"))
                .spawn(move || p.work(&exe, &job, &dir, i));
        }
        p
    }

    fn work(&self, exe: &Path, job: &Path, dir: &Path, id: usize) {
        let mut worker: Option<Worker> = None;
        let (lock, wake) = &*self.0;
        loop {
            let key = {
                let Ok(mut st) = lock.lock() else { return };
                loop {
                    if let Some(k) = st.queue.pop_front() {
                        st.busy.push(k.clone());
                        break k;
                    }
                    let Ok(g) = wake.wait(st) else { return };
                    st = g;
                }
            };
            if worker.is_none() {
                worker = Worker::start(exe, job).ok();
            }
            let file = dir.join(format!("{id}.laag"));
            let task = Task::Layer {
                src: key.0.clone(),
                params: key.1,
                out: file.clone(),
            };
            let layer = worker.as_mut().and_then(|w| {
                let line = serde_json::to_string(&task).ok()?;
                let timeout = Duration::from_secs(PREFETCH_TIMEOUT_S);
                match w.ask(&line, timeout) {
                    Outcome::Line(_) => Layer::from_bytes(&std::fs::read(&file).ok()?),
                    _ => None,
                }
            });
            let _ = std::fs::remove_file(&file);
            if layer.is_none() {
                if let Some(w) = worker.take() {
                    w.stop();
                }
            }
            let Ok(mut st) = lock.lock() else { return };
            st.busy.retain(|k| k != &key);
            if let Some(l) = layer {
                st.done.push_back((key, l));
                while st.done.len() > PREFETCH_KEEP {
                    st.done.pop_front();
                }
            }
        }
    }

    /// Vraagt deze bladen aan; wat nog in de rij stond en niet meer gevraagd is, vervalt.
    pub fn want(&self, keys: Vec<LayerKey>) {
        let (lock, wake) = &*self.0;
        let Ok(mut st) = lock.lock() else { return };
        st.queue.clear();
        for k in keys {
            let known = st.busy.contains(&k) || st.done.iter().any(|(d, _)| d == &k);
            if !known {
                st.queue.push_back(k);
            }
        }
        wake.notify_all();
    }

    /// De vooraf gemaakte laag van dit blad. Wordt hij nog gemaakt, dan is wachten sneller dan
    /// opnieuw beginnen: de renderthread doet dezelfde aanroepen achter een globale mutex.
    fn take(&self, key: &LayerKey, wait: Duration) -> Option<Layer> {
        let (lock, _) = &*self.0;
        let until = std::time::Instant::now() + wait;
        loop {
            {
                let st = lock.lock().ok()?;
                if let Some((_, l)) = st.done.iter().find(|(d, _)| d == key) {
                    return Some(l.clone());
                }
                if !st.busy.contains(key) {
                    return None;
                }
            }
            if std::time::Instant::now() >= until {
                return None;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn prefetch_job() -> Option<(PathBuf, PathBuf, PathBuf)> {
    let exe = std::env::current_exe().ok()?;
    let dir = std::env::temp_dir().join(format!("sheetdiff-buur-{}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let job = dir.join("job.json");
    let text = serde_json::to_string(&Job {
        cfg: Config::default(),
        ink_dir: PathBuf::new(),
    })
    .ok()?;
    std::fs::write(&job, text).ok()?;
    Some((exe, dir, job))
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
    pub prefetch: Prefetch,
}

impl RenderService {
    pub fn start(ctx: eframe::egui::Context) -> Self {
        let (tx, rx_req) = std::sync::mpsc::channel();
        let (tx_resp, rx) = std::sync::mpsc::channel();
        let prefetch = Prefetch::start();
        let ready = prefetch.clone();
        std::thread::Builder::new()
            .name("sheetdiff-render".into())
            .spawn(move || {
                // een paniek in PDFium-code mag de viewer niet stil laten bevriezen: melden en
                // de thread opnieuw laten draaien met dezelfde kanalen
                let rx = std::sync::Arc::new(std::sync::Mutex::new(rx_req));
                loop {
                    let (rx2, tx2, ctx2) = (rx.clone(), tx_resp.clone(), ctx.clone());
                    let ready2 = ready.clone();
                    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                        let guard = rx2.lock().expect("render-ontvanger");
                        worker(&guard, tx2, ctx2, ready2)
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
        Self {
            tx,
            rx,
            gen: 0,
            prefetch,
        }
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

fn worker(rx: &Receiver<Req>, out: Sender<Resp>, ctx: eframe::egui::Context, ready: Prefetch) {
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
                                    send(Resp::ZoneError(z.id, sheetdiff_core::errors::nl(&e)));
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
                                    send(Resp::Error(g, sheetdiff_core::errors::nl(&e)));
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
                Err(e) => Resp::ZoneError(
                    z.id,
                    format!("uitsnede: {}", sheetdiff_core::errors::nl(&e)),
                ),
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
            let key = (src.clone(), p);
            let r = match ready.take(&key, Duration::from_secs(PREFETCH_WAIT_S)) {
                Some(l) => Resp::Base(g, l),
                None => match render_base(&docs, &src, &p) {
                    Ok(l) => Resp::Base(g, l),
                    Err(e) => Resp::Error(g, sheetdiff_core::errors::nl(&e)),
                },
            };
            if !send(r) {
                return;
            }
            continue; // eerst kijken of er al iets nieuws is
        }
        if let Some((r, dpi)) = want_detail.take() {
            let resp = match render_layer(&docs, &src, r, dpi, &p) {
                Ok(l) => Resp::Detail(g, l),
                Err(e) => Resp::Error(g, sheetdiff_core::errors::nl(&e)),
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
                Err(e) => Resp::Error(g, format!("vector: {}", sheetdiff_core::errors::nl(&e))),
            };
            if !send(resp) {
                return;
            }
        }
    }
}
