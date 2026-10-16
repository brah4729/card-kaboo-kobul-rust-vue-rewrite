//! THE ROOM — rooms, seats, and the race the whole design rests on.
//!
//! Three claims worth reading twice:
//! * **The tap race.** Two players tap against the same view; exactly one wins
//!   and the other is told "too late".
//! * **The session is the identity.** A player cannot act as another player by
//!   putting a different id in the request body.
//! * **The wire carries no hidden face.** Checked against the shape of the whole
//!   payload, not against the fields we happen to remember.

mod common;

use common::{Server, leaked, projection};
use kaboo_engine::testkit::act;
use kaboo_engine::{Action, DrawSource, EngineErrorCode, Phase, PlayerId, Projection};
use kaboo_protocol::{RoomErrorCode, RoomView};
use kaboo_room::{Alarm, ROOM_TTL_MS, next_alarm};

/// One action for the player whose turn it is, from their own projection.
fn turn_move(view: &Projection, actor: &str) -> Option<Action> {
    use kaboo_engine::ActionType::*;
    if view.turn_player_id.as_deref() != Some(actor) {
        return None;
    }
    let slot = view.players.iter().find(|p| p.id == actor)?.hand.iter().find(|s| s.occupied).map(|s| s.index);
    let legal = &view.legal_actions;
    if legal.contains(&Draw) {
        let from = if view.discard_top.is_some() { DrawSource::Discard } else { DrawSource::Deck };
        return Some(Action::Draw { player_id: actor.into(), from });
    }
    if legal.contains(&Swap)
        && let Some(slot) = slot
    {
        return Some(act::swap(actor, slot));
    }
    legal.contains(&Discard).then(|| act::discard(actor))
}

/// Play until somebody could tap the pile, without ever tapping.
fn reach_a_tap(server: &mut Server, code: &str, seats: &[common::Seat]) -> Option<(u32, PlayerId)> {
    for _ in 0..600 {
        let state = server.state(code);
        if matches!(state.phase, Phase::Reveal | Phase::GameOver) {
            return None;
        }
        let turn = state.players[state.turn_seat].id.clone();
        let actor = seats.iter().find(|s| s.id == turn)?.clone();
        let sync = server.read(code, &actor.token, 0);
        let view = projection(&sync).clone();
        if view.phase == Phase::Peek {
            // Let the peek clock expire rather than readying each seat by hand.
            server.advance(60_000);
            continue;
        }
        if view.can_tap && view.tap.is_none() {
            return Some((view.discard_seq, turn));
        }
        match turn_move(&view, &actor.id) {
            Some(mv) => {
                server.act(code, &actor.token, &mv);
            }
            None => server.advance(120_000),
        }
    }
    None
}

// --- rooms and seats -------------------------------------------------------------

#[test]
fn creates_a_room_seats_the_creator_and_deals_once_everyone_is_in() {
    let mut server = Server::new(7);
    let (code, seat) = server.create("Ana");
    assert_eq!(code.len(), 4);
    assert_eq!(seat.id, "p1");

    let lobby = server.read(&code, &seat.token, 0);
    let RoomView::Lobby { host, seats, roster, .. } = &lobby.room else { panic!("expected a lobby") };
    assert!(host);
    assert_eq!(*seats, 8);
    assert_eq!(roster.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), vec!["Ana"]);

    server.join(&code, "Bo");
    let dealt = server.start(&code, &seat.token);
    let view = projection(&dealt);
    assert_eq!(view.phase, Phase::Peek);
    assert_eq!(view.round, 1);
}

#[test]
fn refuses_a_ninth_player_and_a_second_start() {
    let mut server = Server::new(7);
    let (code, host) = server.create("p1");
    for i in 2..=8 {
        server.join(&code, &format!("p{i}"));
    }
    assert_eq!(server.join_raw(&code, "p9").status, 409);
    server.start(&code, &host.token);
    assert!(server.join_raw(&code, "Late").body.contains("E_ALREADY_STARTED"));
    assert!(server.start_raw(&code, Some(&host.token)).body.contains("E_ALREADY_STARTED"));
}

#[test]
fn only_the_host_can_deal_and_only_to_a_table_of_at_least_two() {
    let mut server = Server::new(7);
    let (code, host) = server.create("Ana");
    assert!(server.start_raw(&code, Some(&host.token)).body.contains("E_BAD_REQUEST"));
    let bo = server.join(&code, "Bo");
    assert!(server.start_raw(&code, Some(&bo.token)).body.contains("E_BAD_SESSION"));
    assert_eq!(server.start_raw(&code, Some(&host.token)).status, 200);
}

