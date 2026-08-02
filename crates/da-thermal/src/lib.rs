//! da-thermal — the thermal simulation system for DeadAir (SDD §2, SRS §3.3).
//!
//! The world is simulated once, in physical terms; the thermal optic is a
//! lossy view of this single truth. This crate owns:
//!
//! - [`ThermalProfile`] — per-object static thermal description (FR-T1),
//!   with presets for pests, zombies, and common scenery.
//! - [`ambient_at`] — the scripted dusk→dawn ambient temperature curve.
//! - [`contrast`] / [`detection_range_factor`] — the night contrast curve
//!   (FR-T2), the game's difficulty dial: high at dusk, minimum at the
//!   pre-dawn crossover (`CROSSOVER_T ≈ 0.85`), partial dawn recovery.
//! - [`ThermalSim`] — the 1 Hz per-object temperature integrator, including
//!   rain wetting collapse (FR-T3) and clear-sky radiative cooling (metal
//!   roofs read *below* ambient).
//! - [`HeatEvent`] — residual heat decals (FR-T4): bedding, footfalls,
//!   fired barrels, pellet impacts.
//!
//! The zombie invisibility rule (SDD §4.1) is emergent: zombies use
//! [`ThermalProfile::zombie`], which couples them exactly to ambient — the
//! renderer never special-cases them.
//!
//! No function in this crate panics on any input.

#![warn(missing_docs)]

pub mod curve;
pub mod heat;
pub mod profile;
pub mod sim;

pub use curve::{ambient_at, contrast, detection_range_factor, solar_decay};
pub use heat::HeatEvent;
pub use profile::ThermalProfile;
pub use sim::{ThermalSim, ThermalState};
