# Changelog

All notable changes to xRenderEngine are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/) and the project aims for
[Semantic Versioning](https://semver.org/) from 0.1.0 onward.

## [Unreleased]

## [0.1.0] - 2026-06-22

The first release under the project's Semantic Versioning commitment. A broad
interaction pass lands across the stack — full mouse support, exact held-key
tracking, and a textured, lit, pickable grid raycaster — all additive and gated
by the existing determinism, golden-frame, property and zero-alloc test
strategy. The workspace still builds clean on stable with `clippy -D warnings`
and an all-green suite.

### Added

- **Mouse interaction, end to end.** `xre-term`'s `TerminalGuard` now enables
  SGR mouse capture, the keyboard-enhancement (kitty) protocol, and an OSC 22
  mouse-pointer shape — all configurable via the new `GuardOptions` and all
  restored on drop *and* on panic. `xre-tui` gains an immediate-mode
  `MouseRouter` (per-frame hit-testing with drag capture, mirroring the focus
  manager), `handle_mouse` on `List`, `Log`, `Input` and `Tabs` (click-to-select,
  click-to-position-cursor, wheel scroll), and `viewport_gesture`/`ViewportGesture`
  to turn drags and the wheel into camera orbit/zoom. The `dashboard`,
  `spinning-cube` and `rift-fps` demos and `xre view` are wired for mouse.
- **Exact held-key input.** Key events now carry a `KeyState`
  (`Press`/`Repeat`/`Release`) instead of dropping releases. When the kitty
  protocol is active, `InputMap` tracks genuinely simultaneous held keys (e.g.
  W+D) from real releases; when it is not, it synthesises holds with a
  deterministic grace window and exposes `pressed_repeat` for repeat-friendly
  actions. The new `LatchAxis` gives press-only terminals a sticky `-1/0/+1`
  direction for continuous diagonal movement. Wire `report_releases` from
  `TerminalGuard::keyboard_enhanced`.
- **`Scrollbar` widget.** A stateless `Scrollbar` (with `ScrollbarOrientation`)
  rendered against an application-owned `ScrollbarState`, using integer-only
  thumb geometry so frames stay bit-identical. `Log` gains `scroll_to` and
  `scrollbar_state` helpers; golden frames cover it.
- **Textured, lit, pickable raycaster.** `Raycaster::render_textured` draws
  Wolfenstein-style textured walls (per-column U from the wall-hit fraction, V
  from the screen row) and adds a faked 2-D `PointLight2D` proximity glow blended
  over the distance fog. `raycast`/`ray_dir` expose single-ray picking via the
  same DDA the rendered columns use, returning a `RayHit` (distance, point,
  wall id, face, U, normal) — so a pick is consistent with what is drawn.
- **Public row-band fills.** `xre-render` promotes `RowBand` to public API and
  adds `SampleBuffer::par_row_bands`, a deterministic serial/parallel row-band
  fill (byte-identical across thread counts) that the raycaster builds on. The
  raycaster runs row-parallel behind a new **default-on `parallel` feature** in
  `xre-engine`, with the serial path kept bit-identical (gated by determinism,
  golden and `dhat` zero-alloc tests, plus a criterion bench).
- **Collision push-out.** `collide` can push a body out of any static it
  overlaps along the axis of least penetration, resolving inside corners over a
  couple of passes.
- **Demos & docs.** `rift-fps` becomes a textured, lit, mouse-look FPS
  (brick-wall texture asset, point-light landmark, latch movement); `xre view`
  gains arrow-key pan (images) / orbit (meshes) plus drag-to-orbit and
  scroll-to-zoom. The mdBook chapters on terminals, the game loop, the TUI and
  the CLI are updated for mouse and held-key input.

### Changed

- **`xre-engine` enables `parallel` by default.** The serial path is
  byte-identical, so this changes throughput only, never output; disable with
  `--no-default-features` for a rayon-free build.
- **`TerminalGuard::enter` now captures the mouse and requests keyboard
  enhancement by default.** Use `TerminalGuard::enter_with(GuardOptions { … })`
  to opt out (e.g. to preserve native click-drag text selection — users can
  still select with **Shift**/**Option** while capture is on).

### Fixed

- The row-band parallel decision now gates on the (cheap, pure) sample count
  *before* querying `rayon::current_num_threads()`, so a sub-threshold viewport
  never spins up the global thread pool for a frame it will render serially —
  restoring the zero-alloc-per-frame invariant for small buffers.

### Notes

- **Breaking changes vs 0.0.1** (permitted by the SemVer-from-0.1.0 line): the
  `Key` struct gains a `state` field and `Capabilities` gains a `mouse` field
  (both `#[non_exhaustive]`-adjacent additions to public structs), key releases
  are now delivered rather than dropped, and `RowBand` is promoted from
  `pub(crate)` to `pub`.
- Determinism is preserved: result paths still use plain `a*b + c`, and the
  row-parallel raycaster keeps per-sample arithmetic and write order, so golden
  frames stay bit-identical across platforms and core counts.
- Still deferred to a later release: the SIMD hot-loop pass and dirty-viewport
  short-circuit, ordered dithering, the quantized shape-vector LUT, glTF/PNG,
  audio, networking, and publishing the `xre-cli` / `glyphgen` tools to
  crates.io.

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

[Unreleased]: https://github.com/ulpian/xRenderEngine/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/ulpian/xRenderEngine/compare/v0.0.1...v0.1.0
[0.0.1]: https://github.com/ulpian/xRenderEngine/releases/tag/v0.0.1