#[test]
fn gives_each_seat_its_own_token_and_the_token_names_that_seat() {
    let mut server = Server::new(7);
    let (code, seats) = server.deal(&["Ana", "Bo"]);
    assert_eq!(seats.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["p1", "p2"]);
    assert_ne!(seats[0].token, seats[1].token);
    for seat in &seats {
        assert_eq!(server.read(&code, &seat.token, 0).room.you(), seat.id);
    }
}

// --- the session is the identity ----------------------------------------------------

#[test]
fn ignores_a_player_id_forged_into_the_action_body() {
    let mut server = Server::new(7);
    let (code, seats) = server.deal(&["Ana", "Bo"]);
    let state = server.state(&code);
    assert_eq!(state.players[state.turn_seat].id, "p2");

    // Ana is not on turn and claims to be Bo. The engine must see Ana.
    let forged = Action::Draw { player_id: seats[1].id.clone(), from: DrawSource::Deck };
    let result = server.act(&code, &seats[0].token, &forged);
    assert!(result.refusal.is_some());

    // And nothing moved: it is still Bo's turn, and no card was drawn.
    let after = server.state(&code);
    assert_eq!(after.players[after.turn_seat].id, "p2");
    assert!(after.drawn.is_none());
}

#[test]
fn rejects_a_token_minted_for_a_different_room() {
    let mut server = Server::new(7);
    let (_, a) = server.deal(&["Ana", "Bo"]);
    let (b, _) = server.deal(&["Cy", "Di"]);
    assert_eq!(server.try_read(&b, Some(&a[0].token), 0).unwrap_err().code, RoomErrorCode::BadSession);
}

#[test]
fn rejects_a_tampered_or_missing_token() {
    let mut server = Server::new(7);
    let (code, seats) = server.deal(&["Ana", "Bo"]);
    let token = &seats[0].token;
    let mut chars: Vec<char> = token.chars().collect();
    let i = chars.len() - 2;
    chars[i] = if chars[i] == 'A' { 'B' } else { 'A' };
    let tampered: String = chars.into_iter().collect();
    assert_eq!(server.try_read(&code, Some(&tampered), 0).unwrap_err().code, RoomErrorCode::BadSession);
    assert_eq!(server.try_read(&code, None, 0).unwrap_err().code, RoomErrorCode::BadSession);
}

// --- the tap race --------------------------------------------------------------------

fn race(seed: u32, reverse: bool) {
    let mut server = Server::new(seed);
    let (code, seats) = server.deal(&["Ana", "Bo", "Cy"]);
    let (seq, turn) = reach_a_tap(&mut server, &code, &seats).expect("never reached a tappable discard");
    let mut tappers: Vec<_> = seats.iter().filter(|s| s.id != turn).cloned().collect();
    if reverse {
        tappers.reverse();
    }
    assert_eq!(tappers.len(), 2);

    // Both taps are built from the same view — what a slow network does to two
    // near-simultaneous taps. The room runs them one at a time, in arrival order.
    let first = server.act(&code, &tappers[0].token, &act::tap(&tappers[0].id, seq));
    let second = server.act(&code, &tappers[1].token, &act::tap(&tappers[1].id, seq));

    assert!(first.refusal.is_none(), "the first tap to arrive wins");
    assert_eq!(second.refusal.unwrap().code, EngineErrorCode::TooLate);
    assert_eq!(first.sync.room.you(), tappers[0].id);
    assert_eq!(server.state(&code).tap.as_ref().unwrap().player_id, tappers[0].id);
}

#[test]
fn settles_two_simultaneous_taps_into_one_winner_and_one_too_late() {
    race(21, false);
}

#[test]
fn says_too_late_whichever_order_the_two_taps_arrive_in() {
    race(33, true);
}

#[test]
fn refuses_a_tap_built_from_a_discard_that_has_since_changed() {
    let mut server = Server::new(45);
    let (code, seats) = server.deal(&["Ana", "Bo"]);
    let (seq, turn) = reach_a_tap(&mut server, &code, &seats).expect("a tappable discard");
    let actor = seats.iter().find(|s| s.id != turn).unwrap().clone();
    let result = server.act(&code, &actor.token, &act::tap(&actor.id, seq + 1));
    assert_eq!(result.refusal.unwrap().code, EngineErrorCode::Stale);
}

// --- nothing leaks through the wire -----------------------------------------------------

