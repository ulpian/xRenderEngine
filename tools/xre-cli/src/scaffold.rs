//! `xre new <name>` — scaffold a minimal xRenderEngine project (Stage 6.x).

use std::path::Path;

/// Create a new binary project that depends on `xre` and draws a spinning cube.
pub fn run(args: &[String]) -> Result<(), String> {
    let name = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .ok_or("usage: xre new <project-name>")?;
    let dir = Path::new(name);
    if dir.exists() {
        return Err(format!("{name} already exists"));
    }
    std::fs::create_dir_all(dir.join("src")).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("Cargo.toml"), cargo_toml(name)).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("src/main.rs"), MAIN_RS).map_err(|e| e.to_string())?;
    println!("created {name}/ — run it with:\n  cd {name} && cargo run");
    Ok(())
}

fn cargo_toml(name: &str) -> String {
    format!(
        "[package]\nname = {name:?}\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
         [dependencies]\nxre = \"0.1\"\n"
    )
}

const MAIN_RS: &str = r"//! A minimal xRenderEngine app: a spinning cube in the terminal.

use std::time::Instant;
use xre::prelude::*;

fn main() -> std::io::Result<()> {
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
        events.pump(std::time::Duration::from_millis(16)).ok();
        for ev in events.drain() {
            if let Event::Key(k) = ev {
                if matches!(k.code, KeyCode::Char('q') | KeyCode::Esc) {
                    running = false;
                }
            }
        }
        if buf.size() != presenter.size() {
            buf.resize(presenter.size());
        }
        buf.fill(Cell::new(' '));
        let area = buf.area();
        samples.resize(UVec2::new(area.width().max(1), area.height().max(1)), 2, 4);
        samples.clear([0, 0, 0]);

        let cam = Camera::look_at(Vec3::new(0.0, 0.0, 4.0), Vec3::ZERO);
        let vp = cam.view_projection(area.width().max(1), area.height().max(1), Projection::DEFAULT_CELL_ASPECT);
        let angle = start.elapsed().as_secs_f32();
        let mut model = Transform::IDENTITY;
        model.rotation = Quat::from_rotation_y(angle) * Quat::from_rotation_x(angle * 0.5);
        draw_mesh(&mut samples, &cube, model.to_mat4(), vp, &rig,
                  &Material::default(), ShadeMode::PerSample, Cull::Back);

        let mut frame = Frame::root(&mut buf);
        Viewport3D::new(&samples, &shader).render(area, &mut frame);
        presenter.present(&buf).ok();
    }
    Ok(())
}
";
