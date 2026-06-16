# xre-rs

<img src="https://raw.githubusercontent.com/ulpian/xrenderengine/main/xrenderengine_cello.gif" alt="xRenderEngine — cello model demo" width="100%">

**A lightweight 3D rendering engine and game framework that runs in your terminal.**

`xre-rs` is the facade crate for **xRenderEngine** — it re-exports the engine's
sub-crates, ships a `prelude`, and coordinates feature flags. (The library target
is named `xre`, so you depend on `xre-rs` but write `use xre::…`.)

xRenderEngine renders real 3D scenes — and full TUI dashboards — as characters,
using **sub-cell sampling** and **shape-vector glyph selection** to get far past the
usual "one character per pixel" ASCII look. It is dependency-minimal, has no async
runtime and no GPU, and degrades gracefully from truecolor down to plain ASCII.

```toml
[dependencies]
xre-rs = "0.0.1"
```

```rust
use xre::prelude::*;
```

## Why it's different

- **Characters are not pixels.** Each terminal cell is rasterized at sub-cell
  resolution (2×4 samples by default) and resolved by a pluggable *cell shader*
  (luminance ramp, 6-D shape vector, half-block, Braille, block-shades).
- **One frame, two engines.** A TUI layer (panels, grids, widgets) and a 3D layer
  (scenes in `Viewport3D` widgets) share one cell buffer and one diffed presenter.
- **Degrade gracefully.** Truecolor → 256 → 16 → ASCII, detected at runtime.
- **Lightweight.** Few dependencies; no GPU, no async runtime.

## Crates

| Crate | Responsibility |
|-------|----------------|
| `xre-core`   | Math re-exports (glam), `Color`, `Cell`, geometry, errors |
| `xre-term`   | Terminal backend: raw mode, capabilities, diffed presenter, input |
| `xre-tui`    | Panels, grids, layout, widgets, focus, the `Viewport3D` widget |
| `xre-render` | Software 3D: sample buffers, rasterizer, raycasters, cell shaders |
| `xre-cello`  | Scene graph, mesh, camera, lights, materials, animation, OBJ/MTL |
| `xre-engine` | Game loop, time, input mapping, ECS (hecs), assets |
| `xre-rs`     | This facade crate: prelude, feature flags, re-exports |

The `xre` command-line tool (model viewer, scaffolding, benches) is published
separately as [`xre-cli`](https://crates.io/crates/xre-cli) (`cargo install xre-cli`).

## License

Licensed under the Apache License, Version 2.0. See
[`LICENSE`](https://github.com/ulpian/xrenderengine/blob/main/LICENSE).