#[test]
fn never_puts_a_hidden_card_in_any_players_sync_payload() {
    let mut server = Server::new(11);
    let (code, seats) = server.deal(&["Ana", "Bo", "Cy"]);
    for _ in 0..40 {
        if server.state(&code).phase == Phase::GameOver {
            break;
        }
        for seat in &seats {
            let sync = server.read(&code, &seat.token, 0);
            let state = server.state(&code).clone();
            let bad = leaked(&sync.room, &state, &seat.id);
            assert!(bad.is_empty(), "{} could see cards {bad:?}", seat.id);
            // A private event is the one place a face may travel, so routing is
            // the safety property — not the contents.
            for event in &sync.events {
                assert_eq!(event.event.to.as_deref(), Some(seat.id.as_str()));
            }
        }
        let state = server.state(&code);
        let turn = state.players[state.turn_seat].id.clone();
        let actor = seats.iter().find(|s| s.id == turn).unwrap().clone();
        let sync = server.read(&code, &actor.token, 0);
        let view = projection(&sync).clone();
        if view.phase == Phase::Peek {
            server.advance(60_000);
            continue;
        }
        match turn_move(&view, &actor.id) {
            Some(mv) => {
                server.act(&code, &actor.token, &mv);
            }
            None => server.advance(120_000),
        }
    }
}

#[test]
fn hands_a_peek_only_to_the_player_who_earned_it() {
    let mut server = Server::new(5);
    let (code, seats) = server.deal(&["Ana", "Bo"]);
    let peeked = server.act(&code, &seats[0].token, &act::peek("p1", 2));
    assert!(peeked.refusal.is_none());
    assert_eq!(peeked.sync.events.len(), 1);
    assert_eq!(peeked.sync.events[0].event.to.as_deref(), Some("p1"));
    assert!(peeked.sync.events[0].event.card.is_some());

    // Bo is handed nothing, and can see nothing extra.
    let bo = server.read(&code, &seats[1].token, 0);
    assert!(bo.events.is_empty());
    let state = server.state(&code).clone();
    assert!(leaked(&bo.room, &state, "p2").is_empty());
}

#[test]
fn still_delivers_a_peek_whose_push_never_arrived() {
    let mut server = Server::new(5);
    let (code, seats) = server.deal(&["Ana", "Bo"]);
    let ana = &seats[0].token.clone();
    let before = server.read(&code, ana, 0).event_seq;
    let peeked = server.act(&code, ana, &act::peek("p1", 2));
    let card = peeked.sync.events[0].event.card;

    // A client that missed the push asks from its old cursor and still gets it.
    let replay = server.read(&code, ana, before);
    assert_eq!(replay.events.iter().map(|e| e.event.card).collect::<Vec<_>>(), vec![card]);

    // Having collected it, asking from the new cursor returns nothing.
    assert!(server.read(&code, ana, replay.event_seq).events.is_empty());
}

// --- the clock --------------------------------------------------------------------------

#[test]
fn resolves_a_peek_deadline_on_the_next_read_too() {
    let mut server = Server::new(7);
    let (code, seats) = server.deal(&["Ana", "Bo"]);
    let deadline = server.state(&code).peek_deadline;
    server.advance(deadline - server.now + 1);
    let sync = server.read(&code, &seats[0].token, 0);
    assert_eq!(projection(&sync).phase, Phase::TurnStart);
}

#[test]
fn does_not_write_when_a_read_has_nothing_to_do() {
    let mut server = Server::new(7);
    let (code, seats) = server.deal(&["Ana", "Bo"]);
    let first = server.read(&code, &seats[0].token, 0);
    let since = first.event_seq.to_string();
    let response = server.read_raw(&code, Some(&seats[0].token), Some(&since));
    assert!(!response.changed, "a no-op read must not rewrite the room");
}

#[test]
fn the_alarm_fires_a_deadline_with_nobody_touching_the_room() {
    // What replaced polling: the room's own alarm. Nobody reads, nobody acts.
    let mut server = Server::new(7);
    let (code, _) = server.deal(&["Ana", "Bo"]);
    let due = next_alarm(server.record(&code).unwrap());
    assert_eq!(due, server.state(&code).peek_deadline, "the alarm is set for the peek deadline");

    server.now = due - 1;
    assert_eq!(server.alarm(&code), Alarm::Idle, "nothing is due a millisecond early");

    server.now = due;
    server.pushes.clear();
    let Alarm::Ticked(pushes) = server.alarm(&code) else { panic!("the deadline should have fired") };
    assert_eq!(server.state(&code).phase, Phase::TurnStart);
    // Everyone is told, because nobody asked.
    let mut told: Vec<_> = pushes.iter().map(|p| p.player_id.as_str()).collect();
    told.sort();
    assert_eq!(told, vec!["p1", "p2"]);
}

