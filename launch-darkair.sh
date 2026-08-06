#!/usr/bin/env bash
# Launch DarkAir (first-person night pest-control sim). Builds the release
# binary on first run if it is missing. Assets resolve relative to the repo
# root, so run from here.
set -e
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$HERE"

# Rust stable lives behind /snap/bin on this machine (see CLAUDE.md).
export PATH=/snap/bin:$PATH

BIN="target/release/darkair"
if [ ! -x "$BIN" ]; then
    # No terminal when double-clicked, so surface build progress in a
    # dialog if one is available; otherwise just build quietly.
    if command -v zenity >/dev/null 2>&1; then
        ( cargo build --release -p darkair ) | \
            zenity --progress --pulsate --no-cancel --auto-close \
                   --title="Building DarkAir" --text="First-run build…" 2>/dev/null || true
    else
        cargo build --release -p darkair
    fi
fi

exec "$BIN" "$@"
