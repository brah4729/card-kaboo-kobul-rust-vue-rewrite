//! The engine's public entry points: `apply_action`, `tick`, `start_game`.

use crate::cards::{card, effect_for, public_card};
use crate::effects::resolve_effect;
use crate::errors::{Handled, err, illegal};
use crate::events::{EngineEvent, EventType, emit, emit_private, ev};
use crate::scoring::{build_game_result, should_game_end};
use crate::state::{
    PEEK_SLOTS, add_log, create_game, discard_top, draw_pile_empty, get_player, get_player_mut, pop_deck, push_discard,
    set_slot, start_next_round, turn_player_id,
};
use crate::tap::{handle_tap_action, resolve_tap_deadline};
use crate::turn::{advance_turn, begin_final_turns, reveal_round};
use crate::types::{
    Action, Ctx, DrawSource, Drawn, EffectState, EngineError, EngineErrorCode, GameState, LogEntry, LogKind, Phase,
    RuleConfig, Seat,
};

#[derive(Debug, Clone)]
pub struct ApplyResult {
    /// The state to persist. Identical to the input when nothing happened.
    pub state: GameState,
    pub events: Vec<EngineEvent>,
    /// Set when the ACTION was refused. `state` still reflects any deadline
    /// resolution that happened while refusing it, so a timeout is never thrown
    /// away just because the player's next action was invalid. See AGENTS.md §7.
    pub error: Option<EngineError>,
    /// True when `state` differs from the input and should be written back.
    pub version_changed: bool,
}

// ---------------------------------------------------------------------------
// Deadlines. Stored timestamps, resolved whenever anyone touches the room —
// and by the room's alarm, so a table with nobody acting still moves.
// ---------------------------------------------------------------------------

/// All players are ready (or timed out) — begin the first turn of the round.
fn maybe_start_turns(state: &mut GameState, ctx: &mut Ctx, events: &mut Vec<EngineEvent>) {
    if state.phase != Phase::Peek || !state.players.iter().all(|p| p.ready) {
        return;
    }
    state.peek_deadline = 0;
    state.phase = Phase::TurnStart;
    state.turn_deadline = ctx.now + state.rules.turn_deadline_ms;
    emit(
        events,
        EngineEvent { seat: Some(state.turn_seat), player_id: turn_player_id(state), ..ev(EventType::TurnChanged) },
    );
}

fn auto_decline_final_swap(state: &mut GameState, events: &mut Vec<EngineEvent>) {
    let Some(held) = state.held_card_id.take() else { return };
    state.set_aside = Some(held);
    let by = turn_player_id(state);
    let mut entry = LogEntry::new(state.round, LogKind::FinalDecline).text("timeout");
    entry.actor = by.clone();
    add_log(state, entry);
    emit(events, EngineEvent { by, reason: Some("timeout".into()), ..ev(EventType::FinalDecline) });
}

/// Play one safe default for a player who has run out of time.
///
/// Safe means: never call KABOO for them, never swap away a card they chose to
/// keep, never guess at an effect. Discarding the drawn card is the one move
/// that costs them nothing they had before this turn.
fn auto_play_turn(state: &mut GameState, ctx: &mut Ctx, events: &mut Vec<EngineEvent>) {
    let Some(actor) = turn_player_id(state) else { return };
    state.turn_deadline = 0;

    let player_id = actor.clone();
    let fallback = match state.phase {
        Phase::TurnStart => Action::Draw { player_id, from: DrawSource::Deck },
        Phase::Drawn => Action::Discard { player_id },
        Phase::Effect => Action::EffectSkip { player_id },
        _ => return,
    };

    add_log(state, LogEntry::new(state.round, LogKind::TurnLimit).actor(&actor).text("timeout"));
    emit(
        events,
        EngineEvent {
            seat: Some(state.turn_seat),
            player_id: Some(actor),
            reason: Some("timeout".into()),
            ..ev(EventType::TurnChanged)
        },
    );
    // A refusal here is impossible by construction (each fallback is legal in
    // its phase), and if it ever were, the deadline is already cleared.
    let _ = dispatch(state, &fallback, ctx, events);
}

