//! `glyphgen` binary — calibrate a font into an xRenderEngine glyph ramp.
//!
//! Examples:
//! - `glyphgen --font Menlo.ttf --out assets/atlas_menlo.rs`
//! - `glyphgen --builtin --out assets/atlas_generic.rs`
//!
//! All logic lives in the library ([`glyphgen::run_cli`]) so the `xre` CLI can
//! reuse it via `xre glyphgen ...`.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match glyphgen::run_cli(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("glyphgen: error: {msg}");
            ExitCode::FAILURE
        }
    }
}
