//! DeadAir game systems, exposed as a library so integration tests can run
//! full campaigns headless (the binary in `main.rs` is the windowed game).

pub mod camp;
pub mod convert;
pub mod hunt;
pub mod tutorial;
