//! Regression: latch-mode movement on terminals without the kitty protocol,
//! where auto-repeat arrives as `Press` after a long initial delay. The example
//! drives the latch from `InputMap::pressed_repeat` (press *or* repeat) so a held
//! or re-pressed direction always (re-)asserts; SET semantics make it hold-safe.
#![allow(clippy::float_cmp)]

use xre_engine::{Binding, InputMap, LatchAxis};
use xre_term::{Event, Key, KeyCode, KeyState, Modifiers};

const DT: f32 = 1.0 / 60.0;

const fn press(c: char) -> Event {
    Event::Key(Key {
        code: KeyCode::Char(c),
        mods: Modifiers::NONE,
        state: KeyState::Press,
    })
}

fn game_map() -> InputMap {
    let mut input = InputMap::new();
    input.bind("game", "fwd", Binding::key(KeyCode::Char('w')));
    input.bind("game", "back", Binding::key(KeyCode::Char('s')));
    input.push_context("game");
    input.set_release_reporting(false); // legacy: no key releases
    input
}

/// Drive the move latch exactly as `rift-fps` does, from press-or-repeat signals.
fn drive(input: &InputMap, mv: &mut LatchAxis) {
    if input.pressed_repeat("fwd") {
        mv.set_positive();
    }
    if input.pressed_repeat("back") {
        mv.set_negative();
    }
}

/// Times (seconds) a legacy terminal delivers auto-repeat `Press`es for a held
/// key: an initial press, then nothing for `initial_delay`, then one every
/// `repeat`.
fn autorepeat_times(hold_secs: f32, initial_delay: f32, repeat: f32) -> Vec<f32> {
    let mut t = vec![0.0];
    let count = ((hold_secs - initial_delay) / repeat).floor().max(0.0) as usize;
    for i in 0..=count {
        let next = (i as f32).mul_add(repeat, initial_delay);
        if next <= hold_secs {
            t.push(next);
        }
    }
    t
}

#[test]
fn holding_forward_keeps_moving() {
    // Across macOS-ish (0.5s) and slow (2.0s) initial repeat delays, holding W
    // must keep the latch set the whole time (sticky through the silent gap).
    for delay in [0.5_f32, 0.66, 1.0, 2.0] {
        let times = autorepeat_times(2.5, delay, 1.0 / 30.0);
        let mut input = game_map();
        let mut mv = LatchAxis::default();
        let mut ti = 0;
        let mut values = Vec::new();
        for f in 0..180 {
            let now = f as f32 * DT;
            input.begin_frame(DT);
            while ti < times.len() && times[ti] <= now + 1e-6 {
                input.feed(&press('w'));
                ti += 1;
            }
            drive(&input, &mut mv);
            values.push(mv.value());
        }
        assert!(
            values[1..].iter().all(|&v| v > 0.5),
            "latch dropped while held (initial delay {delay}s): {values:?}"
        );
    }
}

#[test]
fn repress_forward_after_reverse_relatches() {
    // Press W, then S (reverse), then W again on the next frame: the re-press must
    // re-latch forward even though W is still within its grace window.
    let mut input = game_map();
    let mut mv = LatchAxis::default();

    let frame = |input: &mut InputMap, mv: &mut LatchAxis, c: char| {
        input.begin_frame(DT);
        input.feed(&press(c));
        drive(input, mv);
    };

    frame(&mut input, &mut mv, 'w');
    assert_eq!(mv.value(), 1.0, "W sets forward");
    frame(&mut input, &mut mv, 's');
    assert_eq!(mv.value(), -1.0, "S reverses");
    frame(&mut input, &mut mv, 'w');
    assert_eq!(
        mv.value(),
        1.0,
        "re-pressing W must re-latch forward, not stay -1"
    );
}

#[test]
fn hold_w_tap_s_recovers_within_a_few_frames() {
    // Hold W continuously (auto-repeat ~30Hz) and tap S once mid-hold. The player
    // may flick backward for a frame or two but must recover to forward quickly,
    // not stay stuck backward (the bug the adversarial pass found).
    let repeat = 1.0 / 30.0;
    let w_times = autorepeat_times(2.0, 0.5, repeat);
    let s_tap = 1.0_f32; // tap S at t=1.0, well into W's repeat phase
    let mut input = game_map();
    let mut mv = LatchAxis::default();
    let mut ti = 0;
    let mut s_done = false;
    let mut backward_frames = 0;
    let mut last_value = 1.0;

    for f in 0..150 {
        let now = f as f32 * DT;
        input.begin_frame(DT);
        while ti < w_times.len() && w_times[ti] <= now + 1e-6 {
            input.feed(&press('w'));
            ti += 1;
        }
        if !s_done && now >= s_tap {
            input.feed(&press('s'));
            s_done = true;
        }
        drive(&input, &mut mv);
        if s_done && mv.value() < 0.0 {
            backward_frames += 1;
        }
        last_value = mv.value();
    }
    assert!(
        backward_frames <= 3,
        "stuck moving backward for {backward_frames} frames after a single S tap"
    );
    assert_eq!(last_value, 1.0, "should end moving forward (W still held)");
}

#[test]
fn opposite_then_clear_stops() {
    let mut input = game_map();
    let mut mv = LatchAxis::default();
    let frame = |input: &mut InputMap, mv: &mut LatchAxis, c: char| {
        input.begin_frame(DT);
        input.feed(&press(c));
        drive(input, mv);
    };
    frame(&mut input, &mut mv, 'w');
    assert_eq!(mv.value(), 1.0);
    frame(&mut input, &mut mv, 's');
    assert_eq!(mv.value(), -1.0);
    mv.clear(); // Space
    assert_eq!(mv.value(), 0.0);
}
