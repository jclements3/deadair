//! da-core — shared foundation for DeadAir: ids, night clock, weather,
//! deterministic RNG, and common units.
//!
//! Everything downstream (scene graph, thermal sim, AI, economy) builds on
//! these types. This crate has no I/O and no platform dependencies.

pub mod clock;
pub mod id;
pub mod rng;
pub mod units;
pub mod weather;

pub use clock::NightClock;
pub use id::{EntityId, IdGen, NodeId};
pub use rng::Rng;
pub use units::TempF;
pub use weather::{Forecast, WeatherMods};
