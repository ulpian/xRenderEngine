//! Terminal spike (Stage 0.3): prove the riskiest platform assumptions before
//! the real presenter exists.
//!
//! It probes terminal capabilities, prints a report, then animates a truecolor
//! gradient block-field with *naive full redraws* at a 60 FPS target — measuring
//! the bytes pushed per frame, which is the number the diffed presenter in
//! Phase 1 has to beat. Press `q` or `Esc` to quit early.
//!
//! Run with: `cargo run -p xre-term --example terminal-spike`

use std::fmt::Write as _;
use std::io::{self, Write};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode};
use xre_term::{Capabilities, TerminalGuard};

const TARGET_FPS: u64 = 60;
const FRAME_BUDGET: Duration = Duration::from_micros(1_000_000 / TARGET_FPS);
const FRAMES: u32 = 240;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let caps = Capabilities::probe();
    println!("xRenderEngine — terminal spike");
    println!("  color depth ......... {:?}", caps.color);
    println!("  unicode level ....... {:?}", caps.unicode);
    println!(
        "  size ................ {}x{} cells",
        caps.size.x, caps.size.y
    );
    println!("  synchronized output . {}", caps.synchronized_output);
    println!(
        "\nAnimating {FRAMES} frames at {TARGET_FPS} FPS (naive full redraw). Press q/Esc to quit."
    );
    println!("Starting in 1s...");
    std::thread::sleep(Duration::from_secs(1));

    let cols = caps.size.x.max(1);
    let rows = caps.size.y.saturating_sub(1).max(1); // leave the last row clear

    let guard = TerminalGuard::enter()?;
    let mut out = io::stdout();

    let mut total_bytes = 0usize;
    let mut frames_drawn = 0u32;
    let run_start = Instant::now();

    for frame in 0..FRAMES {
        if quit_requested()? {
            break;
        }
        let frame_start = Instant::now();
        let buf = render_frame(cols, rows, frame);
        total_bytes += buf.len();
        out.write_all(buf.as_bytes())?;
        out.flush()?;
        frames_drawn += 1;

        if let Some(remaining) = FRAME_BUDGET.checked_sub(frame_start.elapsed()) {
            std::thread::sleep(remaining);
        }
    }

    let elapsed = run_start.elapsed();
    drop(guard);

    let avg_bytes = total_bytes / (frames_drawn.max(1) as usize);
    let fps = f64::from(frames_drawn) / elapsed.as_secs_f64();
    println!("spike done: {frames_drawn} frames in {elapsed:.2?}");
    println!("  measured ............ {fps:.1} FPS");
    println!("  bytes / frame ....... {avg_bytes} (full redraw, {cols}x{rows})");
    println!("  total bytes ......... {total_bytes}");
    Ok(())
}

/// Poll (non-blocking) for a quit key.
fn quit_requested() -> Result<bool, Box<dyn std::error::Error>> {
    if event::poll(Duration::from_millis(0))? {
        if let Event::Key(key) = event::read()? {
            return Ok(matches!(key.code, KeyCode::Char('q') | KeyCode::Esc));
        }
    }
    Ok(false)
}

/// Build one full frame of the moving gradient as a single ANSI byte string.
fn render_frame(cols: u32, rows: u32, frame: u32) -> String {
    let mut buf = String::with_capacity((cols * rows) as usize * 20 + rows as usize * 8);
    let phase = frame as f32 * 0.06;
    for row in 0..rows {
        // Cursor to the start of this row (1-based).
        let _ = write!(buf, "\x1b[{};1H", row + 1);
        for col in 0..cols {
            let (red, green, blue) = gradient(col, row, cols, rows, phase);
            let _ = write!(buf, "\x1b[48;2;{red};{green};{blue}m ");
        }
    }
    buf.push_str("\x1b[0m");
    buf
}

/// A smoothly scrolling RGB gradient.
fn gradient(col: u32, row: u32, cols: u32, rows: u32, phase: f32) -> (u8, u8, u8) {
    let u = col as f32 / cols as f32;
    let v = row as f32 / rows as f32;
    let red = 0.5f32.mul_add(u.mul_add(6.0, phase).sin(), 0.5);
    let green = 0.5f32.mul_add(v.mul_add(6.0, phase * 1.3).sin(), 0.5);
    let blue = 0.5f32.mul_add((u + v).mul_add(4.0, phase * 0.7).cos(), 0.5);
    (
        (red * 255.0) as u8,
        (green * 255.0) as u8,
        (blue * 255.0) as u8,
    )
}
