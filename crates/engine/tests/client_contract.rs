//! THE CLIENT CONTRACT. These tests play complete games using NOTHING but
//! `project()` output: every choice comes from a projection field, exactly as
//! the UI must.
//!
//! A projection could be perfectly safe (it leaks nothing) and still be
//! useless, because it omits something a client needs in order to act. This
//! file is where that shows up as a red test instead of a dead button. If the
//! UI can do it, a case belongs here.

use kaboo_engine::testkit::{Table, TableOptions, choose_move};
use kaboo_engine::{Action, EngineErrorCode, LogKind, Phase};

/// Drive the table until it stalls, every player acting purely off their projection.
fn play_out(t: &mut Table, max_steps: usize) -> (usize, Option<Phase>) {
    let mut steps = 0;
    let mut idle = 0;
    while steps < max_steps {
        if t.state.phase == Phase::GameOver {
            break;
        }
        // Try every seat in order; the first one with a legal move acts.
        let mut moved = false;
        for id in t.ids.clone() {
            let Some(mv) = choose_move(&t.view(&id), &id) else { continue };
            let kind = mv.kind();
            let result = t.send(mv);
            // A refusal is fine — a stale tap loses the race by design — but it
            // must never be a refusal of something the projection advertised.
            if let Some(e) = &result.error {
                assert!(
                    matches!(e.code, EngineErrorCode::TooLate | EngineErrorCode::Stale),
                    "projection advertised {kind:?} for {id} but the engine refused: {:?}",
                    e.code
                );
            }
            if result.version_changed {
                moved = true;
                break;
            }
        }
        if moved {
            idle = 0;
        } else {
            // Nobody could act. Let the clock run, which is how deadlines resolve.
            idle += 1;
            t.advance(120_000);
            if t.tick().is_none() && idle > 4 {
                return (steps, Some(t.state.phase));
            }
        }
        steps += 1;
    }
    (steps, None)
}

fn seeded(players: usize, seed: u32) -> Table {
    Table::new(TableOptions { players: Some(players), seed: Some(seed), ..Default::default() })
}

#[test]
fn reaches_the_reveal_in_a_two_player_game() {
    let mut t = seeded(2, 20_261_016);
    let (steps, stuck) = play_out(&mut t, 4000);
    assert_eq!(stuck, None, "the table stalled with nobody able to act");
    assert!(matches!(t.state.phase, Phase::Reveal | Phase::GameOver));
    assert!(steps < 1000, "got there by exhausting the step budget");
    assert!(!t.state.discard.is_empty(), "a round of Kaboo must have produced a discard pile");
    assert!(t.state.log.iter().any(|e| e.kind == LogKind::RoundEnd), "the round must have been scored");
}

#[test]
fn reaches_the_reveal_at_every_table_size() {
    for players in 2..=8 {
        let mut t = seeded(players, 4000 + players as u32);
        let (steps, stuck) = play_out(&mut t, 4000);
        assert_eq!(stuck, None, "{players} players stalled");
        assert!(matches!(t.state.phase, Phase::Reveal | Phase::GameOver), "{players} players");
        assert!(steps < 1000, "{players} players ground to the step limit");
    }
}

#[test]
fn lets_a_player_tap_without_being_the_turn_player() {
    // The race is the point of the game, and it is only expressible if the
    // projection ships `can_tap` and `discard_seq` to everyone, not just the actor.
    let mut t = seeded(3, 99);
    t.finish_peek();
    let mut saw_off_turn_tap = false;

    for _ in 0..600 {
        if t.state.phase == Phase::Reveal {
            break;
        }
        let turn = t.turn_id();
        for id in t.ids.clone() {
            let view = t.view(&id);
            if id == turn || !view.can_tap || view.tap.is_some() {
                continue;
            }
            if let Some(mv @ Action::Tap { .. }) = choose_move(&view, &id) {
                saw_off_turn_tap = true;
                assert!(t.send(mv).error.is_none());
                break;
            }
        }
        if saw_off_turn_tap {
            break;
        }
        match choose_move(&t.view(&turn), &turn) {
            Some(mv) => {
                t.send(mv);
            }
            None => t.advance(120_000),
        }
    }
    assert!(saw_off_turn_tap, "no off-turn tap was ever available across 600 steps");
}
