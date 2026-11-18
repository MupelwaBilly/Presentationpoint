# Presentationpoint

Presentationpoint is a native Windows presentation editor built in Rust. Its
files are ordinary OOXML `.pptx` packages, and its runtime has no account,
telemetry, update check, or network feature.

The current delivery covers M0 through M3: typed foundations, local settings
and recent files, a native ribbon/backstage shell, and a PPTX package
reader/writer that preserves every part's uncompressed bytes. The slide area is
rendered through a WGPU callback and supports zoom, pan, rulers, gridlines,
guide placement, and grid snapping. Deck shape and text rendering/editing are
later milestones; this canvas currently displays the selected slide as a blank
page while preserving all original OOXML parts for save.

## Build

Use a Rust toolchain with edition 2024 support. After dependencies are present
in Cargo's local cache, the project can be built and checked without network
access:

```powershell
cargo check --offline
cargo test --offline
```

If the Rust toolchain targets Windows GNU, make its MinGW `bin` directory
available on `PATH`, or set `PRESENTATIONPOINT_GNU_TOOLCHAIN_BIN` to that
directory before running the check script.

Run `tools/check.ps1` on Windows or `tools/check.sh` on a POSIX shell to check
formatting, lint the code, run round-trip checks, and verify generated sample
decks. When a real-deck corpus is added, put `.pptx` files under `testdata/`;
the check scripts treat those files as read-only inputs.

## Local storage

On Windows, settings and recent files live in
`%LOCALAPPDATA%/Presentationpoint/`, with `autorecover/` and `snapshots/`
directories ready for later milestones. No registry or roaming profile data is
used.

See [the architecture notes](docs/architecture.md) for the fixed stack and the
current milestone boundary.
