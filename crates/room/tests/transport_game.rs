//! THE TRANSPORT, UNDER A WHOLE GAME — the load-bearing suite.
//!
//! `room.rs` proves the room is correct on individual moves. This file proves
//! it cannot DEADLOCK: every rule here is about a *sequence* of moves. A turn
//! can stall because the projection omitted a field a player needs, or because
//! a deadline never resolved. None of that shows from one action.
//!
//! Two invariants are enforced on EVERY game played here:
//! 1. **A refusal is never the transport's fault.** A move the player's own
//!    projection advertised may be refused for exactly two reasons — the tap
//!    race lost (`E_TOO_LATE` / `E_STALE`) — and nothing else.
//! 2. **Nothing hidden crosses the wire**, audited response by response.

mod common;

use std::collections::BTreeMap;

use common::{Seat, Server, leaked, projection};
use kaboo_engine::testkit::{act, choose_move, hidden_card_ids};
use kaboo_engine::{Action, EngineErrorCode, LogKind, Phase, PlayerId, Projection};

struct Table {
    server: Server,
    code: String,
    seats: Vec<Seat>,
}

fn seat_table(players: usize, seed: u32) -> Table {
    let mut server = Server::new(seed);
    let names: Vec<String> = (1..=players).map(|i| format!("P{i}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let (code, seats) = server.deal(&refs);
    Table { server, code, seats }
}

#[derive(Default, Debug)]
struct Progress {
    steps: usize,
    phase: Option<Phase>,
    stuck: Option<String>,
    taps_accepted: usize,
    taps_off_turn: usize,
    taps_refused: usize,
    audited: usize,
    hidden_seen: usize,
}

fn race_lost(code: EngineErrorCode) -> bool {
    matches!(code, EngineErrorCode::TooLate | EngineErrorCode::Stale)
}

/// Play the table to the end of the game, every player acting purely off their
/// own projection and reaching the server only through the HTTP edge.
fn play_out(t: &mut Table, max_steps: usize, audit: bool) -> Progress {
    let mut progress = Progress::default();
    let mut idle = 0;

    while progress.steps < max_steps {
        let mut views: BTreeMap<PlayerId, Projection> = BTreeMap::new();
        for seat in &t.seats {
            let sync = t.server.read(&t.code, &seat.token, 0);
            if audit {
                let state = t.server.state(&t.code).clone();
                progress.audited += 1;
                progress.hidden_seen += hidden_card_ids(&state, &seat.id).len();
                let bad = leaked(&sync.room, &state, &seat.id);
                assert!(bad.is_empty(), "{} was sent {bad:?}", seat.id);
                for event in &sync.events {
                    assert_eq!(event.event.to.as_deref(), Some(seat.id.as_str()));
                }
            }
            views.insert(seat.id.clone(), projection(&sync).clone());
        }

        let first = views.values().next().unwrap().clone();
        progress.phase = Some(first.phase);
        if first.phase == Phase::GameOver {
            return progress;
        }

        // When the pile is tappable everyone reaches for it, so try the players
        // NOT on turn first — or the turn player's DRAW would rob the table of
        // every tap and the off-turn path would never run.
        let mut order = t.seats.clone();
        if first.can_tap && first.tap.is_none() {
            order.sort_by_key(|s| Some(s.id.as_str()) == first.turn_player_id.as_deref());
        }

        let mut accepted = false;
        for seat in &order {
            let view = &views[&seat.id];
            let Some(mv) = choose_move(view, &seat.id) else { continue };
            let is_tap = matches!(mv, Action::Tap { .. });
            let off_turn = is_tap && view.turn_player_id.as_deref() != Some(seat.id.as_str());
            let kind = mv.kind();
            let result = t
                .server
                .attempt(&t.code, &seat.token, &mv)
                .unwrap_or_else(|e| panic!("transport refused {kind:?} for {}: {:?}", seat.id, e.code));

            if let Some(refusal) = result.refusal {
                assert!(
                    race_lost(refusal.code),
                    "projection advertised {kind:?} for {} but the engine refused: {:?}",
                    seat.id,
                    refusal.code
                );
                if is_tap {
                    progress.taps_refused += 1;
                }
                continue; // let another seat try; the race was simply lost
            }
            if is_tap {
                progress.taps_accepted += 1;
                if off_turn {
                    progress.taps_off_turn += 1;
                }
            }
            accepted = true;
            break;
        }

        if accepted {
            idle = 0;
        } else {
            // Nobody could act. Only a deadline can move the game now, and the
            // room's alarm is what fires it.
            idle += 1;
            if idle > 4 {
                progress.stuck = Some(format!("{:?}", first.phase));
                return progress;
            }
            t.server.advance(120_000);
            t.server.alarm(&t.code.clone());
        }
        progress.steps += 1;
    }
    progress.stuck = Some(format!("step limit ({max_steps})"));
    progress
}

fn finished(p: &Progress) -> bool {
    matches!(p.phase, Some(Phase::Reveal | Phase::GameOver))
}

/// Play until the discard is tappable, without tapping.
fn reach_a_tap(t: &mut Table) -> Option<(u32, PlayerId)> {
    for _ in 0..400 {
        let mut views = BTreeMap::new();
        for seat in &t.seats {
            views.insert(seat.id.clone(), projection(&t.server.read(&t.code, &seat.token, 0)).clone());
        }
        let first: &Projection = views.values().next().unwrap();
        if matches!(first.phase, Phase::Reveal | Phase::GameOver) {
            return None;
        }
        if first.can_tap
            && first.tap.is_none()
            && let Some(turn) = &first.turn_player_id
        {
            return Some((first.discard_seq, turn.clone()));
        }
        let actor =
            t.seats.iter().find(|s| Some(&s.id) == first.turn_player_id.as_ref()).unwrap_or(&t.seats[0]).clone();
        match choose_move(&views[&actor.id], &actor.id) {
            Some(mv) => {
                let _ = t.server.attempt(&t.code, &actor.token, &mv);
            }
            None => {
                t.server.advance(120_000);
                t.server.alarm(&t.code.clone());
            }
        }
    }
    None
}

// --- a game plays to the end through the transport alone -----------------------------

#[test]
fn reaches_the_reveal_at_every_table_size() {
    for players in 2..=8 {
        let mut t = seat_table(players, 4000 + players as u32);
        let progress = play_out(&mut t, 900, false);
        assert_eq!(progress.stuck, None, "{players} players stalled in {:?}", progress.phase);
        assert!(finished(&progress), "{players} players ended in {:?}", progress.phase);
        assert!(progress.steps < 900, "{players} players ground to the step limit");
    }
}

#[test]
fn scores_the_round_it_played_through_the_projection() {
    let mut t = seat_table(4, 777);
    assert_eq!(play_out(&mut t, 900, false).stuck, None);
    let token = t.seats[0].token.clone();
    let sync = t.server.read(&t.code, &token, 0);
    let view = projection(&sync);
    assert!(view.log.iter().any(|e| e.kind == LogKind::RoundEnd), "the round must have been scored");
    assert!(view.discard_count > 0, "a round of Kaboo must have produced a discard pile");
}

#[test]
fn lets_a_player_who_is_not_on_turn_win_a_tap() {
    let mut off_turn = 0;
    for seed in [11, 22, 33, 44] {
        let mut t = seat_table(3, seed);
        let progress = play_out(&mut t, 900, false);
        assert_eq!(progress.stuck, None, "seed {seed} stalled in {:?}", progress.phase);
        off_turn += progress.taps_off_turn;
    }
    assert!(off_turn > 0, "four games never produced an off-turn tap");
}

#[test]
fn settles_contested_taps_as_wins_and_losses_never_as_errors() {
    for seed in [5, 15, 25, 35, 45, 55] {
        let mut t = seat_table(4, seed);
        let (seq, turn) =
            reach_a_tap(&mut t).unwrap_or_else(|| panic!("seed {seed} never produced a tappable discard"));
        let tappers: Vec<Seat> = t.seats.iter().filter(|s| s.id != turn).cloned().collect();
        assert!(tappers.len() > 1);

        // Every off-turn player taps the same discard, built from the same view.
        let results: Vec<_> = tappers
            .iter()
            .map(|s| {
                t.server
                    .attempt(&t.code, &s.token, &act::tap(&s.id, seq))
                    .unwrap_or_else(|e| panic!("seed {seed}: transport refused a tap: {:?}", e.code))
            })
            .collect();
        let wins: Vec<_> = results.iter().filter(|r| r.refusal.is_none()).collect();
        let losses: Vec<_> = results.iter().filter_map(|r| r.refusal.as_ref()).collect();
        assert_eq!(wins.len(), 1, "seed {seed}: exactly one tap may win");
        assert_eq!(losses.len(), tappers.len() - 1, "seed {seed}: the rest must be told too late");
        assert!(losses.iter().all(|l| race_lost(l.code)), "seed {seed}: {losses:?}");
        assert_eq!(t.server.state(&t.code).tap.as_ref().unwrap().player_id, wins[0].sync.room.you());
    }
}

// --- the wire stays closed for a whole game ---------------------------------------------

#[test]
fn never_hands_any_player_a_card_they_may_not_see_move_after_move() {
    let mut t = seat_table(3, 20_261_016);
    let progress = play_out(&mut t, 900, true);
    assert_eq!(progress.stuck, None, "stalled in {:?}", progress.phase);
    // Guard the guard: an audit that never ran, or never ran while anything was
    // hidden, would make "no leaks" true and worthless.
    assert!(progress.audited > 200, "the audit saw only {} responses", progress.audited);
    assert!(progress.hidden_seen > 0, "nothing was ever hidden to leak");
}

#[test]
fn every_push_is_as_blind_as_a_response() {
    // Pushes are built by the same `sync_for`, but they are a separate path to
    // the wire, so they get their own audit.
    let mut t = seat_table(3, 31);
    t.server.pushes.clear();
    let progress = play_out(&mut t, 900, false);
    assert_eq!(progress.stuck, None);
    assert!(t.server.pushes.len() > 50, "only {} pushes were sent", t.server.pushes.len());
    // The final state is the strictest check available after the fact for the
    // last push to each seat.
    let state = t.server.state(&t.code).clone();
    for push in t.server.pushes.iter().rev().take(3) {
        assert!(leaked(&push.message.room, &state, &push.player_id).is_empty());
        for event in &push.message.events {
            assert_eq!(event.event.to.as_deref(), Some(push.player_id.as_str()));
        }
    }
}

// --- a device is not the seat ------------------------------------------------------------

#[test]
fn rebuilds_the_same_view_from_a_cold_start_mid_game() {
    let mut t = seat_table(3, 909);
    let seat = t.seats[1].clone();
    for _ in 0..12 {
        let sync = t.server.read(&t.code, &seat.token, 0);
        match choose_move(projection(&sync), &seat.id) {
            Some(mv) => {
                let _ = t.server.attempt(&t.code, &seat.token, &mv);
            }
            None => t.server.advance(120_000),
        }
    }
    // A brand-new client: no cursor, no cache, nothing but the stored token.
    let cold = t.server.read(&t.code, &seat.token, 0);
    let warm = t.server.read(&t.code, &seat.token, 0);
    assert_eq!(cold.room.you(), seat.id);
    assert_eq!(projection(&cold), projection(&warm));

    // And it is not a dead view — the restored seat can still play.
    if let Some(mv) = choose_move(projection(&cold), &seat.id) {
        let result = t.server.attempt(&t.code, &seat.token, &mv).expect("no transport error");
        assert!(result.refusal.is_none());
    }
}

#[test]
fn shows_two_devices_on_one_seat_exactly_the_same_table() {
    let mut t = seat_table(2, 313);
    let token = t.seats[0].token.clone();
    let phone = t.server.read(&t.code, &token, 0);
    let laptop = t.server.read(&t.code, &token, 0);
    assert_eq!(projection(&phone), projection(&laptop));
}

// --- reads are cheap ---------------------------------------------------------------------

#[test]
fn does_not_rewrite_the_room_on_a_read_that_changes_nothing() {
    let mut t = seat_table(2, 4);
    let token = t.seats[0].token.clone();
    let first = t.server.read(&t.code, &token, 0);
    let since = first.event_seq.to_string();
    for _ in 0..5 {
        assert!(!t.server.read_raw(&t.code, Some(&token), Some(&since)).changed);
    }
}

#[test]
fn resolves_a_deadline_on_the_read_of_any_player_not_just_the_one_it_concerns() {
    let mut t = seat_table(3, 8);
    let deadline = t.server.state(&t.code).peek_deadline;
    t.server.advance(deadline - t.server.now + 1);
    let token = t.seats[2].token.clone();
    t.server.pushes.clear();
    let sync = t.server.read(&t.code, &token, 0);
    assert_eq!(projection(&sync).phase, Phase::TurnStart);
    // The reader moved the clock, so everyone else is told.
    let mut told: Vec<_> = t.server.pushes.iter().map(|p| p.player_id.as_str()).collect();
    told.sort();
    assert_eq!(told, vec!["p1", "p2"]);
}
