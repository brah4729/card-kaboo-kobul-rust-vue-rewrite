//! Game construction, the deal, and the small mutation helpers every handler
//! shares. Every slot mutation bumps `gen` so the UI can animate a change
//! without ever being told which card arrived (AGENTS.md §7).

use crate::cards::{is_empty_hand, make_deck, shuffle};
use crate::errors::{Handled, illegal};
use crate::types::{CardId, CardRef, Ctx, GameState, LogEntry, LogKind, Phase, PlayerState, RuleConfig, Seat, Slot};

pub const MIN_PLAYERS: usize = 2;
pub const MAX_PLAYERS: usize = 8;

/// The two slots a player may look at once, at the start of a round.
///
/// Slots 0,1 are the far row and 2,3 the near row of the physical 2×2 square.
/// The app draws the four in one row, so these read as cards 3 and 4.
pub const PEEK_SLOTS: [usize; 2] = [2, 3];

/// The log is for the UI; capped so a long game cannot bloat the stored record.
const LOG_CAP: usize = 200;

pub fn player_index(state: &GameState, id: &str) -> Option<usize> {
    state.players.iter().position(|p| p.id == id)
}

pub fn get_player<'a>(state: &'a GameState, id: &str) -> Option<&'a PlayerState> {
    state.players.iter().find(|p| p.id == id)
}

pub fn get_player_mut<'a>(state: &'a mut GameState, id: &str) -> Option<&'a mut PlayerState> {
    state.players.iter_mut().find(|p| p.id == id)
}

/// The player whose turn it is, by id.
pub fn turn_player_id(state: &GameState) -> Option<String> {
    state.players.get(state.turn_seat).map(|p| p.id.clone())
}

/// The ring index of the seat after `seat`.
pub fn next_seat(seat: usize, count: usize) -> usize {
    (seat + 1) % count
}

/// Advance to the next seat that is still owed a turn.
///
/// When a KABOO is live, only players in `final_turns_remaining` are eligible,
/// so the caller's own seat is skipped and nobody gets a second final turn.
pub fn next_eligible_seat(state: &GameState, from: usize) -> usize {
    let count = state.players.len();
    if state.kaboo_caller_id.is_none() {
        return next_seat(from, count);
    }
    for step in 1..=count {
        let seat = (from + step) % count;
        if state.final_turns_remaining.contains(&state.players[seat].id) {
            return seat;
        }
    }
    // Nobody left to play — the caller's turn resolution ends the round.
    from
}

pub fn new_player(id: &str, name: &str, seat: usize, hand_size: usize) -> PlayerState {
    PlayerState {
        id: id.to_string(),
        name: name.to_string(),
        seat,
        hand: vec![None; hand_size],
        generation: vec![0; hand_size],
        score: 0,
        connected: true,
        ready: false,
        peeked: vec![false; hand_size],
        called_kaboo: false,
        counter_kaboo: false,
        final_turn_done: false,
    }
}

fn player_mut_or_panic<'a>(state: &'a mut GameState, id: &str) -> &'a mut PlayerState {
    get_player_mut(state, id).unwrap_or_else(|| panic!("kaboo: unknown player {id}"))
}

/// Put a card in a slot. Out-of-range is an engine bug: handlers check first.
pub fn set_slot(state: &mut GameState, player_id: &str, index: usize, card_id: CardId) {
    let p = player_mut_or_panic(state, player_id);
    assert!(index < p.hand.len(), "kaboo: slot {index} out of range");
    p.hand[index] = Some(card_id);
    p.generation[index] += 1;
}

pub fn clear_slot(state: &mut GameState, player_id: &str, index: usize) {
    let p = player_mut_or_panic(state, player_id);
    assert!(index < p.hand.len(), "kaboo: slot {index} out of range");
    p.hand[index] = None;
    p.generation[index] += 1;
}

/// Penalty cards land in a brand-new slot; hands may grow past the dealt size.
pub fn append_slot(state: &mut GameState, player_id: &str, card_id: CardId) -> usize {
    let p = player_mut_or_panic(state, player_id);
    p.hand.push(Some(card_id));
    p.generation.push(0);
    p.peeked.push(false);
    p.hand.len() - 1
}

pub fn slot_card(state: &GameState, r: &CardRef) -> Slot {
    get_player(state, &r.player_id).and_then(|p| p.hand.get(r.slot).copied().flatten())
}

pub fn is_slot_occupied(state: &GameState, r: &CardRef) -> bool {
    slot_card(state, r).is_some()
}

// ---------------------------------------------------------------------------
// Piles
// ---------------------------------------------------------------------------

pub fn discard_top(state: &GameState) -> Option<CardId> {
    state.discard.last().copied()
}

pub fn push_discard(state: &mut GameState, card_id: CardId) {
    state.discard.push(card_id);
    state.discard_seq += 1;
}

