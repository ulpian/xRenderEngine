//! `dashboard` — the Phase 1.5 milestone demo.
//!
//! A live TUI dashboard: header tabs, a grid of animated gauges and sparklines,
//! a scrolling log fed by fake data, and a command input. Exercises the layout
//! solver, panels, the widget set, the theme, the diffed presenter and the input
//! pump together.
//!
//! Run with `cargo run -p xre-tui --example dashboard` (add `--ascii` for the
//! degraded ASCII/mono rendering used on Linux VTs and minimal terminals). Press
//! `Tab` to switch tabs, type into the command line, and `q` / `Esc` to quit.

use std::time::Duration;

use xre_core::{Attrs, CellBuffer, Color, Style};
use xre_term::{Capabilities, Event, EventQueue, KeyCode, Presenter, TerminalGuard};
use xre_tui::{
    BorderSet, Constraint, Frame, Gauge, GridLayout, Input, Layout, Log, Panel, Sparkline, Tabs,
    Text, Theme, Widget,
};

struct App {
    ascii: bool,
    theme: Theme,
    tab: usize,
    tabs: Vec<String>,
    frame: u64,
    series: Vec<Vec<f32>>,
    log: Log,
    input: Input,
    running: bool,
}

impl App {
    fn new(ascii: bool) -> Self {
        let theme = if ascii {
            Theme::mono()
        } else {
            Theme::default()
        };
        let mut log = Log::new(256);
        log.push("dashboard started — press Tab to switch, q to quit");
        Self {
            ascii,
            theme,
            tab: 0,
            tabs: vec!["Overview".into(), "Metrics".into(), "Logs".into()],
            frame: 0,
            series: vec![Vec::new(); 3],
            log,
            input: Input::new().focused(true),
            running: true,
        }
    }

    const fn border(&self) -> BorderSet {
        if self.ascii {
            BorderSet::ASCII
        } else {
            BorderSet::ROUNDED
        }
    }

    /// Advance the fake data one tick.
    fn tick(&mut self) {
        self.frame += 1;
        let t = self.frame as f32 * 0.12;
        for (i, s) in self.series.iter_mut().enumerate() {
            let phase = i as f32 * 1.7;
            let v = (t + phase).sin().mul_add(0.5, 0.5) * 100.0;
            s.push(v);
            if s.len() > 120 {
                s.remove(0);
            }
        }
        if self.frame.is_multiple_of(24) {
            self.log.push(format!(
                "tick {} — cpu {:.0}%",
                self.frame,
                self.gauge_ratio(0) * 100.0
            ));
        }
    }

    fn gauge_ratio(&self, i: usize) -> f32 {
        self.series
            .get(i)
            .and_then(|s| s.last())
            .copied()
            .unwrap_or(0.0)
            / 100.0
    }

    fn handle(&mut self, ev: &Event) {
        let Event::Key(k) = ev else { return };
        match k.code {
            KeyCode::Char('q') | KeyCode::Esc => self.running = false,
            KeyCode::Tab => self.tab = (self.tab + 1) % self.tabs.len(),
            KeyCode::Enter => {
                if let Some(line) = self.input.handle_key(*k) {
                    if !line.is_empty() {
                        self.log.push(format!("> {line}"));
                    }
                }
            }
            _ => {
                self.input.handle_key(*k);
            }
        }
    }

    fn render(&self, buf: &mut CellBuffer) {
        let bg = if self.ascii {
            Style::DEFAULT
        } else {
            Style::DEFAULT.with_bg(Color::Rgb(16, 18, 24))
        };
        buf.fill(bg.cell(' '));
        let mut frame = Frame::root(buf);
        let area = frame.area();

        let rows = Layout::vertical([
            Constraint::Len(1),  // tab bar
            Constraint::Fill(1), // body
            Constraint::Len(3),  // command panel
        ])
        .split(area);

        // Header tabs.
        Tabs::new(&self.tabs, self.tab)
            .active_style(self.theme.style("tabs.active"))
            .inactive_style(self.theme.style("tabs.inactive"))
            .render(rows[0], &mut frame);

        self.render_body(rows[1], &mut frame);

        // Command input panel.
        let panel = Panel::new()
            .border(Some(self.border()))
            .border_style(self.theme.style("panel.border"))
            .title("command")
            .title_style(self.theme.style("panel.title"));
        let inner = panel.render(rows[2], &mut frame);
        self.input.render(inner, &mut frame);
    }

    fn render_body(&self, area: xre_core::Rect, frame: &mut Frame) {
        let grid = GridLayout::new(
            [Constraint::Fill(1), Constraint::Fill(1)],
            [Constraint::Fill(1), Constraint::Fill(1)],
        )
        .gaps(1, 0)
        .split(area);

        let labels = ["cpu", "mem", "net"];
        for (i, label) in labels.iter().enumerate() {
            let cell = grid[i / 2][i % 2];
            let panel = Panel::new()
                .border(Some(self.border()))
                .border_style(self.theme.style("panel.border"))
                .title(*label)
                .title_style(self.theme.style("panel.title"));
            let inner = panel.render(cell, frame);
            let split = Layout::vertical([Constraint::Len(1), Constraint::Fill(1)]).split(inner);
            Gauge::new(self.gauge_ratio(i))
                .ascii(self.ascii)
                .bar_style(self.theme.style("gauge.bar"))
                .bg_style(self.theme.style("gauge.bg"))
                .label(format!("{:.0}%", self.gauge_ratio(i) * 100.0))
                .render(split[0], frame);
            Sparkline::new(self.series[i].clone())
                .max(100.0)
                .style(self.theme.style("sparkline"))
                .render(split[1], frame);
        }

        // Fourth cell: the log.
        let cell = grid[1][1];
        let panel = Panel::new()
            .border(Some(self.border()))
            .border_style(self.theme.style("panel.border"))
            .title("log")
            .title_style(self.theme.style("panel.title"));
        let inner = panel.render(cell, frame);
        if self.tab == 2 {
            self.log.render(inner, frame);
        } else {
            Text::styled(
                "switch to the Logs tab to follow output",
                Style::DEFAULT.with_attrs(Attrs::DIM),
            )
            .wrap(true)
            .render_into(inner, frame);
            self.log.render(
                xre_core::Rect::new(
                    inner.left(),
                    inner.top() + 1,
                    inner.width(),
                    inner.height().saturating_sub(1),
                ),
                frame,
            );
        }
    }
}

fn main() -> std::io::Result<()> {
    let ascii = std::env::args().any(|a| a == "--ascii");
    let _guard = TerminalGuard::enter().map_err(std::io::Error::other)?;

    let mut caps = Capabilities::probe();
    if ascii {
        caps.color = xre_core::ColorDepth::Ansi16;
    }
    let mut presenter = Presenter::stdout(&caps);
    let mut buf = CellBuffer::new(caps.size);
    let mut events = EventQueue::new();
    let mut app = App::new(ascii);

    while app.running {
        // Drain input for this frame (frame-coherent).
        events
            .pump(Duration::from_millis(16))
            .map_err(std::io::Error::other)?;
        let drained: Vec<Event> = events.drain().collect();
        for ev in &drained {
            if let Event::Resize(size) = ev {
                buf.resize(*size);
                presenter.resize(*size);
            }
            app.handle(ev);
        }

        app.tick();
        if buf.size() != presenter.size() {
            buf.resize(presenter.size());
        }
        app.render(&mut buf);
        presenter.present(&buf).map_err(std::io::Error::other)?;
    }
    Ok(())
}
