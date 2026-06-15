# Quickstart

This chapter gets a lit, spinning cube rotating in your terminal, then explains
every line. If you would rather start from a generated project, jump to
[Scaffolding](#scaffolding-a-project) at the end — `xre new my-game` writes exactly
this program for you.

## Add the dependency

```toml
[dependencies]
xre = "0.1"
```

That single facade crate re-exports everything and carries the feature flags. The
row-parallel renderer is on by default; nothing else is required to get started.

## A spinning cube

```rust,ignore
use std::time::{Duration, Instant};
use xre::prelude::*;

fn main() -> std::io::Result<()> {
    // RAII: raw mode + alternate screen, restored on drop *and* on panic.
    let _guard = TerminalGuard::enter().map_err(std::io::Error::other)?;
    let caps = Capabilities::probe();
    let mut presenter = Presenter::stdout(&caps);
    let mut buf = CellBuffer::new(caps.size);
    let mut events = EventQueue::new();
    let mut samples = SampleBuffer::new(UVec2::new(1, 1), 2, 4);

    let cube = Mesh::cube();
    let shader = LuminanceRamp::default();
    let rig = LightRig::default();
    let start = Instant::now();
    let mut running = true;

    while running {
        // Pump input, blocking at most one frame budget.
        events.pump(Duration::from_millis(16)).ok();
        for ev in events.drain() {
            if let Event::Key(k) = ev {
                if matches!(k.code, KeyCode::Char('q') | KeyCode::Esc) {
                    running = false;
                }
            }
        }

        // Keep the buffers sized to the terminal.
        if buf.size() != presenter.size() {
            buf.resize(presenter.size());
        }
        buf.fill(Cell::new(' '));
        let area = buf.area();
        samples.resize(UVec2::new(area.width().max(1), area.height().max(1)), 2, 4);
        samples.clear([0, 0, 0]);

        // Camera and a per-frame rotation.
        let cam = Camera::look_at(Vec3::new(0.0, 0.0, 4.0), Vec3::ZERO);
        let vp = cam.view_projection(
            area.width().max(1),
            area.height().max(1),
            Projection::DEFAULT_CELL_ASPECT,
        );
        let angle = start.elapsed().as_secs_f32();
        let mut model = Transform::IDENTITY;
        model.rotation =
            Quat::from_rotation_y(angle) * Quat::from_rotation_x(angle * 0.5);

        // Stage 1: rasterize into the sub-cell sample buffer.
        draw_mesh(
            &mut samples,
            &cube,
            model.to_mat4(),
            vp,
            &rig,
            &Material::default(),
            ShadeMode::PerSample,
            Cull::Back,
        );

        // Stage 2: resolve samples → glyphs and present the diff.
        let mut frame = Frame::root(&mut buf);
        Viewport3D::new(&samples, &shader).render(area, &mut frame);
        presenter.present(&buf).ok();
    }
    Ok(())
}
```

Run it with `cargo run`. Press `q` or `Esc` to quit. You should see a shaded cube
turning, lit by the default directional light.

## What each part does

- **`TerminalGuard::enter()`** switches the terminal into raw mode and the
  alternate screen, hides the cursor, and installs a panic hook. Whether the loop
  ends normally or a panic unwinds, your shell is restored. Bind it to a real name
  (`_guard`), not `_`, or it drops immediately. See [Terminals](terminals.md).
- **`Capabilities::probe()`** detects color depth, Unicode level, synchronized
  output, and terminal size, all with safe fallbacks.
- **`Presenter::stdout(&caps)`** is the diffed writer: each `present` compares the
  new `CellBuffer` against the last and emits only the bytes that changed.
- **`CellBuffer`** is the 2D grid of `Cell`s — the shared canvas for both TUI and
  3D. **`SampleBuffer::new(size, 2, 4)`** is the sub-cell render target: `2×4`
  samples per cell.
- **`Mesh::cube()`**, **`LightRig::default()`**, **`LuminanceRamp::default()`** are
  the geometry, lighting, and cell shader. The default `LuminanceRamp` needs no
  font atlas (see [Cell shaders](cell-shaders.md)).
- **`Camera::look_at` + `view_projection(cols, rows, cell_aspect)`** build the
  view-projection matrix. `Projection::DEFAULT_CELL_ASPECT` (0.5) accounts for the
  fact that terminal cells are about twice as tall as they are wide, so spheres
  look round rather than squashed.
- **`draw_mesh(...)`** is stage one: it transforms vertices, near-clips, and
  rasterizes depth-tested, perspective-correct, Lambert-lit samples into
  `samples`. `ShadeMode::PerSample` lights every sample; `Cull::Back` drops
  back-faces. Full details in [3D rendering](rendering.md).
- **`Viewport3D::new(&samples, &shader).render(area, &mut frame)`** is stage two:
  it resolves each sample block to a glyph through the cell shader and composites
  into the frame, leaving empty cells transparent.

## Resizing, input, and the loop shape

The loop is the canonical shape for any xRenderEngine app: pump input within the
frame budget, drain events, keep the buffers sized to the terminal, fill, draw,
present. `events.drain()` hands you frame-coherent events; resizing is just
`buf.resize(presenter.size())` (the presenter tracks the live size). For a
*game*, you don't write this loop yourself — implement the `Game` trait and call
`run`, which adds a deterministic fixed-timestep update and a precise frame pacer.
See [The game loop](game-loop.md).

## Scaffolding a project

The CLI writes a runnable version of this program for you:

```sh
xre new my-game
cd my-game
cargo run
```

You get a `Cargo.toml` depending on `xre` and a `src/main.rs` with the spinning
cube above. From there, swap in your own mesh, add a `Panel` around the viewport
([TUI guide](tui.md)), or load a model with the [OBJ loader](scenes.md).

## Where to go next

- [TUI guide](tui.md) — panels, the widget gallery, layout, themes.
- [3D rendering](rendering.md) — cameras, the rasterizer, lighting modes.
- [Cell shaders](cell-shaders.md) — from the luminance ramp to shape vectors.
- [The game loop](game-loop.md) — fixed timestep, ECS, input, collision.
