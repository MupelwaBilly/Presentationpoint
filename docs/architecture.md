# Architecture and milestone boundary

## Confirmed stack

The project uses the fixed Rust stack from the product brief. The active M0–M3
dependencies are pinned by `Cargo.toml`: `winit` 0.30, `wgpu` 30, `egui` and
`egui-wgpu` 0.37, `bytemuck`, `zip`, `quick-xml`, `roxmltree`, `serde`,
`indexmap`, `rfd`, `thiserror`, and `tracing`.

Later milestones add the already selected stack components when their owning
feature arrives: `tiny-skia`; `parley`, `swash`, and `fontdb`; `usvg` and
`resvg`; `arboard`; `pdf-writer`; `slotmap`; `uuid`; `rayon`; `tokio`; and
`notify`. Keeping these libraries out until their features arrive avoids
making the M0–M3 executable depend on features that do not exist yet.

## Module boundaries

- `application` owns the native window and routes user actions. It keeps the
  selected path, ribbon/backstage state, task panes, and status text outside the
  document. M2 file actions still route document replacement through the
  command bus.
- `ooxml::package` owns ZIP members, package relationships, XML validation,
  deterministic writing, and atomic save verification. Other modules work
  with a `PptxDocument` and never coordinate ZIP or XML details.
- `storage` owns local settings and recent-file persistence. It prunes paths
  that no longer exist and writes JSON atomically under local app data.
- `commands` owns document replacement history and the undo/redo stack. Any
  document-state replacement passes through this command bus.
- `model` owns integer EMU measurements, angle units, theme references, and
  slide dimensions. UI state is not serialized into a deck.
- `ui::tokens` and `ui::icons` own Office-style chrome colors and the local
  Fluent icon asset pipeline. `canvas` owns viewport transforms, ruler and
  guide state, snapping, and a WGPU render pipeline for slide backgrounds and
  grid lines; egui paints the surrounding shell and ruler/guide overlays.
  The mouse wheel zooms, middle-button or Space-drag pans, and dragging out of
  a ruler adds an EMU-positioned guide with optional half-inch snapping.

The initial Fluent bundle contains Microsoft `@fluentui/svg-icons` 1.1.346
artwork for Open, Save, Undo, and Redo at 16, 20, and 24 px. The SVGs are
embedded in the binary and validated at startup. The bundled attribution is in
`assets/icons/NOTICE.md`.

## File-preservation contract in M1

Every regular package member is retained under its original part name with its
original uncompressed payload bytes. This includes unrecognized parts,
`mc:AlternateContent`, and extension lists. The writer changes ZIP container
compression and timestamps but does not rewrite the XML payloads. Before a
save replaces its destination, it reopens the temporary output and compares
all member names, types, bytes, and slide count against the in-memory package.

The M1 document model remains package-preserving and now exposes the slide
dimensions parsed from `p:sldSz`. The M3 canvas uses that size for its viewport,
but does not yet decode and draw slide shapes or text. Later milestones add
structured editing while keeping raw package parts available for
lossless round-tripping.

## Offline behavior

All runtime assets are local. The app includes no network client, cloud service,
account flow, telemetry, or update check. Cargo dependencies are ordinary
build-time Rust packages; the application itself does not fetch them or any
other resource at runtime.
