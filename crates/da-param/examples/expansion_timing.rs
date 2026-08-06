//! Zone-expansion timing sanity check (SDD perf note): expand each shipped
//! zone a few times and report the best wall time. Run with
//! `cargo run --release -p da-param --example expansion_timing`.

use std::path::PathBuf;
use std::time::Instant;

fn main() {
    let zones_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/zones");
    for file in [
        "home_farm.zone.ron",
        "main_street.zone.ron",
        "grain_coop.zone.ron",
        "town_edge.zone.ron",
    ] {
        let src = da_param::load_zone_file(zones_dir.join(file)).expect("zone loads");
        // Warm-up + best-of-5: expansion is a pure function, so the best
        // sample is the honest cost with cache noise removed.
        let mut best = f64::INFINITY;
        let mut nodes = 0;
        for _ in 0..5 {
            let t0 = Instant::now();
            let zone = da_param::expand_zone(&src).expect("zone expands");
            let dt = t0.elapsed().as_secs_f64() * 1000.0;
            nodes = zone.scene.len();
            if dt < best {
                best = dt;
            }
        }
        println!("{file:<28} {best:8.2} ms   ({nodes} nodes)");
    }
}
