//! sheetdiff-core: pure logica voor het vergelijken van tekeninguitgiftes. Geen UI.

pub mod choices;
pub mod comments;
pub mod compare;
pub mod config;
pub mod errors;
pub mod export;
pub mod geom;
pub mod identify;
pub mod issue;
pub mod layer;
pub mod moved;
pub mod overlay;
pub mod pairing;
pub mod parallel;
pub mod pdf;
pub mod raster;
pub mod register;
pub mod report;
pub mod textdiff;
pub mod tiles;
pub mod vector;

pub use config::Config;
