//! The tap race: claim, choose, place, decline, and the timeout penalty.

use crate::cards::{card, matches_for_tap, public_card};
use crate::errors::{Handled, err, illegal};
use crate::events::{EngineEvent, EventType, emit, ev};
use crate::state::{
    add_log, append_slot, clear_slot, discard_top, draw_pile_empty, is_slot_occupied, pop_deck, push_discard, set_slot,
    slot_card,
};
use crate::turn::{check_auto_kaboo, reveal_round};
use crate::types::{Action, CardRef, Ctx, EngineErrorCode, GameState, LogEntry, LogKind, Phase, TapState};

/// Tapping is live whenever the table has a discard to match and the round is
/// still being played. It is deliberately forbidden during `FinalSwap` — the
/// house rule says so, and it also stops a tap landing between the last card
/// being drawn and the round ending.
pub fn can_tap_now(state: &GameState) -> bool {
    matches!(state.phase, Phase::TurnStart | Phase::Drawn | Phase::Effect) && discard_top(state).is_some()
}

/// Give the tapper a penalty card from the top of the draw pile.
///
/// Penalty cards ALWAYS land in the hand, even when they turn out to be the
/// last card of the pile — in which case the round ends immediately with no
/// swap offered to anyone.
fn apply_penalty_card(state: &mut GameState, player_id: &str, ctx: &mut Ctx, events: &mut Vec<EngineEvent>) {
    let round = state.round;
    if let Some(penalty) = pop_deck(state) {
        append_slot(state, player_id, penalty);
        add_log(state, LogEntry::new(round, LogKind::TapWrong).actor(player_id).text("penalty"));
    }
    if draw_pile_empty(state) {
        add_log(state, LogEntry::new(round, LogKind::DeckOut).actor(player_id).text("penalty"));
        emit(events, EngineEvent { reason: Some("penalty".into()), ..ev(EventType::DeckOut) });
        reveal_round(state, ctx, events);
        return;
    }
    check_auto_kaboo(state, ctx, events);
}

/// A tap that did not produce a matching card. The named card simply goes
/// back — nothing was moved yet — and the tapper eats a penalty card.
///
/// This is a successful ACTION: it resolves, it does not reject. Returning an
/// error would tell the tapper whether the card matched, which is information
/// they did not have.
fn wrong_tap(state: &mut GameState, player_id: &str, ctx: &mut Ctx, events: &mut Vec<EngineEvent>) -> Handled {
    state.tap = None;
    let round = state.round;
    add_log(state, LogEntry::new(round, LogKind::TapWrong).actor(player_id));
    emit(events, EngineEvent { by: Some(player_id.into()), ..ev(EventType::TapWrong) });
    apply_penalty_card(state, player_id, ctx, events);
    Ok(())
}

fn finish_tap(state: &mut GameState, ctx: &mut Ctx, events: &mut Vec<EngineEvent>) {
    state.tap = None;
    check_auto_kaboo(state, ctx, events);
}

/// Claim the tap. First claim wins; the loser is told only "too late".
fn claim_tap(
    state: &mut GameState,
    player_id: &str,
    seen_seq: u32,
    ctx: &mut Ctx,
    events: &mut Vec<EngineEvent>,
) -> Handled {
    if state.tap.is_some() {
        return Err(err(EngineErrorCode::TooLate));
    }
    if !can_tap_now(state) {
        return Err(illegal());
    }
    // The client tells us which discard top it was reacting to. Tapping a view
    // that has already moved on is refused rather than silently resolved
    // against a card the player never saw.
    if state.rules.tap_closes_on_discard_change && seen_seq != state.discard_seq {
        return Err(err(EngineErrorCode::Stale));
    }

    state.tap = Some(TapState {
        player_id: player_id.to_string(),
        seq: state.discard_seq,
        claimed_at: ctx.now,
        deadline: ctx.now + state.rules.tap_deadline_ms,
        awaiting_placement: false,
        source: None,
    });
    let round = state.round;
    add_log(state, LogEntry::new(round, LogKind::TapClaimed).actor(player_id));
    emit(events, EngineEvent { by: Some(player_id.into()), ..ev(EventType::TapClaimed) });
    Ok(())
}

