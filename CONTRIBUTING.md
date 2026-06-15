# Contributing to xRenderEngine

Thanks for your interest! xRenderEngine is in **early Phase 0** (foundations).
The authoritative design lives in [`RiftEngine-Plan/`](RiftEngine-Plan/) — start with
[`04-roadmap-overview.md`](RiftEngine-Plan/04-roadmap-overview.md) and
[`02-architecture.md`](RiftEngine-Plan/02-architecture.md).

## Local development

```sh
cargo build --workspace            # build everything
cargo test  --workspace            # run tests
cargo fmt --all --check            # formatting (CI-enforced)
cargo clippy --workspace --all-targets -- -D warnings   # lints (CI-enforced)
cargo deny check                   # licenses & advisories (needs cargo-deny)
```

CI runs `fmt`, `clippy`, and `test` on Linux, macOS, and Windows; a green pipeline
from a clean clone is the Phase 0 exit criterion.

## Code conventions

These are hard rules, enforced by the workspace lint set in [`Cargo.toml`](Cargo.toml):

- **No `unsafe`** — `unsafe_code = "forbid"` across the workspace.
- **No panics in library code** — return errors via per-crate `thiserror` enums.
  The presenter restores the terminal via an RAII guard *and* a panic hook.
- **Document public items** — each crate sets `#![deny(missing_docs)]`.
- **Determinism** — given a fixed `dt` and seed, rendering must be bit-identical
  across platforms (this is what makes golden-frame tests possible). Avoid
  platform-specific float intrinsics in result paths.
- **`clippy::pedantic` + `clippy::nursery`** are on (warn). The four `cast_*` lints
  are allowed only in rendering hot paths.

## Commit messages

Use [Conventional Commits](https://www.conventionalcommits.org/) — e.g.
`feat(xre-core): add Rect::intersect`, `fix(xre-term): restore raw mode on panic`.

## Testing

See [`RiftEngine-Plan/12-testing-strategy.md`](RiftEngine-Plan/12-testing-strategy.md):
unit tests, `insta` golden frames, `proptest` properties, `cargo fuzz` parsers,
`criterion` benchmarks, and a PTY integration harness.
