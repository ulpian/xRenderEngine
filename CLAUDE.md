# CLAUDE.md — xRenderEngine working guide

Guidance for Claude Code (and other agents) working in this repository.

## What this is

**xRenderEngine** (crate prefix `xre-`, binary `xre`) is a lightweight 3D /
sub-cell-ASCII rendering engine and game framework in Rust, targeting the
terminal. The **source of truth for design** is
[`RiftEngine-Plan/`](RiftEngine-Plan/) (the planning archive keeps its original
name) — read [`04-roadmap-overview.md`](RiftEngine-Plan/04-roadmap-overview.md)
and [`02-architecture.md`](RiftEngine-Plan/02-architecture.md) before non-trivial work.
[`ymael_analysis.md`](ymael_analysis.md) analyses the C++ predecessor whose algorithms are being ported.

**Current state:** Phases 0–5 plus the Phase 4.5 parallelism pass implemented and
tested (workspace builds clean under `clippy -D warnings`, all-green test suite
incl. property/golden/bench/zero-alloc coverage).
`xre-core` (types, color, OKLab, geometry), `xre-term` (probe, RAII guard, diffed
presenter, input), `xre-tui` (layout, panels, widgets, theme, `Viewport3D`),
`xre-render` (sample buffer, rasterizer, lighting, luminance/shape/Unicode shaders;
row-parallel rasterization + cell shading via rayon behind the default-on
`parallel` feature, byte-identical to serial), `xre-cello` (OBJ/MTL loader, scene
graph, controllers, textures), `xre-engine` (fixed-timestep loop, `hecs` ECS,
animation, input map, swept collision, grid raycaster), `glyphgen` (ramps + shape
vectors), and the `xre` CLI (`view`/`bench`/`new`/`glyphgen`). Demos: `dashboard`,
`spinning-cube`, `rift-fps`. The ten-chapter mdBook lives in `docs/book/`.
Remaining for 0.1: the Phase 4.5 SIMD hot-loop pass (rayon/tile-binning done) and
packaging.

## Workspace map

Cargo workspace; members under `crates/*` and `tools/*` (see `Cargo.toml`).

| Crate | Phase | Responsibility |
|-------|-------|----------------|
| `xre-core`   | 0 | math re-exports (glam), `Color`, `Cell`, `Rect`, errors |
| `xre-term`   | 1 | raw mode, capability probe, diffed `Presenter`, input |
| `xre-tui`    | 1–2 | layout, panels, widgets, `Viewport3D` |
| `xre-render` | 2 | `SampleBuffer`, rasterizer, `CellShader` trait + impls |
| `xre-cello`  | 3 | scene graph, OBJ/MTL loader, cameras, animation |
| `xre-engine` | 5 | game loop, `FramePacer`, hecs ECS, `InputMap`, assets |
| `xre`        | — | facade: prelude + feature flags (re-exports the above) |
| `tools/xre-cli` | 3.5+ | the `xre` binary (`new`/`view`/`bench`/`glyphgen`) |
| `tools/glyphgen` | 0.4 | offline font → luminance ramp / shape vectors |

## Commands

```sh
cargo build --workspace
cargo test  --workspace
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo deny check                 # needs: cargo install cargo-deny
cargo run -p xre-cli -- --help   # binary is named `xre`
```

## Hard rules (enforced by `[workspace.lints]` in `Cargo.toml`)

- **No `unsafe`** — `unsafe_code = "forbid"`.
- **No panics in library code** — use per-crate `thiserror` error enums. The
  presenter must restore the terminal via RAII guard *and* a panic hook.
- **`#![deny(missing_docs)]`** in every crate; document public items.
- **Determinism** — fixed `dt` + seed ⇒ bit-identical frames across platforms
  (enables `insta` golden-frame tests). No platform-specific float intrinsics in
  result paths.
- **Lints** — `clippy::pedantic` + `clippy::nursery` are on (warn);
  `clippy::unwrap_used` warns; the four `cast_*` lints are allowed only in
  rendering hot paths.
- **Conventional Commits** for messages (e.g. `feat(xre-core): add Rect::split`).

## Testing strategy

Per [`RiftEngine-Plan/12-testing-strategy.md`](RiftEngine-Plan/12-testing-strategy.md):
`cargo test` units, `insta` golden frames, `proptest` properties, `cargo fuzz`
parsers, `criterion` benchmarks (with CI thresholds + `dhat` zero-alloc checks),
and a PTY integration harness. Frames are text — snapshot them.

## Agent Skills

Skills are installed from [skills.sh](https://www.skills.sh/) via the `npx skills`
CLI. Sources live in `.agents/skills/` (portable across agents) and are symlinked
into `.claude/skills/` for Claude Code. Installed for this repo:

- **`rust-skills`** — `leonardomso/rust-skills` (179 Rust rules: ownership, error
  handling, testing, API design, performance, project structure, linting,
  anti-patterns, …). Invoke with `/rust-skills`.
- **`test-driven-development`** — `addyosmani/agent-skills` (Red-Green-Refactor,
  test pyramid, DAMP-over-DRY). Pairs with `12-testing-strategy.md`.
- **`documentation-and-adrs`** — `addyosmani/agent-skills` (Architecture Decision
  Records, API docs, inline-doc standards — supports the rustdoc + mdBook work).

To add more: `npx skills add <owner>/<repo> [--skill <name>]`, then restart the
session. Skills are plain-text instructions that run with full agent permissions —
review before relying on them.

## MCP

`.mcp.json` ships empty (`{ "mcpServers": {} }`). To enable a server, add an entry —
for example **context7** for up-to-date crate docs:

```json
{
  "mcpServers": {
    "context7": {
      "command": "npx",
      "args": ["-y", "@upstash/context7-mcp"]
    }
  }
}
```

(or a GitHub MCP server for issues/PRs to support the Phase 6 release workflow).

## Suggested permissions (optional)

To cut approval prompts for routine Rust commands, the user can run `/permissions`
and add allow rules such as:
`Bash(cargo build:*)`, `Bash(cargo test:*)`, `Bash(cargo clippy:*)`,
`Bash(cargo fmt:*)`, `Bash(cargo check:*)`, `Bash(cargo run:*)`,
`Bash(cargo deny:*)`. These are intentionally **not** pre-applied.
