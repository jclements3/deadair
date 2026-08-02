//! da-render — the optics renderer.
//!
//! Three pipelines over one scene truth (SDD §4): naked eye, night vision,
//! thermal. This crate currently holds the GPU-independent color science —
//! thermal palettes with auto-gain windowing, and deterministic NV sensor
//! grain — verified against real scope footage
//! (`assets/reference/optics-look.md`). The wgpu passes build on these.

pub mod grain;
pub mod palette;

pub use grain::nv_grain;
pub use palette::{Agc, ThermalPalette};
