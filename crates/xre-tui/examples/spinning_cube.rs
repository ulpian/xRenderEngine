//! `spinning-cube` — the Phase 2 / Gate G2 milestone demo.
//!
//! A lit rotating cube and torus rendered into a `Viewport3D` panel beside live
//! `Text` widgets, all sharing one diffed presenter. Demonstrates the whole
//! two-stage pipeline end to end: mesh → sample buffer → cell shader → cells.
//!
//! Run with `cargo run -p xre-tui --example spinning-cube`. Press `q` / `Esc` to
//! quit, `Space` to pause, `w` to toggle the second mesh, `c` to cycle shaders.

use std::time::{Duration, Instant};

use xre_core::math::{UVec2, Vec3};
use xre_core::{CellBuffer, Color, Style, Transform};
use xre_render::{
    builtin_cell_shaders, draw_mesh, Camera, Cull, LightRig, Material, Mesh, Projection,
    SampleBuffer, ShadeMode,
};
use xre_term::{Capabilities, Event, EventQueue, KeyCode, Presenter, TerminalGuard};
use xre_tui::{BorderSet, Constraint, Frame, Layout, Panel, Text, Viewport3D, Widget};

#[allow(clippy::too_many_lines)]
fn main() -> std::io::Result<()> {
    let _guard = TerminalGuard::enter().map_err(std::io::Error::other)?;
    let caps = Capabilities::probe();
    let mut presenter = Presenter::stdout(&caps);
    let mut buf = CellBuffer::new(caps.size);
    let mut events = EventQueue::new();

    let cube = Mesh::cube();
    let torus = Mesh::torus(1.4, 0.4, 32, 16);
    let shaders = builtin_cell_shaders();
    let mut shader_idx = 0usize;
    let rig = LightRig::default().with_light(xre_render::Light::point(Vec3::new(3.0, 3.0, 3.0)));
    let mut samples = SampleBuffer::new(UVec2::new(1, 1), 2, 4);

    let start = Instant::now();
    let mut paused = false;
    let mut show_torus = true;
    let mut angle = 0.0f32;
    let mut last = Instant::now();
    let mut running = true;

    while running {
        events
            .pump(Duration::from_millis(16))
            .map_err(std::io::Error::other)?;
        for ev in events.drain() {
            match ev {
                Event::Resize(size) => {
                    buf.resize(size);
                    presenter.resize(size);
                }
                Event::Key(k) => match k.code {
                    KeyCode::Char('q') | KeyCode::Esc => running = false,
                    KeyCode::Char(' ') => paused = !paused,
                    KeyCode::Char('w') => show_torus = !show_torus,
                    KeyCode::Char('c') => shader_idx = (shader_idx + 1) % shaders.len(),
                    _ => {}
                },
                _ => {}
            }
        }

        let dt = last.elapsed().as_secs_f32();
        last = Instant::now();
        if !paused {
            angle += dt;
        }

        if buf.size() != presenter.size() {
            buf.resize(presenter.size());
        }
        buf.fill(Style::DEFAULT.with_bg(Color::Rgb(8, 10, 16)).cell(' '));

        let area = buf.area();
        let cols = Layout::horizontal([Constraint::Fill(1), Constraint::Len(28)]).split(area);

        // 3D viewport panel.
        let panel = Panel::new()
            .border(Some(BorderSet::ROUNDED))
            .border_style(Style::fg(Color::Rgb(90, 110, 140)))
            .title("Viewport3D")
            .title_style(Style::fg(Color::Rgb(120, 200, 255)));
        let inner = panel.inner(cols[0]);

        // Fill the sample buffer for this frame.
        samples.resize(
            UVec2::new(inner.width().max(1), inner.height().max(1)),
            2,
            4,
        );
        samples.clear([8, 10, 16]);
        let cam = Camera::look_at(Vec3::new(0.0, 1.6, 4.5), Vec3::ZERO);
        let vp = cam.view_projection(
            inner.width().max(1),
            inner.height().max(1),
            Projection::DEFAULT_CELL_ASPECT,
        );
        let mut model = Transform::IDENTITY;
        model.rotation = xre_core::math::Quat::from_rotation_y(angle)
            * xre_core::math::Quat::from_rotation_x(angle * 0.6);
        draw_mesh(
            &mut samples,
            &cube,
            model.to_mat4(),
            vp,
            &rig,
            &Material::colored(Vec3::new(0.7, 0.8, 0.9)),
            ShadeMode::PerSample,
            Cull::Back,
        );
        if show_torus {
            let mut tmodel = Transform::from_translation(Vec3::new(0.0, 0.0, 0.0));
            tmodel.rotation = xre_core::math::Quat::from_rotation_x(angle * -0.4);
            tmodel.scale = Vec3::splat(1.0);
            draw_mesh(
                &mut samples,
                &torus,
                tmodel.to_mat4(),
                vp,
                &rig,
                &Material::colored(Vec3::new(0.9, 0.7, 0.5)),
                ShadeMode::PerSample,
                Cull::Back,
            );
        }

        {
            let mut frame = Frame::root(&mut buf);
            panel.render(cols[0], &mut frame);
            Viewport3D::new(&samples, shaders[shader_idx].1.as_ref()).render(inner, &mut frame);

            // Info panel.
            let info = Panel::new()
                .border(Some(BorderSet::ROUNDED))
                .border_style(Style::fg(Color::Rgb(90, 110, 140)))
                .title("info");
            let info_inner = info.render(cols[1], &mut frame);
            let fps = 1.0 / dt.max(1e-3);
            let lines = [
                format!("fps    {fps:5.0}"),
                format!("angle  {angle:5.1}"),
                format!(
                    "tris   {}",
                    cube.triangle_count()
                        + if show_torus {
                            torus.triangle_count()
                        } else {
                            0
                        }
                ),
                format!("uptime {:4.0}s", start.elapsed().as_secs_f32()),
                format!("shader {}", shaders[shader_idx].0),
                String::new(),
                "space  pause".into(),
                "w      torus".into(),
                "c      shader".into(),
                "q      quit".into(),
            ];
            for (i, line) in lines.iter().enumerate() {
                Text::styled(line.clone(), Style::fg(Color::Rgb(180, 190, 200))).render_into(
                    xre_core::Rect::new(
                        info_inner.left(),
                        info_inner.top() + i as u32,
                        info_inner.width(),
                        1,
                    ),
                    &mut frame,
                );
            }
        }

        presenter.present(&buf).map_err(std::io::Error::other)?;
    }
    Ok(())
}