fn run_deadlines(state: &mut GameState, ctx: &mut Ctx, events: &mut Vec<EngineEvent>) {
    if state.phase == Phase::GameOver {
        return;
    }

    if state.phase == Phase::Peek && state.peek_deadline > 0 && ctx.now >= state.peek_deadline {
        for p in &mut state.players {
            p.ready = true;
        }
        add_log(state, LogEntry::new(state.round, LogKind::Ready).text("timeout"));
        maybe_start_turns(state, ctx, events);
    }

    if state.tap.is_some() {
        resolve_tap_deadline(state, ctx, events);
    }

    if state.phase == Phase::FinalSwap && state.final_swap_deadline > 0 && ctx.now >= state.final_swap_deadline {
        auto_decline_final_swap(state, events);
        reveal_round(state, ctx, events);
    }

    // The turn clock only runs while nobody is mid-tap: a claimed tap owns the
    // table until it resolves.
    if state.tap.is_none() && state.turn_deadline > 0 && ctx.now >= state.turn_deadline {
        auto_play_turn(state, ctx, events);
    }
}

/// The earliest moment at which `tick` would change something, if any. The
/// room sets its alarm from this so deadlines fire with every client idle.
pub fn next_deadline(state: &GameState) -> Option<i64> {
    if state.phase == Phase::GameOver {
        return None;
    }
    let mut candidates = Vec::new();
    if state.phase == Phase::Peek && state.peek_deadline > 0 {
        candidates.push(state.peek_deadline);
    }
    if let Some(tap) = &state.tap {
        candidates.push(tap.deadline);
    }
    if state.phase == Phase::FinalSwap && state.final_swap_deadline > 0 {
        candidates.push(state.final_swap_deadline);
    }
    if state.tap.is_none() && state.turn_deadline > 0 {
        candidates.push(state.turn_deadline);
    }
    candidates.into_iter().min()
}

// ---------------------------------------------------------------------------
// Action handlers
// ---------------------------------------------------------------------------

fn require_turn(state: &GameState, player_id: &str) -> Handled {
    match state.players.get(state.turn_seat) {
        Some(p) if p.id == player_id => Ok(()),
        _ => Err(err(EngineErrorCode::NotYourTurn)),
    }
}

/// No turn-spine action is legal while a tap claim holds the table.
fn blocked_by_tap(state: &GameState) -> Handled {
    if state.tap.is_some() { Err(illegal()) } else { Ok(()) }
}

fn handle_peek(state: &mut GameState, player_id: &str, slot: usize, events: &mut Vec<EngineEvent>) -> Handled {
    if state.phase != Phase::Peek {
        return Err(illegal());
    }
    let p = get_player_mut(state, player_id).ok_or_else(illegal)?;
    if p.ready || !PEEK_SLOTS.contains(&slot) || p.peeked.get(slot).copied().unwrap_or(true) {
        return Err(illegal());
    }
    let Some(id) = p.hand.get(slot).copied().flatten() else { return Err(illegal()) };

    p.peeked[slot] = true;
    emit_private(
        events,
        player_id,
        EngineEvent {
            player_id: Some(player_id.into()),
            slot: Some(slot),
            card: Some(public_card(id)),
            reason: Some("peek".into()),
            ..ev(EventType::Peeked)
        },
    );
    add_log(state, LogEntry::new(state.round, LogKind::Peeked).actor(player_id));
    Ok(())
}

fn handle_ready(state: &mut GameState, player_id: &str, ctx: &mut Ctx, events: &mut Vec<EngineEvent>) -> Handled {
    if state.phase != Phase::Peek {
        return Err(illegal());
    }
    get_player_mut(state, player_id).ok_or_else(illegal)?.ready = true;
    add_log(state, LogEntry::new(state.round, LogKind::Ready).actor(player_id));
    emit(
        events,
        EngineEvent { phase: Some(Phase::Peek), ready_id: Some(player_id.into()), ..ev(EventType::PhaseChanged) },
    );
    maybe_start_turns(state, ctx, events);
    Ok(())
}

fn handle_continue(state: &mut GameState, ctx: &mut Ctx, events: &mut Vec<EngineEvent>) -> Handled {
    if state.phase != Phase::Reveal {
        return Err(illegal());
    }
    if should_game_end(state) {
        state.phase = Phase::GameOver;
        let result = build_game_result(state);
        state.game_result = Some(result.clone());
        add_log(state, LogEntry::new(state.round, LogKind::GameEnd));
        emit(events, EngineEvent { game_result: Some(result), ..ev(EventType::GameEnd) });
        return Ok(());
    }
    start_next_round(state, ctx)
}

