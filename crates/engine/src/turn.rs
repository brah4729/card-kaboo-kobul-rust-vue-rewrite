//! The turn ring: advancing play, KABOO's final turns, and the end of a round.

use std::collections::BTreeMap;

use crate::cards::public_card;
use crate::events::{EngineEvent, EventType, emit, ev};
use crate::scoring::score_round;
use crate::state::{add_log, append_slot, get_player_mut, next_eligible_seat};
use crate::types::{Ctx, GameState, LogEntry, LogKind, Phase, PublicCard};

/// Take `id` out of the queue of players still owed a final turn.
///
/// Removing them from the list is what makes "exactly one final turn" true:
/// the seat ring skips anyone no longer in it.
pub fn mark_final_turn_done(state: &mut GameState, id: &str) {
    state.final_turns_remaining.retain(|p| p != id);
    if let Some(p) = get_player_mut(state, id) {
        p.final_turn_done = true;
    }
}

/// A KABOO has been called (or auto-called). Every OTHER player now gets
/// exactly one more turn, and the caller gets none.
pub fn begin_final_turns(state: &mut GameState, caller_id: &str) {
    state.kaboo_caller_id = Some(caller_id.to_string());
    state.final_turns_remaining = state.players.iter().filter(|p| p.id != caller_id).map(|p| p.id.clone()).collect();
    for p in &mut state.players {
        p.final_turn_done = p.id == caller_id;
    }
}

/// Public snapshot of every hand — only ever used once the round is over.
pub fn revealed_hands(state: &GameState) -> BTreeMap<String, Vec<Option<PublicCard>>> {
    state.players.iter().map(|p| (p.id.clone(), p.hand.iter().map(|s| s.map(public_card)).collect())).collect()
}

/// Put any card that is "in front of" a player back into the accounting.
///
/// A round can end while somebody is mid-decision. The reachable case is the
/// house rule's own edge: a wrong tap draws the LAST card of the draw pile as a
/// penalty, which ends the round immediately — while the turn player is still
/// holding a card they just drew. That card is physically in their hand, so it
/// goes into their hand. Clearing the field would delete the card from the game.
///
/// A deck-out card still under consideration when the round ended is treated
/// as declined: set aside, and not scored.
fn settle_loose_cards(state: &mut GameState) {
    if let Some(d) = state.drawn.take() {
        append_slot(state, &d.by, d.card_id);
    }
    if let Some(held) = state.held_card_id.take()
        && state.set_aside.is_none()
    {
        state.set_aside = Some(held);
    }
}

/// End the round: flip everything face-up, score it, and park in `Reveal`.
///
/// From here the round result is public — and only from here. Nothing before
/// this point may compute or expose a total.
pub fn reveal_round(state: &mut GameState, _ctx: &mut Ctx, events: &mut Vec<EngineEvent>) {
    if matches!(state.phase, Phase::Reveal | Phase::GameOver) {
        return;
    }

    state.phase = Phase::Reveal;
    settle_loose_cards(state);
    state.tap = None;
    state.effect = None;
    state.turn_deadline = 0;
    state.final_swap_deadline = 0;
    state.peek_deadline = 0;

    let result = score_round(state);
    state.round_result = Some(result.clone());

    let round = state.round;
    add_log(state, LogEntry::new(round, LogKind::Reveal));
    add_log(state, LogEntry::new(round, LogKind::RoundEnd));

    emit(
        events,
        EngineEvent {
            round: Some(round),
            hands: Some(revealed_hands(state)),
            set_aside: Some(state.set_aside.map(public_card)),
            totals: Some(result.totals.clone()),
            winner_id: Some(result.winner_id.clone()),
            penalty_ids: Some(result.penalty_ids.clone()),
            ..ev(EventType::Reveal)
        },
    );
    emit(events, EngineEvent { round_result: Some(result), ..ev(EventType::RoundEnd) });
}

/// Finish the current player's turn and hand play to the next eligible seat.
///
/// When a KABOO is live this also retires the current player from the
/// final-turn queue and ends the round once the queue drains.
pub fn advance_turn(state: &mut GameState, ctx: &mut Ctx, events: &mut Vec<EngineEvent>) {
    state.drawn = None;
    state.effect = None;
    state.turn_deadline = 0;
    state.final_swap_deadline = 0;

    let current = state
        .players
        .get(state.turn_seat)
        .unwrap_or_else(|| panic!("kaboo: no player at seat {}", state.turn_seat))
        .id
        .clone();

    if state.kaboo_caller_id.is_some() {
        mark_final_turn_done(state, &current);
        if state.final_turns_remaining.is_empty() {
            reveal_round(state, ctx, events);
            return;
        }
    }

    state.turn_count += 1;

    // The guard that makes "a round always terminates" true even if the table
    // ping-pongs the discard pile forever without consuming the draw pile.
    if state.turn_count >= state.rules.turn_limit {
        let round = state.round;
        add_log(state, LogEntry::new(round, LogKind::TurnLimit));
        emit(events, EngineEvent { limit: Some(state.rules.turn_limit), ..ev(EventType::TurnLimit) });
        reveal_round(state, ctx, events);
        return;
    }

    let next = next_eligible_seat(state, state.turn_seat);
    state.turn_seat = next;
    state.phase = Phase::TurnStart;
    state.turn_deadline = ctx.now + state.rules.turn_deadline_ms;
    emit(
        events,
        EngineEvent { seat: Some(next), player_id: Some(state.players[next].id.clone()), ..ev(EventType::TurnChanged) },
    );
}

/// HOUSE RULE: losing all your cards auto-KABOOs. Returns true when it fired.
///
/// Only checked when no call is already live — a second call would restart the
/// final-turn queue and let a round run indefinitely.
pub fn check_auto_kaboo(state: &mut GameState, _ctx: &mut Ctx, events: &mut Vec<EngineEvent>) -> bool {
    if state.kaboo_caller_id.is_some() {
        return false;
    }
    if matches!(state.phase, Phase::Reveal | Phase::GameOver) {
        return false;
    }

    let Some(idx) = state.players.iter().position(|p| p.hand.iter().all(Option::is_none)) else {
        return false;
    };
    state.players[idx].called_kaboo = true;
    let id = state.players[idx].id.clone();
    let round = state.round;
    add_log(state, LogEntry::new(round, LogKind::AutoKaboo).actor(&id));
    emit(events, EngineEvent { by: Some(id.clone()), ..ev(EventType::AutoKaboo) });
    begin_final_turns(state, &id);
    true
}