#[test]
fn the_alarm_reaps_a_room_nobody_has_touched_for_its_ttl() {
    let mut server = Server::new(7);
    let (code, seats) = server.deal(&["Ana", "Bo"]);
    // Expiry is checked before any deadline: an abandoned table is reaped, not played on.
    server.advance(ROOM_TTL_MS + 1);
    assert_eq!(server.alarm(&code), Alarm::Expired);
    assert!(server.record(&code).is_none());
    assert_eq!(server.try_read(&code, Some(&seats[0].token), 0).unwrap_err().code, RoomErrorCode::NotFound);
}

// --- fan-out ------------------------------------------------------------------------------

#[test]
fn pushes_a_move_to_every_other_seat_and_never_to_the_mover() {
    let mut server = Server::new(7);
    let (code, seats) = server.deal(&["Ana", "Bo", "Cy"]);
    server.pushes.clear();
    server.act(&code, &seats[0].token, &act::peek("p1", 2));
    let mut told: Vec<_> = server.pushes.iter().map(|p| p.player_id.clone()).collect();
    told.sort();
    assert_eq!(told, vec!["p2", "p3"]);
    // And what they are told carries no peek: it was not theirs.
    for push in &server.pushes {
        assert!(push.message.events.is_empty());
        assert_eq!(push.message.room.you(), push.player_id);
    }
}

#[test]
fn pushes_nothing_for_a_refused_move_that_moved_nothing() {
    let mut server = Server::new(7);
    let (code, seats) = server.deal(&["Ana", "Bo"]);
    server.pushes.clear();
    let result = server.act(&code, &seats[0].token, &act::peek("p1", 0)); // top row: refused
    assert!(result.refusal.is_some());
    assert!(server.pushes.is_empty());
}

// --- leaving -----------------------------------------------------------------------------

#[test]
fn frees_the_seat_before_the_deal_but_never_reuses_the_id() {
    let mut server = Server::new(7);
    let (code, _) = server.create("Ana");
    let bo = server.join(&code, "Bo");
    assert_eq!(server.leave_raw(&code, Some(&bo.token)).status, 200);
    let names: Vec<_> = server.record(&code).unwrap().roster.iter().map(|r| r.name.clone()).collect();
    assert_eq!(names, vec!["Ana"]);

    // The next arrival gets a NEW id. Reusing p2 would let the departed Bo's
    // token — still validly signed, still naming p2 — act as the newcomer.
    let again = server.join(&code, "Bo");
    assert_eq!(again.id, "p3");
    assert_eq!(server.try_read(&code, Some(&bo.token), 0).unwrap_err().code, RoomErrorCode::BadSession);
}

#[test]
fn keeps_the_seat_after_the_deal_and_only_marks_the_player_gone() {
    let mut server = Server::new(7);
    let (code, seats) = server.deal(&["Ana", "Bo"]);
    server.leave_raw(&code, Some(&seats[1].token));
    let record = server.record(&code).unwrap();
    assert_eq!(record.roster.len(), 2, "dropping a player must not renumber everybody");
    assert!(!record.state.as_ref().unwrap().players.iter().find(|p| p.id == "p2").unwrap().connected);
}

#[test]
fn hands_the_host_over_when_the_host_leaves_the_lobby() {
    let mut server = Server::new(7);
    let (code, host) = server.create("Ana");
    server.join(&code, "Bo");
    server.leave_raw(&code, Some(&host.token));
    let record = server.record(&code).unwrap();
    assert_eq!(record.host_id, "p2");
    assert_eq!(record.roster.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), vec!["Bo"]);
}

// --- input is not trusted ------------------------------------------------------------------

#[test]
fn rejects_an_empty_name_and_caps_a_long_one_at_twelve_characters() {
    let mut server = Server::new(7);
    assert_eq!(server.create_raw(r#"{"name":"   "}"#).status, 400);
    assert_eq!(server.create_raw(r#"{"name":42}"#).status, 400);
    let (code, _) = server.create("  Ana With A Very Long Name  ");
    assert_eq!(server.record(&code).unwrap().roster[0].name, "Ana With A V");
}

#[test]
fn refuses_an_action_while_the_room_is_still_a_lobby() {
    let mut server = Server::new(7);
    let (code, host) = server.create("Ana");
    server.join(&code, "Bo");
    let err = server.attempt(&code, &host.token, &act::draw("p1")).unwrap_err();
    assert_eq!(err.code, RoomErrorCode::BadRequest);
}
