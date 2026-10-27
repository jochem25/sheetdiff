//! Tegelrender-service voor de viewer. Een eigen thread bezit de PDF-documenten (PDFium-
//! objecten zijn niet Send) en rendert tegels op aanvraag, op de zoomfactor zelf: geen
//! uitvergrote bitmap, dus scherp tot op pixelniveau. Recente tegels blijven in een LRU-cache
//! met een geheugengrens.

use crate::geom::Affine;
use crate::overlay::{self, OverlayColors, ViewMode};
use crate::pdf::Doc;
use crate::raster::DiffParams;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

pub const TILE: usize = 512;

/// Welke bladen de service toont: pagina A, pagina B en de uitlijning B -> A.
#[derive(Debug, Clone, PartialEq)]
pub struct Sheet {
    pub a: Option<(PathBuf, usize)>,
    pub b: Option<(PathBuf, usize)>,
    pub b_to_a: Affine,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileKey {
    /// Zoomniveau als pixels per inch (afgerond), zodat tegels herbruikbaar zijn.
    pub dpi: u32,
    pub tx: i32,
    pub ty: i32,
    pub mode: u8,
    /// Hash van tolerantie/kleuren/dekking, zodat een instellingswijziging nieuwe tegels geeft.
    pub style: u64,
    pub sheet_gen: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    pub mode: ViewMode,
    pub params: DiffParams,
    pub colors: OverlayColors,
    pub dim_same: f32,
    pub mix: f32,
}

impl Style {
    pub fn hash(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (self.params.tolerance, self.params.radius_px).hash(&mut h);
        (self.colors.removed, self.colors.added, self.colors.same).hash(&mut h);
        ((self.dim_same * 100.0) as i32, (self.mix * 100.0) as i32).hash(&mut h);
        h.finish()
    }
    fn mode_id(&self) -> u8 {
        match self.mode {
            ViewMode::Diff => 0,
            ViewMode::A => 1,
            ViewMode::B => 2,
            ViewMode::Blend => 3,
        }
    }
}

pub struct Tile {
    pub key: TileKey,
    pub rgba: Vec<u8>,
    pub removed: u64,
    pub added: u64,
    pub ms: u128,
}

enum Req {
    SetSheet(Sheet, u64),
    Render(TileKey, Style),
    /// Wachtrij leegmaken (bijv. na zoomen: oude aanvragen zijn niet meer nodig).
    Cancel,
}

/// Handvat voor de UI-kant.
pub struct TileService {
    tx: Sender<Req>,
    pub rx: Receiver<Tile>,
    gen: u64,
}

impl TileService {
    pub fn start() -> Self {
        let (tx, rx_req) = std::sync::mpsc::channel::<Req>();
        let (tx_tile, rx) = std::sync::mpsc::channel::<Tile>();
        std::thread::Builder::new()
            .name("sheetdiff-tegels".into())
            .spawn(move || worker(rx_req, tx_tile))
            .expect("tegelthread");
        Self { tx, rx, gen: 0 }
    }
    pub fn set_sheet(&mut self, s: Sheet) -> u64 {
        self.gen += 1;
        let _ = self.tx.send(Req::SetSheet(s, self.gen));
        self.gen
    }
    pub fn request(&self, dpi: u32, tx: i32, ty: i32, style: &Style) -> TileKey {
        let key = TileKey {
            dpi,
            tx,
            ty,
            mode: style.mode_id(),
            style: style.hash(),
            sheet_gen: self.gen,
        };
        let _ = self.tx.send(Req::Render(key, *style));
        key
    }
    pub fn cancel(&self) {
        let _ = self.tx.send(Req::Cancel);
    }
}

fn worker(rx: Receiver<Req>, out: Sender<Tile>) {
    let mut docs: HashMap<PathBuf, Doc> = HashMap::new();
    let mut sheet: Option<(Sheet, u64)> = None;
    let mut queue: VecDeque<(TileKey, Style)> = VecDeque::new();
    loop {
        // alles wat klaarstaat ophalen; nieuwste aanvragen eerst renderen (wat zichtbaar is)
        let first = if queue.is_empty() {
            rx.recv().ok()
        } else {
            rx.try_recv().ok()
        };
        let mut msgs: Vec<Req> = first.into_iter().collect();
        while let Ok(m) = rx.try_recv() {
            msgs.push(m);
        }
        if msgs.is_empty() && queue.is_empty() {
            return; // UI is weg
        }
        for m in msgs {
            match m {
                Req::SetSheet(s, g) => {
                    queue.clear();
                    for (f, _) in s.a.iter().chain(s.b.iter()) {
                        if !docs.contains_key(f) {
                            if let Ok(d) = Doc::open(f) {
                                docs.insert(f.clone(), d);
                            }
                        }
                    }
                    sheet = Some((s, g));
                }
                Req::Render(k, st) => {
                    if !queue.iter().any(|(q, _)| *q == k) {
                        queue.push_back((k, st));
                    }
                }
                Req::Cancel => queue.clear(),
            }
        }
        let Some((k, st)) = queue.pop_back() else {
            continue;
        };
        let Some((s, g)) = &sheet else { continue };
        if k.sheet_gen != *g {
            continue;
        }
        let t0 = std::time::Instant::now();
        let page = |p: &Option<(PathBuf, usize)>| {
            p.as_ref()
                .and_then(|(f, i)| docs.get(f).and_then(|d| d.page(*i).ok()))
        };
        let (pa, pb) = (page(&s.a), page(&s.b));
        let scale = k.dpi as f64 / 72.0;
        let to_px = Affine::scale(scale).then(&Affine::translate(
            -(k.tx as f64 * TILE as f64),
            -(k.ty as f64 * TILE as f64),
        ));
        let r = overlay::render_tile(
            pa.as_ref(),
            pb.as_ref(),
            &s.b_to_a,
            &to_px,
            TILE,
            TILE,
            &st.params,
            st.mode,
            &st.colors,
            st.dim_same,
            st.mix,
        );
        if let Ok((rgba, removed, added)) = r {
            if out
                .send(Tile {
                    key: k,
                    rgba,
                    removed,
                    added,
                    ms: t0.elapsed().as_millis(),
                })
                .is_err()
            {
                return;
            }
        }
    }
}

/// Eenvoudige LRU-cache voor de UI-kant (begrensd in bytes).
pub struct TileCache<V> {
    map: HashMap<TileKey, V>,
    order: VecDeque<TileKey>,
    budget: usize,
    per_item: usize,
}

impl<V> TileCache<V> {
    pub fn new(budget_bytes: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
            budget: budget_bytes,
            per_item: TILE * TILE * 4,
        }
    }
    pub fn get(&mut self, k: &TileKey) -> Option<&V> {
        if self.map.contains_key(k) {
            if let Some(p) = self.order.iter().position(|x| x == k) {
                self.order.remove(p);
            }
            self.order.push_back(*k);
        }
        self.map.get(k)
    }
    pub fn insert(&mut self, k: TileKey, v: V) {
        self.map.insert(k, v);
        self.order.push_back(k);
        while self.order.len() * self.per_item > self.budget {
            if let Some(old) = self.order.pop_front() {
                self.map.remove(&old);
            }
        }
    }
    pub fn len(&self) -> usize {
        self.map.len()
    }
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}
