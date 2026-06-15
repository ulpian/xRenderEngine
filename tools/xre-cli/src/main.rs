//! `xre` — the xRenderEngine command-line tool.
//!
//! Subcommands: `view <obj|image>` (the OBJ / image viewer & snapshot exporter),
//! `bench` (the render-pipeline timings), `new <name>` (project scaffold) and
//! `glyphgen` (offline font calibration).

mod bench;
mod scaffold;
mod view;

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let command = args.next();
    let rest: Vec<String> = args.collect();
    let result: Result<(), String> = match command.as_deref() {
        Some("glyphgen") => glyphgen::run_cli(&rest),
        Some("view") => view::run(&rest),
        Some("bench") => bench::run(&rest),
        Some("new") => scaffold::run(&rest),
        _ => return usage(),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("xre {}: error: {msg}", command.as_deref().unwrap_or("?"));
            ExitCode::FAILURE
        }
    }
}

fn usage() -> ExitCode {
    println!("xre — xRenderEngine CLI");
    println!("usage: xre <view|bench|new|glyphgen>");
    println!(
        "  view       render an .obj model or image (try: xre view model.obj, xre view photo.png)"
    );
    println!("  bench      report the render-pipeline timings on this machine");
    println!("  new        scaffold a new xRenderEngine project (xre new my-game)");
    println!("  glyphgen   calibrate a font into a glyph ramp (try: xre glyphgen --help)");
    ExitCode::SUCCESS
}
