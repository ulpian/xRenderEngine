# Changelog

All notable changes to xRenderEngine are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/) and the project aims for
[Semantic Versioning](https://semver.org/) from 0.1.0 onward.

## [Unreleased]

## [0.0.1] - 2026-06-16

First public release. The full engine across Phases 0–5 of the roadmap (plus the
Phase 4.5 parallelism pass) is implemented and tested; the workspace builds clean
on stable with `clippy -D warnings` and a green test suite (unit, property,
golden-frame and benchmark coverage).

### Added

- **Phase 0 — Foundations.** `xre-core` (math re-exports, `Color` downgrade
  chain, `Cell`/`CellBuffer`, `Rect`, `Style`, OKLab), the `xre-term` capability
  probe and RAII `TerminalGuard`, and the `glyphgen` calibration tool.
- **Phase 1 — TUI core.** Diffed `Presenter` (minimal-byte SGR state machine,
  wide-glyph aware, synchronized output), normalized input events with a
  frame-coherent `EventQueue`, a remainder-exact `Layout`/`GridLayout` solver,
  `Panel`s, the widget set (`Text`, `List`, `Table`, `Gauge`, `Sparkline`,
  `Tabs`, `Input`, `Log`, `Separator`), `Theme`, focus, and the `dashboard`
  example.
- **Phase 2 — Renderer core.** Sub-cell `SampleBuffer`, cell-aspect-aware
  `Camera`/`Projection`, robust near-plane clipping, a perspective-correct
  edge-function rasterizer with depth test and top-left fill rule, Lambert
  lighting (flat/Gouraud/per-sample, cel, fog), procedural meshes, the
  `LuminanceRamp` cell shader, the `Viewport3D` widget and the `spinning-cube`
  example. Cross-OS golden frames.
- **Phase 3 — Assets & scenes.** A from-scratch, fuzz-hardened OBJ/MTL loader
  with a vendored ear-clipping triangulator, an arena scene graph with draw-list
  extraction and frustum culling, camera controllers, PGM/PPM/BMP textures, and
  the `xre view` viewer (`--snapshot` headless export + interactive orbit).
- **Phase 4 — Advanced shading.** The 6-D `ShapeVector` shader with contrast
  enhancement (plus offline `glyphgen` shape measurement), the `HalfBlock`,
  `BlockShades` and `Braille` Unicode modes (live-cyclable in `xre view`), OKLab
  palette mapping, and criterion benchmarks.
- **Phase 5 — Game engine.** Fixed-timestep loop with `FramePacer` and a
  deterministic accumulator, a thin `hecs` ECS facade with components and a
  system `Schedule`, keyframe animation (`Track`/`Clip`/`Animator`/`Tween`), an
  action-based `InputMap` with contexts, tunnel-proof swept-AABB collision, the
  grid raycaster backend, the `Game`/`run` entry point and the `rift-fps` demo.
- **Phase 4.5 — Parallelism pass.** Row-parallel rasterization and cell shading
  via rayon behind the default-on `parallel` feature: a reusable `Rasterizer`
  owns the per-frame scratch, triangles are tile-binned into disjoint row bands,
  and bands fill concurrently — **byte-identical** to the serial path (a
  bit-equality unit test and proptest gate it). The cell-shade stage gains
  `resolve_cells`, used by `Viewport3D`. A `dhat` test enforces zero per-frame
  allocation, the criterion suite adds a combined frame benchmark, and
  `xre bench` reports the parallel timings.
- **The xRenderEngine Book.** A ten-chapter mdBook (`docs/book/`): introduction,
  quickstart, TUI guide, 3D rendering, cell shaders, scenes & assets, the game
  loop, glyph calibration, terminal compatibility, and a CLI reference.

### Packaging

- **Published to crates.io.** The seven library crates ship as `xre-core`,
  `xre-term`, `xre-render`, `xre-cello`, `xre-tui`, `xre-engine`, and the facade
  `xre-rs` (its library target is named `xre`, as the `xre` name was taken).
  Downstream code depends on `xre-rs = "0.0.1"` and keeps writing `use xre::…`.
  Internal path dependencies now pin their version, so `cargo deny`'s wildcard
  ban passes.
- **Prebuilt binaries.** The `Release` workflow attaches `xre` binaries for the
  six tier-1 targets (Linux gnu/musl x86_64, musl aarch64, macOS x86_64/aarch64,
  Windows MSVC x86_64) to this GitHub release.
- The `xre-cli` and `glyphgen` tools are **not** on crates.io yet; install the
  CLI from source (`cargo install --path tools/xre-cli`) or use the attached
  binaries.

### Notes

- Determinism: result paths use plain `a*b + c` rather than FMA, and the
  row-parallel renderer preserves per-pixel arithmetic and depth-test order, so
  golden frames stay bit-identical across platforms and core counts.
- Deferred to post-0.1: the SIMD hot-loop pass and dirty-viewport short-circuit,
  ordered dithering, the quantized shape-vector LUT, glTF/PNG, audio, networking,
  and publishing the `xre-cli` / `glyphgen` tools to crates.io.

[Unreleased]: https://github.com/ulpian/xRenderEngine/compare/v0.0.1...HEAD
[0.0.1]: https://github.com/ulpian/xRenderEngine/releases/tag/v0.0.1