fn handle_kaboo(
    state: &mut GameState,
    player_id: &str,
    counter: bool,
    ctx: &mut Ctx,
    events: &mut Vec<EngineEvent>,
) -> Handled {
    if state.phase != Phase::TurnStart {
        return Err(illegal());
    }
    require_turn(state, player_id)?;
    blocked_by_tap(state)?;
    let caller_live = state.kaboo_caller_id.is_some();
    let counter_rule = state.rules.counter_kaboo;
    let round = state.round;
    let p = get_player_mut(state, player_id).ok_or_else(illegal)?;

    if !counter {
        if caller_live || p.called_kaboo {
            return Err(illegal());
        }
        p.called_kaboo = true;
        add_log(state, LogEntry::new(round, LogKind::Kaboo).actor(player_id));
        emit(events, EngineEvent { by: Some(player_id.into()), ..ev(EventType::Kaboo) });
        begin_final_turns(state, player_id);
        // Calling ends your turn — the caller does not draw.
        advance_turn(state, ctx, events);
        return Ok(());
    }

    // Counter-KABOO: the minimal reading. Skip your turn and try to beat the
    // caller's total. See AGENTS.md §6.
    if !counter_rule || !caller_live || p.counter_kaboo {
        return Err(illegal());
    }
    p.counter_kaboo = true;
    add_log(state, LogEntry::new(round, LogKind::CounterKaboo).actor(player_id));
    emit(events, EngineEvent { by: Some(player_id.into()), ..ev(EventType::CounterKaboo) });
    advance_turn(state, ctx, events);
    Ok(())
}

fn handle_draw(
    state: &mut GameState,
    player_id: &str,
    from: DrawSource,
    ctx: &mut Ctx,
    events: &mut Vec<EngineEvent>,
) -> Handled {
    if state.phase != Phase::TurnStart {
        return Err(illegal());
    }
    require_turn(state, player_id)?;
    blocked_by_tap(state)?;
    let round = state.round;

    if from == DrawSource::Discard {
        let Some(top) = discard_top(state) else { return Err(illegal()) };
        state.discard.pop();
        // The top changed, so anyone who claimed a tap against the old top is stale.
        state.discard_seq += 1;
        state.drawn = Some(Drawn { card_id: top, from: DrawSource::Discard, by: player_id.into() });
        state.phase = Phase::Drawn;
        state.turn_deadline = ctx.now + state.rules.turn_deadline_ms;
        add_log(state, LogEntry::new(round, LogKind::TookDiscard).actor(player_id));
        emit(
            events,
            EngineEvent { by: Some(player_id.into()), card: Some(public_card(top)), ..ev(EventType::TookDiscard) },
        );
        return Ok(());
    }

    let Some(id) = pop_deck(state) else { return Err(illegal()) };

    // HOUSE RULE — DECK OUT: drawing the last card ends the round, and the
    // drawer gets one swap-or-decline decision. No discard, no effect, no tapping.
    if draw_pile_empty(state) {
        state.held_card_id = Some(id);
        state.phase = Phase::FinalSwap;
        state.turn_deadline = 0;
        state.final_swap_deadline = ctx.now + state.rules.final_swap_deadline_ms;
        add_log(state, LogEntry::new(round, LogKind::DeckOut).actor(player_id));
        emit(
            events,
            EngineEvent { by: Some(player_id.into()), reason: Some("lastCard".into()), ..ev(EventType::DeckOut) },
        );
        emit_private(
            events,
            player_id,
            EngineEvent { by: Some(player_id.into()), card: Some(public_card(id)), ..ev(EventType::DeckOut) },
        );
        return Ok(());
    }

    state.drawn = Some(Drawn { card_id: id, from: DrawSource::Deck, by: player_id.into() });
    state.phase = Phase::Drawn;
    state.turn_deadline = ctx.now + state.rules.turn_deadline_ms;
    add_log(state, LogEntry::new(round, LogKind::Drew).actor(player_id));
    // The drawn card is the drawer's private look — nobody else may see it.
    emit_private(
        events,
        player_id,
        EngineEvent {
            by: Some(player_id.into()),
            from: Some(DrawSource::Deck),
            card: Some(public_card(id)),
            ..ev(EventType::Drew)
        },
    );
    emit(
        events,
        EngineEvent {
            by: Some(player_id.into()),
            from: Some(DrawSource::Deck),
            hidden: Some(true),
            ..ev(EventType::Drew)
        },
    );
    Ok(())
}