/// Removes and returns the top of the draw pile (index 0).
pub fn pop_deck(state: &mut GameState) -> Option<CardId> {
    if state.deck.is_empty() { None } else { Some(state.deck.remove(0)) }
}

pub fn draw_pile_empty(state: &GameState) -> bool {
    state.deck.is_empty()
}

pub fn add_log(state: &mut GameState, entry: LogEntry) {
    state.log.push(entry);
    if state.log.len() > LOG_CAP {
        let excess = state.log.len() - LOG_CAP;
        state.log.drain(..excess);
    }
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

fn fresh_round_fields(state: &mut GameState) {
    state.deck.clear();
    state.discard.clear();
    state.discard_seq = 0;
    state.set_aside = None;
    state.drawn = None;
    state.held_card_id = None;
    state.tap = None;
    state.effect = None;
    state.kaboo_caller_id = None;
    state.final_turns_remaining.clear();
    state.final_swap_deadline = 0;
    state.turn_deadline = 0;
    state.round_result = None;
    state.turn_count = 0;
}

/// Deal a round: shuffle, hand out `hand_size` cards per player, and open the
/// peek phase.
///
/// NOTE: the discard pile starts EMPTY — the brief says the remaining cards
/// form the draw pile, so nothing is flipped to start. The first player
/// therefore cannot take from the discard. See AGENTS.md §7.
pub fn start_round(state: &mut GameState, ctx: &mut Ctx) {
    let hand_size = state.rules.hand_size;
    fresh_round_fields(state);
    state.round += 1;
    state.phase = Phase::Peek;

    state.deck = shuffle(&make_deck(&state.rules), ctx.rng);

    for p in &mut state.players {
        p.hand = vec![None; hand_size];
        p.generation = vec![0; hand_size];
        p.ready = false;
        p.peeked = vec![false; hand_size];
        p.called_kaboo = false;
        p.counter_kaboo = false;
        p.final_turn_done = false;
    }

    // Deal one card at a time around the table, slot by slot — the way it is
    // dealt by hand. Reproducible given a seeded rng.
    let ids: Vec<String> = state.players.iter().map(|p| p.id.clone()).collect();
    for slot in 0..hand_size {
        for id in &ids {
            let card_id = pop_deck(state).expect("kaboo: deck exhausted while dealing");
            set_slot(state, id, slot, card_id);
        }
    }

    state.turn_seat = next_seat(state.dealer_seat, state.players.len());
    state.peek_deadline = ctx.now + state.rules.peek_deadline_ms;
    state.turn_deadline = 0;
    state.last_activity_at = ctx.now;

    let round = state.round;
    add_log(state, LogEntry::new(round, LogKind::RoundStart));
}

/// Build a game and deal round 1. Panics on a roster outside 2–8: the room
/// layer validates the roster before it ever gets here.
pub fn create_game(
    room_code: &str,
    players: &[Seat],
    rules: RuleConfig,
    dealer_seat: usize,
    ctx: &mut Ctx,
) -> GameState {
    assert!(players.len() >= MIN_PLAYERS, "kaboo: need at least 2 players");
    assert!(players.len() <= MAX_PLAYERS, "kaboo: at most 8 players");

    let built: Vec<PlayerState> =
        players.iter().enumerate().map(|(i, p)| new_player(&p.id, &p.name, i, rules.hand_size)).collect();

    let mut state = GameState {
        room_code: room_code.to_string(),
        version: 0,
        // `start_round` below immediately sets this to Peek.
        phase: Phase::Peek,
        round: 0,
        rules,
        seat_order: built.iter().map(|p| p.id.clone()).collect(),
        players: built,
        dealer_seat,
        turn_seat: 0,
        turn_count: 0,
        deck: Vec::new(),
        discard: Vec::new(),
        discard_seq: 0,
        set_aside: None,
        drawn: None,
        held_card_id: None,
        tap: None,
        effect: None,
        kaboo_caller_id: None,
        final_turns_remaining: Vec::new(),
        peek_deadline: 0,
        final_swap_deadline: 0,
        turn_deadline: 0,
        round_result: None,
        game_result: None,
        log: Vec::new(),
        started_at: ctx.now,
        last_activity_at: ctx.now,
    };

    start_round(&mut state, ctx);
    state
}

/// Begin the next round: advance the dealer, then deal.
pub fn start_next_round(state: &mut GameState, ctx: &mut Ctx) -> Handled {
    if state.phase != Phase::Reveal {
        return Err(illegal());
    }
    state.dealer_seat = next_seat(state.dealer_seat, state.players.len());
    start_round(state, ctx);
    Ok(())
}

/// Has any player run out of cards? That player auto-KABOOs (house rule).
pub fn find_empty_hand_player(state: &GameState) -> Option<&PlayerState> {
    state.players.iter().find(|p| is_empty_hand(&p.hand))
}
