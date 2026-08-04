//! DeadAir game systems, exposed as a library so integration tests can run
//! full campaigns headless (the binary in `main.rs` is the windowed game).

pub mod aim;
pub mod camp;
pub mod camp3d;
pub mod convert;
pub mod fauna;
pub mod flora;
pub mod hunt;
pub mod range;
pub mod tutorial;