fn handle_swap(
    state: &mut GameState,
    player_id: &str,
    slot: usize,
    ctx: &mut Ctx,
    events: &mut Vec<EngineEvent>,
) -> Handled {
    if state.phase != Phase::Drawn {
        return Err(illegal());
    }
    require_turn(state, player_id)?;
    blocked_by_tap(state)?;

    let (Some(d), Some(p)) = (state.drawn.clone(), get_player(state, player_id)) else {
        return Err(illegal());
    };
    if slot >= p.hand.len() {
        return Err(illegal());
    }

    let replaced = p.hand[slot];
    set_slot(state, player_id, slot, d.card_id);
    if let Some(r) = replaced {
        push_discard(state, r);
    }
    state.drawn = None;

    add_log(state, LogEntry::new(state.round, LogKind::Swapped).actor(player_id));
    emit(events, EngineEvent { by: Some(player_id.into()), slot: Some(slot), ..ev(EventType::Swapped) });
    advance_turn(state, ctx, events);
    Ok(())
}

fn handle_discard(state: &mut GameState, player_id: &str, ctx: &mut Ctx, events: &mut Vec<EngineEvent>) -> Handled {
    if state.phase != Phase::Drawn {
        return Err(illegal());
    }
    require_turn(state, player_id)?;
    blocked_by_tap(state)?;
    let Some(d) = state.drawn.take() else { return Err(illegal()) };

    let c = card(d.card_id);
    push_discard(state, d.card_id);

    add_log(state, LogEntry::new(state.round, LogKind::Discarded).actor(player_id));
    emit(
        events,
        EngineEvent { by: Some(player_id.into()), card: Some(public_card(d.card_id)), ..ev(EventType::Discarded) },
    );

    // Only a discarded card triggers an effect — never a swap, never a tap.
    if effect_for(c).is_some() {
        state.effect =
            Some(EffectState { by: player_id.into(), rank: c.rank, a: None, b: None, awaiting_king_decision: false });
        state.phase = Phase::Effect;
        state.turn_deadline = ctx.now + state.rules.turn_deadline_ms;
        return Ok(());
    }

    advance_turn(state, ctx, events);
    Ok(())
}

fn handle_final_swap(
    state: &mut GameState,
    player_id: &str,
    slot: Option<usize>,
    ctx: &mut Ctx,
    events: &mut Vec<EngineEvent>,
) -> Handled {
    if state.phase != Phase::FinalSwap {
        return Err(illegal());
    }
    require_turn(state, player_id)?;

    let (Some(held), Some(p)) = (state.held_card_id, get_player(state, player_id)) else {
        return Err(illegal());
    };
    let round = state.round;

    match slot {
        Some(slot) => {
            if slot >= p.hand.len() {
                return Err(illegal());
            }
            let replaced = p.hand[slot];
            set_slot(state, player_id, slot, held);
            state.held_card_id = None;
            // The replaced card still has to go somewhere, or a card id would
            // vanish from the accounting. It lands on the discard, with no effect.
            if let Some(r) = replaced {
                push_discard(state, r);
            }
            add_log(state, LogEntry::new(round, LogKind::FinalSwap).actor(player_id).text("swapped"));
            emit(events, EngineEvent { by: Some(player_id.into()), slot: Some(slot), ..ev(EventType::FinalSwap) });
        }
        None => {
            state.set_aside = Some(held);
            state.held_card_id = None;
            add_log(state, LogEntry::new(round, LogKind::FinalDecline).actor(player_id));
            emit(events, EngineEvent { by: Some(player_id.into()), ..ev(EventType::FinalDecline) });
        }
    }

    reveal_round(state, ctx, events);
    Ok(())
}

fn handle_leave(state: &mut GameState, player_id: &str, events: &mut Vec<EngineEvent>) -> Handled {
    get_player_mut(state, player_id).ok_or_else(|| err(EngineErrorCode::UnknownPlayer))?.connected = false;
    add_log(state, LogEntry::new(state.round, LogKind::PlayerLeft).actor(player_id));
    emit(events, EngineEvent { left_id: Some(player_id.into()), ..ev(EventType::PhaseChanged) });
    Ok(())
}

