# Lab setup after Dropbox transfer

This is the full DarkAir working tree (git history included, build
artifacts excluded). To continue on the lab machine:

1. Rust stable 1.97+ required. If the system rustc is old, install via
   rustup; on the WSL dev box it lives behind `PATH=/snap/bin:$PATH`.
2. Build: `cargo build --release -p darkair` (first build recompiles
   everything — `target/` was deliberately not transferred, ~12 GB).
3. Tests: `cargo test --workspace` (350 green as of transfer).
4. `ffmpeg` needed for `--demo-film` and the clip tooling.
5. GPU notes: under WSL2 pin Vulkan (the Intel GL/D3D12 adapter always
   fails request_device — see CLAUDE.md). Native Linux/real GPU: it just
   works, and is dramatically faster than llvmpipe.
6. Remote is https://github.com/jclements3/deadair.git (main). The tree
   was pushed through commit 24c7f0a; run `git status` to see any
   work-in-progress files newer than that.
7. Worth running on the 40-cpu box: `darkair --calibrate` for its
   N-rabbit machine card (laptop rated 131), and `darkair --demo-film`
   re-renders the promo reel fast.

Untracked-but-included extras: `videos/` (reference recordings + the
rendered `darkair_demo.mp4`), which git ignores by design.