/// The tapper names the card they are matching with.
fn choose_tap_card(
    state: &mut GameState,
    player_id: &str,
    target: &CardRef,
    ctx: &mut Ctx,
    events: &mut Vec<EngineEvent>,
) -> Handled {
    let Some(tap) = state.tap.as_ref() else { return Err(illegal()) };
    if tap.player_id != player_id || tap.awaiting_placement {
        return Err(illegal());
    }
    if state.rules.tap_closes_on_discard_change && tap.seq != state.discard_seq {
        return wrong_tap(state, player_id, ctx, events);
    }

    let (Some(top_id), Some(picked_id)) = (discard_top(state), slot_card(state, target)) else {
        return wrong_tap(state, player_id, ctx, events);
    };
    if !matches_for_tap(card(picked_id), card(top_id), &state.rules) {
        return wrong_tap(state, player_id, ctx, events);
    }

    clear_slot(state, &target.player_id, target.slot);
    push_discard(state, picked_id);
    if let Some(tap) = state.tap.as_mut() {
        tap.source = Some(target.clone());
    }

    let round = state.round;
    add_log(state, LogEntry::new(round, LogKind::TapSuccess).actor(player_id).target(&target.player_id));
    emit(
        events,
        EngineEvent {
            by: Some(player_id.into()),
            source: Some(target.clone()),
            card: Some(public_card(picked_id)),
            ..ev(EventType::TapResolved)
        },
    );

    if target.player_id == player_id {
        finish_tap(state, ctx, events);
    } else if let Some(tap) = state.tap.as_mut() {
        // They may now drop one of their own cards into the hole they just made.
        tap.awaiting_placement = true;
    }
    Ok(())
}

/// Drop one of your own cards into the slot you just emptied on someone else.
fn place_after_tap(
    state: &mut GameState,
    player_id: &str,
    slot: usize,
    ctx: &mut Ctx,
    events: &mut Vec<EngineEvent>,
) -> Handled {
    let Some(tap) = state.tap.as_ref() else { return Err(illegal()) };
    if tap.player_id != player_id || !tap.awaiting_placement {
        return Err(illegal());
    }
    let Some(source) = tap.source.clone() else { return Err(illegal()) };
    if is_slot_occupied(state, &source) {
        return Err(illegal());
    }
    let own_ref = CardRef { player_id: player_id.to_string(), slot };
    let Some(own) = slot_card(state, &own_ref) else { return Err(illegal()) };

    clear_slot(state, player_id, slot);
    set_slot(state, &source.player_id, source.slot, own);
    let round = state.round;
    add_log(state, LogEntry::new(round, LogKind::Placed).actor(player_id).target(&source.player_id));
    emit(events, EngineEvent { by: Some(player_id.into()), into: Some(source), ..ev(EventType::Placed) });
    finish_tap(state, ctx, events);
    Ok(())
}

fn decline_placement(state: &mut GameState, player_id: &str, ctx: &mut Ctx, events: &mut Vec<EngineEvent>) -> Handled {
    let Some(tap) = state.tap.as_ref() else { return Err(illegal()) };
    if tap.player_id != player_id || !tap.awaiting_placement {
        return Err(illegal());
    }
    finish_tap(state, ctx, events);
    Ok(())
}

/// A claimed tap that was never completed.
///
/// By default it counts as a wrong tap. Without that, "claim a tap, then
/// stall" would be a free way to freeze the table for everyone else.
pub fn resolve_tap_deadline(state: &mut GameState, ctx: &mut Ctx, events: &mut Vec<EngineEvent>) -> bool {
    let Some(tap) = state.tap.as_ref() else { return false };
    if ctx.now < tap.deadline {
        return false;
    }

    let player_id = tap.player_id.clone();
    state.tap = None;
    let round = state.round;
    add_log(state, LogEntry::new(round, LogKind::TapTimeout).actor(&player_id));
    emit(events, EngineEvent { by: Some(player_id.clone()), ..ev(EventType::TapTimeout) });

    if state.rules.tap_timeout_is_wrong_tap {
        apply_penalty_card(state, &player_id, ctx, events);
    }
    true
}

/// Route a tap-family action to its handler.
pub fn handle_tap_action(
    state: &mut GameState,
    action: &Action,
    ctx: &mut Ctx,
    events: &mut Vec<EngineEvent>,
) -> Handled {
    match action {
        Action::Tap { player_id, seen_seq } => claim_tap(state, player_id, *seen_seq, ctx, events),
        Action::TapChoose { player_id, target } => choose_tap_card(state, player_id, target, ctx, events),
        Action::TapPlace { player_id, slot } => place_after_tap(state, player_id, *slot, ctx, events),
        Action::TapDeclinePlace { player_id } => decline_placement(state, player_id, ctx, events),
        _ => Err(illegal()),
    }
}
