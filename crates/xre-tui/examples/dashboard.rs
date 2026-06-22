//! `dashboard` — the Phase 1.5 milestone demo.
//!
//! A live TUI dashboard: header tabs, a grid of animated gauges and sparklines,
//! a scrolling log fed by fake data, and a command input. Exercises the layout
//! solver, panels, the widget set, the theme, the diffed presenter and the input
//! pump together.
//!
//! Run with `cargo run -p xre-tui --example dashboard` (add `--ascii` for the
//! degraded ASCII/mono rendering used on Linux VTs and minimal terminals). Press
//! `Tab` (or click a tab) to switch views — Overview (a 2×2 meter grid + log),
//! Metrics (the meters full-width), and Logs (the full log). Type into the command
//! line, and `q` / `Esc` to quit.
//!
//! Mouse: click a tab to switch views, scroll the wheel over the log to scroll it,
//! drag the log's scrollbar, and click the command line to position the cursor.
//! (Hold Shift / Option to select text while mouse capture is on.)

use std::time::Duration;

use xre_core::{CellBuffer, Color, Rect, Style};
use xre_term::{
    Capabilities, Event, EventQueue, KeyCode, KeyState, MouseEvent, Presenter, TerminalGuard,
};
use xre_tui::{
    BorderSet, Constraint, FocusId, Frame, Gauge, GridLayout, Input, Layout, Log, MouseRouter,
    Panel, Scrollbar, ScrollbarOrientation, Sparkline, Tabs, Theme, Widget,
};

/// Hit-test ids for the mouse-interactive regions (see [`App::handle_mouse`]).
const TAB_ID: FocusId = FocusId(0);
const LOG_ID: FocusId = FocusId(1);
const LOG_BAR_ID: FocusId = FocusId(2);
const INPUT_ID: FocusId = FocusId(3);

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
    // Mouse routing: rebuilt each render, consumed by the next frame's input.
    router: MouseRouter,
    tab_rect: Rect,
    log_view: Rect,
    log_track: Rect,
    input_rect: Rect,
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
            router: MouseRouter::new(),
            tab_rect: Rect::default(),
            log_view: Rect::default(),
            log_track: Rect::default(),
            input_rect: Rect::default(),
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
        match ev {
            // Act on press/repeat only; releases (kitty protocol) drive nothing
            // here and must not double-trigger.
            Event::Key(k) if k.state != KeyState::Release => match k.code {
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
            },
            Event::Mouse(m) => self.handle_mouse(m),
            _ => {}
        }
    }

    /// Route a mouse event to the region under the cursor (regions were
    /// registered during the previous frame's render).
    fn handle_mouse(&mut self, m: &MouseEvent) {
        let Some(id) = self.router.route(m) else {
            return;
        };
        match id {
            TAB_ID => {
                let hit = Tabs::new(&self.tabs, self.tab).handle_mouse(m, self.tab_rect);
                if let Some(idx) = hit {
                    self.tab = idx;
                }
            }
            LOG_ID => {
                self.log.handle_mouse(m, self.log_view);
            }
            LOG_BAR_ID => {
                let vp = self.log_view.height() as usize;
                let mut state = self.log.scrollbar_state(self.log_view.height() as u16);
                if Scrollbar::new(ScrollbarOrientation::VerticalRight).handle_mouse(
                    m,
                    self.log_track,
                    &mut state,
                ) {
                    self.log.scroll_to(state.get_position(), vp);
                }
            }
            INPUT_ID => {
                self.input.handle_mouse(m, self.input_rect);
            }
            _ => {}
        }
    }

    fn render(&mut self, buf: &mut CellBuffer) {
        // Rebuild the mouse hit-regions for this frame; routed next frame.
        self.router.begin_frame();
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
        self.tab_rect = rows[0];
        self.router.register(TAB_ID, rows[0]);

        self.render_body(rows[1], &mut frame);

        // Command input panel.
        let panel = Panel::new()
            .border(Some(self.border()))
            .border_style(self.theme.style("panel.border"))
            .title("command")
            .title_style(self.theme.style("panel.title"));
        let inner = panel.render(rows[2], &mut frame);
        self.input.render(inner, &mut frame);
        self.input_rect = inner;
        self.router.register(INPUT_ID, inner);
    }

    /// Each tab shows a distinct view, so clicking (or pressing Tab) visibly
    /// switches content.
    fn render_body(&mut self, area: Rect, frame: &mut Frame) {
        match self.tab {
            0 => self.render_overview(area, frame),
            1 => self.render_metrics(area, frame),
            _ => self.render_logs(area, frame),
        }
    }

    /// A panelled gauge + sparkline for series `i` drawn into `cell`.
    fn render_meter(&self, cell: Rect, i: usize, label: &str, frame: &mut Frame) {
        let panel = Panel::new()
            .border(Some(self.border()))
            .border_style(self.theme.style("panel.border"))
            .title(label)
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

    /// Overview tab: a 2×2 grid of the three meters plus a scrollable log.
    fn render_overview(&mut self, area: Rect, frame: &mut Frame) {
        let grid = GridLayout::new(
            [Constraint::Fill(1), Constraint::Fill(1)],
            [Constraint::Fill(1), Constraint::Fill(1)],
        )
        .gaps(1, 0)
        .split(area);

        let labels = ["cpu", "mem", "net"];
        for (i, label) in labels.iter().enumerate() {
            self.render_meter(grid[i / 2][i % 2], i, label, frame);
        }
        self.render_log_panel(grid[1][1], frame);
    }

    /// Metrics tab: the three meters stacked full-width for a closer look.
    fn render_metrics(&self, area: Rect, frame: &mut Frame) {
        let rows = Layout::vertical([
            Constraint::Fill(1),
            Constraint::Fill(1),
            Constraint::Fill(1),
        ])
        .split(area);
        let labels = ["cpu", "mem", "net"];
        for (i, label) in labels.iter().enumerate() {
            self.render_meter(rows[i], i, label, frame);
        }
    }

    /// Logs tab: the log filling the whole body with its scrollbar.
    fn render_logs(&mut self, area: Rect, frame: &mut Frame) {
        self.render_log_panel(area, frame);
    }

    /// Draw the bordered log panel into `area` with a draggable scrollbar on its
    /// right edge, and register its hit-regions for the mouse router.
    fn render_log_panel(&mut self, area: Rect, frame: &mut Frame) {
        let panel = Panel::new()
            .border(Some(self.border()))
            .border_style(self.theme.style("panel.border"))
            .title("log")
            .title_style(self.theme.style("panel.title"));
        let inner = panel.render(area, frame);
        let cols = Layout::horizontal([Constraint::Fill(1), Constraint::Len(1)]).split(inner);
        let (content, track) = (cols[0], cols[1]);
        self.log.render(content, frame);

        let mut sb_state = self.log.scrollbar_state(content.height() as u16);
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .ascii(self.ascii)
            .track_style(self.theme.style("scrollbar.track"))
            .thumb_style(self.theme.style("scrollbar.thumb"))
            .render_stateful(track, frame, &mut sb_state);

        self.log_view = content;
        self.log_track = track;
        self.router.register(LOG_ID, content);
        self.router.register(LOG_BAR_ID, track);
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
