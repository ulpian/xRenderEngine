# CLI reference

The `xre` binary (the `xre-cli` crate) bundles four subcommands: a model viewer,
a benchmark, a project scaffold, and the font calibrator. Build and run it from
the workspace with `cargo run -p xre-cli -- <subcommand>`, or install it and call
`xre <subcommand>` directly.

```text
xre — xRenderEngine CLI
usage: xre <view|bench|new|glyphgen>
  view       render an .obj model (try: xre view model.obj --snapshot out.txt)
  bench      report the render-pipeline timings on this machine
  new        scaffold a new xRenderEngine project (xre new my-game)
  glyphgen   calibrate a font into a glyph ramp (try: xre glyphgen --help)
```

## `xre view` — model viewer & snapshot exporter

Loads an OBJ (resolving sibling `mtllib`s), fits it to the unit sphere, and either
renders one **headless snapshot** to a text file or opens an **interactive orbit
viewer**.

```sh
# Interactive viewer (orbit with the mouse / arrows; cycle shaders and lighting)
xre view assets/cube.obj

# Headless: render one frame to a text file (the CI / QA path)
xre view model.obj --snapshot out.txt
xre view model.obj --snapshot out.txt --size 120x60 --ascii
```

Flags:

| Flag | Meaning |
|------|---------|
| `--snapshot <path>` | render a single frame to a text file and exit (no TTY needed) |
| `--size <WxH>` | viewport size in cells for the snapshot (e.g. `120x60`) |
| `--ascii` | force ASCII borders and the mono theme (the degraded path) |

In interactive mode, the viewer is the project's manual QA harness. The controls:

| Key / input | Action |
|-------------|--------|
| mouse drag | orbit the camera |
| `←` `→` `↑` `↓` | rotate (yaw / pitch) |
| mouse scroll | zoom in / out |
| `+` or `=` | zoom in |
| `-` | zoom out |
| `m` | cycle lighting mode |
| `c` | cycle cell shader (luminance ramp → shape vector → the Unicode modes) |
| `q` / `Esc` | quit |

The `--snapshot` path is pure and terminal-free, which is exactly why it is
unit-tested — frames are text, so they diff and snapshot perfectly. Loader
warnings (malformed faces, missing materials) are printed to stderr without
aborting the load.

### Sample assets

The repo ships two models you can point the viewer at: `assets/cube.obj` (a
minimal test cube) and `assets/biohand.obj` (a detailed mesh for exercising the
loader and the parallel rasterizer on real geometry):

```sh
xre view assets/biohand.obj
```

> "Robotic Hand" (<https://skfb.ly/o8PzE>) by Jack Ansell is licensed under
> [Creative Commons Attribution](http://creativecommons.org/licenses/by/4.0/).
> See the `NOTICE` file for all asset and prior-art attributions.

## `xre bench` — pipeline timings

Reports the render-pipeline timings on your machine, mirroring the `criterion`
baselines so you can compare your hardware against the documented budget. The
header notes how the renderer was compiled — `[row-parallel (rayon)]` with the
default `parallel` feature, or `[single core]` without it.

```text
$ xre bench
xre bench — software render pipeline [row-parallel (rayon)]

scene                              tris    draw (ms)
----------------------------------------------------
cube 120x36                          12        0.113
sphere 120x36                      3072        0.378
torus 120x36                       2304        0.415
sphere 200x60                      6144        0.720

cell shader 120x36                        shade (ms)
----------------------------------------------------
LuminanceRamp                                  0.045
ShapeVector                                    0.081
HalfBlock                                      0.053
Braille                                        0.072
```

Run it under `--release` for representative numbers
(`cargo run -p xre-cli --release -- bench`); a debug build is several times
slower. To read the parallel speedup, build the binary both ways
(`--no-default-features` gives the single-core renderer) and compare.

## `xre new` — project scaffold

Creates a new binary project that depends on `xre` and draws a spinning cube — the
[Quickstart](quickstart.md) program, ready to run.

```sh
xre new my-game
cd my-game
cargo run
```

It writes `my-game/Cargo.toml` (with `xre` as a dependency) and
`my-game/src/main.rs`. It refuses to overwrite an existing directory.

## `xre glyphgen` — font calibration

Measures a font into a luminance ramp emitted as Rust source you check in and
`include!`. With a font:

```sh
xre glyphgen --font /path/to/Menlo.ttf --out assets/atlas_menlo.rs
```

Without one, it emits the font-free density ramp:

```sh
xre glyphgen --builtin --out assets/atlas_generic.rs
```

See [Glyph calibration](glyphgen.md) for the full flag list, the workflow, and how
to feed the output into a `LuminanceRamp` or `ShapeTable`.

## Exit codes

Every subcommand returns `0` on success and `1` on error, printing
`xre <cmd>: error: <message>` to stderr — so the CLI composes cleanly in scripts
and CI.