/// The single place that maps an action to its handler.
fn dispatch(state: &mut GameState, action: &Action, ctx: &mut Ctx, events: &mut Vec<EngineEvent>) -> Handled {
    match action {
        Action::Peek { player_id, slot } => handle_peek(state, player_id, *slot, events),
        Action::Ready { player_id } => handle_ready(state, player_id, ctx, events),
        Action::Continue { .. } => handle_continue(state, ctx, events),
        Action::CallKaboo { player_id } => handle_kaboo(state, player_id, false, ctx, events),
        Action::CounterKaboo { player_id } => handle_kaboo(state, player_id, true, ctx, events),
        Action::Draw { player_id, from } => handle_draw(state, player_id, *from, ctx, events),
        Action::Swap { player_id, slot } => handle_swap(state, player_id, *slot, ctx, events),
        Action::Discard { player_id } => handle_discard(state, player_id, ctx, events),
        Action::FinalSwap { player_id, slot } => handle_final_swap(state, player_id, Some(*slot), ctx, events),
        Action::FinalDecline { player_id } => handle_final_swap(state, player_id, None, ctx, events),
        Action::EffectSkip { .. }
        | Action::EffectPeekOwn { .. }
        | Action::EffectPeekOther { .. }
        | Action::EffectBlindSwap { .. }
        | Action::EffectQueenSwap { .. }
        | Action::EffectKingPeek { .. }
        | Action::EffectKingDecide { .. } => {
            if state.phase != Phase::Effect {
                return Err(illegal());
            }
            resolve_effect(state, action, ctx, events)
        }
        Action::Tap { .. } | Action::TapChoose { .. } | Action::TapPlace { .. } | Action::TapDeclinePlace { .. } => {
            handle_tap_action(state, action, ctx, events)
        }
        Action::Leave { player_id } => handle_leave(state, player_id, events),
    }
}

/// Resolve any expired deadlines without applying an action. `None` when
/// nothing happened.
pub fn tick(state: &GameState, ctx: &mut Ctx) -> Option<ApplyResult> {
    let mut next = state.clone();
    let mut events = Vec::new();
    run_deadlines(&mut next, ctx, &mut events);
    if events.is_empty() {
        return None;
    }
    next.version = state.version + 1;
    next.last_activity_at = ctx.now;
    Some(ApplyResult { state: next, events, error: None, version_changed: true })
}

/// The engine's public entry point.
///
/// Pure and total: it clones, never mutates its input, and never panics for a
/// bad action — it reports the refusal in `error`.
pub fn apply_action(state: &GameState, action: &Action, ctx: &mut Ctx) -> ApplyResult {
    let unchanged =
        |code| ApplyResult { state: state.clone(), events: Vec::new(), error: Some(err(code)), version_changed: false };
    if get_player(state, action.player_id()).is_none() {
        return unchanged(EngineErrorCode::UnknownPlayer);
    }
    if state.phase == Phase::GameOver {
        return unchanged(EngineErrorCode::GameOver);
    }

    let mut next = state.clone();
    let mut events = Vec::new();

    // Deadlines resolve first, so a player's action always meets the table as
    // it really is, not as it was when they looked.
    run_deadlines(&mut next, ctx, &mut events);
    let deadline_moved = !events.is_empty();

    match dispatch(&mut next, action, ctx, &mut events) {
        Ok(()) => {
            next.version = state.version + 1;
            next.last_activity_at = ctx.now;
            ApplyResult { state: next, events, error: None, version_changed: true }
        }
        Err(rejection) if !deadline_moved => {
            ApplyResult { state: state.clone(), events: Vec::new(), error: Some(rejection), version_changed: false }
        }
        Err(rejection) => {
            // Time moved on even though the action was refused — hand back the
            // movement alongside the refusal so a timeout is never dropped.
            next.version = state.version + 1;
            next.last_activity_at = ctx.now;
            ApplyResult { state: next, events, error: Some(rejection), version_changed: true }
        }
    }
}

/// Start a fresh game in a room. Round 1's dealer is seat 0.
pub fn start_game(room_code: &str, players: &[Seat], ctx: &mut Ctx, rules: Option<RuleConfig>) -> GameState {
    create_game(room_code, players, rules.unwrap_or_default(), 0, ctx)
}
