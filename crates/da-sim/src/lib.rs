//! da-sim — headless gameplay simulation for DarkAir.
//!
//! Weapons (SDD §5), AI (SDD §6), hazards/health (SRS §3.5), and the
//! per-tick [`SimEvent`] stream that the economy, render, audio, and
//! thermal layers consume without coupling to sim internals.
//!
//! Entities live in plain world coordinates ([`glam::Vec3`], meters).
//! Everything is deterministic: the same seed and command script always
//! replay to the identical event log ([`da_core::Rng`] streams are forked
//! per entity and per subsystem).
//!
//! No rendering, no I/O — this crate is pure gameplay logic.

#![warn(missing_docs)]

pub mod ai;
pub mod ballistics;
pub mod events;
pub mod hazard;
pub mod hit;
pub mod noise;
pub mod sim;
pub mod weapon;

pub use ai::{AiCtx, Animal, Light};
pub use ballistics::{aim_solution, drop_at, lethal_range_m, muzzle_velocity_mps, AimSolution};
pub use events::{DamageCause, HeatKind, SimEvent};
pub use hazard::{Hazard, HazardKind, Health, Optic};
pub use hit::{check_backstop, resolve_shot, ShotOutcome, Species, Sphere, Target};
pub use noise::{discharge_noise_radius_m, NoiseEvent, NoiseKind, MODERATOR_FACTOR};
pub use sim::{Command, Player, Sim};
pub use weapon::{
    Caliber, PelletVariant, PowerPlant, PowerSetting, RifleConfig, RifleTier,
};
