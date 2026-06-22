//! Regression test mirroring the dashboard's tab-click flow end to end:
//! register the tab bar in a `MouseRouter`, route a left click, and resolve it to
//! a tab via `Tabs::hit`. Proves that clicking a tab selects it (the same effect
//! as pressing `Tab`).
#![allow(clippy::unwrap_used)]

use xre_term::{Modifiers, MouseButton, MouseEvent, MouseKind};
use xre_tui::{FocusId, MouseRouter, Rect, Tabs};

const TAB_ID: FocusId = FocusId(0);

const fn click(col: u32, row: u32) -> MouseEvent {
    MouseEvent {
        kind: MouseKind::Down(MouseButton::Left),
        col,
        row,
        mods: Modifiers::NONE,
    }
}

#[test]
fn clicking_tabs_selects_them() {
    let titles: Vec<String> = vec!["Overview".into(), "Metrics".into(), "Logs".into()];
    let tab_bar = Rect::new(0, 0, 120, 1);

    // render phase: register the full-width tab bar plus a non-overlapping body.
    let mut router = MouseRouter::new();
    router.begin_frame();
    router.register(TAB_ID, tab_bar);
    router.register(FocusId(1), Rect::new(0, 1, 120, 30));

    let mut tab = 0usize;
    let select = |router: &mut MouseRouter, ev: &MouseEvent, tab: &mut usize| {
        // input phase: exactly what App::handle_mouse does for the tab region.
        if router.route(ev) == Some(TAB_ID) {
            if let Some(idx) = Tabs::new(&titles, *tab).handle_mouse(ev, tab_bar) {
                *tab = idx;
            }
        }
    };

    // " Overview " = cols 0..10, " │ " = 10..13, " Metrics " = 13..22,
    // " │ " = 22..25, " Logs " = 25..31.
    select(&mut router, &click(16, 0), &mut tab);
    assert_eq!(tab, 1, "clicking 'Metrics' selects tab 1");
    select(&mut router, &click(27, 0), &mut tab);
    assert_eq!(tab, 2, "clicking 'Logs' selects tab 2");
    select(&mut router, &click(4, 0), &mut tab);
    assert_eq!(tab, 0, "clicking 'Overview' selects tab 0");

    // A click on a divider or empty space leaves the selection unchanged.
    select(&mut router, &click(11, 0), &mut tab);
    assert_eq!(tab, 0, "clicking the divider changes nothing");
}
