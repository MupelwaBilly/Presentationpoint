#!/usr/bin/env sh
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_root"

if [ -n "${PRESENTATIONPOINT_GNU_TOOLCHAIN_BIN:-}" ]; then
    PATH="$PRESENTATIONPOINT_GNU_TOOLCHAIN_BIN:$PATH"
    export PATH
fi

cargo fmt --check
cargo clippy --offline --all-targets -- -D warnings
cargo test --offline
cargo run --offline -- --generate-samples target/check-input
cargo run --offline -- --roundtrip-corpus target/check-input target/check-output

if [ -d testdata ] && find testdata -type f -iname '*.pptx' -print -quit | grep -q .; then
    cargo run --offline -- --roundtrip-corpus testdata target/corpus-roundtrip
fi
