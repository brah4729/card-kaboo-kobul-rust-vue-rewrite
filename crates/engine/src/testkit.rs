//! Test support, shared by the engine's own suites and the room's. Compiled
//! only with the `testkit` feature, so none of it reaches the Worker.
//!
//! * [`mulberry32`] — the same generator the TypeScript suites used, bit for
//!   bit, so a seed means the same deal it always did.
//! * [`Table`] — a game plus a clock, with `ok`/`fail` assertions.
//! * [`arrange`] / [`arrange_short_deck`] — lay a table out deterministically.
//! * [`leaked_card_ids`] / [`accounting_errors`] — the two invariants.
//! * [`random_action`] — the deliberately sloppy fuzzer.
//! * [`choose_move`] — what a client would do next, from a `Projection` ALONE.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::EngineEvent;
use crate::cards::{DECK_SIZE, RANKS, SUITS, card};
use crate::project::{Projection, project};
use crate::reduce::{ApplyResult, apply_action, tick};
use crate::state::create_game;
use crate::types::{
    Action, ActionType, CardId, CardRef, Ctx, DrawSource, GameState, Millis, Phase, PlayerId, Rank, RuleConfig, Seat,
    Slot, Suit,
};

pub const T0: Millis = 1_700_000_000_000;

/// mulberry32 — small, fast, fully deterministic. A faithful port of the TS
/// version (32-bit wrapping arithmetic, `>>>` as a logical shift).
pub fn mulberry32(seed: u32) -> impl FnMut() -> f64 {
    let mut a = seed;
    move || {
        a = a.wrapping_add(0x6d2b_79f5);
        let mut t = (a ^ (a >> 15)).wrapping_mul(1 | a);
        t = (t.wrapping_add((t ^ (t >> 7)).wrapping_mul(61 | t))) ^ t;
        f64::from(t ^ (t >> 14)) / 4_294_967_296.0
    }
}

/// Card id for a rank + suit, e.g. `cid(Rank::Seven, Suit::H)`.
pub fn cid(rank: Rank, suit: Suit) -> CardId {
    let s = SUITS.iter().position(|x| *x == suit).unwrap();
    let r = RANKS.iter().position(|x| *x == rank).unwrap();
    (s * RANKS.len() + r) as CardId
}

/// The wire name of a rank: "7", "10", "K", "JOKER".
pub fn rank_str(rank: Rank) -> String {
    serde_json::to_value(rank).unwrap().as_str().unwrap().to_string()
}

pub fn card_label(id: Slot) -> String {
    match id {
        None => "--".into(),
        Some(id) => {
            let c = card(id);
            match c.suit {
                None => "JOKER".into(),
                Some(s) => format!("{}{:?}", rank_str(c.rank), s),
            }
        }
    }
}

/// Terse action constructors, so a test reads like the move it makes.
pub mod act {
    use crate::types::{Action, CardRef, DrawSource};

    pub fn r(player: &str, slot: usize) -> CardRef {
        CardRef { player_id: player.into(), slot }
    }
    pub fn peek(p: &str, slot: usize) -> Action {
        Action::Peek { player_id: p.into(), slot }
    }
    pub fn ready(p: &str) -> Action {
        Action::Ready { player_id: p.into() }
    }
    pub fn cont(p: &str) -> Action {
        Action::Continue { player_id: p.into() }
    }
    pub fn kaboo(p: &str) -> Action {
        Action::CallKaboo { player_id: p.into() }
    }
    pub fn counter(p: &str) -> Action {
        Action::CounterKaboo { player_id: p.into() }
    }
    pub fn draw(p: &str) -> Action {
        Action::Draw { player_id: p.into(), from: DrawSource::Deck }
    }
    pub fn take(p: &str) -> Action {
        Action::Draw { player_id: p.into(), from: DrawSource::Discard }
    }
    pub fn swap(p: &str, slot: usize) -> Action {
        Action::Swap { player_id: p.into(), slot }
    }
    pub fn discard(p: &str) -> Action {
        Action::Discard { player_id: p.into() }
    }
    pub fn final_swap(p: &str, slot: usize) -> Action {
        Action::FinalSwap { player_id: p.into(), slot }
    }
    pub fn final_decline(p: &str) -> Action {
        Action::FinalDecline { player_id: p.into() }
    }
    pub fn skip(p: &str) -> Action {
        Action::EffectSkip { player_id: p.into() }
    }
    pub fn peek_own(p: &str, slot: usize) -> Action {
        Action::EffectPeekOwn { player_id: p.into(), slot }
    }
    pub fn peek_other(p: &str, target: &str, slot: usize) -> Action {
        Action::EffectPeekOther { player_id: p.into(), target_player_id: target.into(), slot }
    }
    pub fn blind_swap(p: &str, a: CardRef, b: CardRef) -> Action {
        Action::EffectBlindSwap { player_id: p.into(), a, b }
    }
    pub fn queen_swap(p: &str, slot: usize, target: CardRef) -> Action {
        Action::EffectQueenSwap { player_id: p.into(), slot, target }
    }
    pub fn king_peek(p: &str, a: CardRef, b: CardRef) -> Action {
        Action::EffectKingPeek { player_id: p.into(), a, b }
    }
    pub fn king_decide(p: &str, swap: bool) -> Action {
        Action::EffectKingDecide { player_id: p.into(), swap }
    }
    pub fn tap(p: &str, seen_seq: u32) -> Action {
        Action::Tap { player_id: p.into(), seen_seq }
    }
    pub fn tap_choose(p: &str, target: CardRef) -> Action {
        Action::TapChoose { player_id: p.into(), target }
    }
    pub fn tap_place(p: &str, slot: usize) -> Action {
        Action::TapPlace { player_id: p.into(), slot }
    }
    pub fn tap_decline(p: &str) -> Action {
        Action::TapDeclinePlace { player_id: p.into() }
    }
    pub fn leave(p: &str) -> Action {
        Action::Leave { player_id: p.into() }
    }
}

/// Events delivered privately to `player`.
pub fn private_to<'a>(events: &'a [EngineEvent], player: &str) -> Vec<&'a EngineEvent> {
    events.iter().filter(|e| e.to.as_deref() == Some(player)).collect()
}

/// Events with no recipient.
pub fn public_events(events: &[EngineEvent]) -> Vec<&EngineEvent> {
    events.iter().filter(|e| e.to.is_none()).collect()
}

pub fn ids(n: usize) -> Vec<PlayerId> {
    (1..=n).map(|i| format!("p{i}")).collect()
}

pub fn seats(n: usize) -> Vec<Seat> {
    (1..=n).map(|i| Seat { id: format!("p{i}"), name: format!("P{i}") }).collect()
}

/// A game, a clock and a seeded generator, with assertion helpers.
pub struct Table {
    pub state: GameState,
    pub ids: Vec<PlayerId>,
    pub seed: u32,
    pub events: Vec<EngineEvent>,
    pub now: Millis,
    rng: Box<dyn FnMut() -> f64>,
}

#[derive(Debug, Clone, Default)]
pub struct TableOptions {
    pub players: Option<usize>,
    pub seed: Option<u32>,
    pub rules: Option<RuleConfig>,
}

impl Table {
    pub fn new(opts: TableOptions) -> Self {
        let n = opts.players.unwrap_or(2);
        let seed = opts.seed.unwrap_or(7);
        let mut rng: Box<dyn FnMut() -> f64> = Box::new(mulberry32(seed));
        let now = T0;
        let state = {
            let mut ctx = Ctx { now, rng: &mut *rng };
            create_game("TEST", &seats(n), opts.rules.unwrap_or_default(), 0, &mut ctx)
        };
        Table { state, ids: ids(n), seed, events: Vec::new(), now, rng }
    }

    pub fn players(n: usize) -> Self {
        Self::new(TableOptions { players: Some(n), ..Default::default() })
    }

    pub fn with_rules(n: usize, rules: RuleConfig) -> Self {
        Self::new(TableOptions { players: Some(n), rules: Some(rules), ..Default::default() })
    }

    pub fn advance(&mut self, ms: Millis) {
        self.now += ms;
    }

    pub fn send(&mut self, action: Action) -> ApplyResult {
        let mut ctx = Ctx { now: self.now, rng: &mut *self.rng };
        let result = apply_action(&self.state, &action, &mut ctx);
        if result.version_changed {
            self.state = result.state.clone();
            self.events.extend(result.events.iter().cloned());
        }
        result
    }

    #[track_caller]
    pub fn ok(&mut self, action: Action) -> ApplyResult {
        let kind = action.kind();
        let result = self.send(action);
        if let Some(e) = &result.error {
            panic!("expected {kind:?} to succeed, got {:?}: {}", e.code, e.message);
        }
        result
    }

    #[track_caller]
    pub fn fail(&mut self, action: Action) -> ApplyResult {
        let kind = action.kind();
        let result = self.send(action);
        assert!(result.error.is_some(), "expected {kind:?} to be refused, but it succeeded");
        result
    }

    /// Run deadlines without an action, like the room's alarm.
    pub fn tick(&mut self) -> Option<ApplyResult> {
        let mut ctx = Ctx { now: self.now, rng: &mut *self.rng };
        let result = tick(&self.state, &mut ctx);
        if let Some(r) = &result {
            self.state = r.state.clone();
            self.events.extend(r.events.iter().cloned());
        }
        result
    }

    pub fn view(&self, id: &str) -> Projection {
        project(&self.state, id)
    }

    pub fn hand_of(&self, id: &str) -> Vec<Slot> {
        self.state.players.iter().find(|p| p.id == id).expect("no such player").hand.clone()
    }

    pub fn turn_id(&self) -> PlayerId {
        self.state.players[self.state.turn_seat].id.clone()
    }

    /// Every player peeks their bottom two and declares ready.
    pub fn finish_peek(&mut self) {
        for id in self.ids.clone() {
            if self.state.phase != Phase::Peek {
                break;
            }
            self.ok(Action::Peek { player_id: id.clone(), slot: 2 });
            self.ok(Action::Peek { player_id: id.clone(), slot: 3 });
            self.ok(Action::Ready { player_id: id });
        }
    }

    pub fn total_cards(&self) -> usize {
        if self.state.rules.jokers { usize::from(DECK_SIZE) + 2 } else { usize::from(DECK_SIZE) }
    }
}

// ---------------------------------------------------------------------------
// Deterministic table arrangement
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct ArrangeSpec {
    /// Full hand for a player. `None` in a slot means an empty slot.
    pub hands: BTreeMap<PlayerId, Vec<Slot>>,
    /// Draw pile, index 0 = the next card drawn.
    pub deck: Option<Vec<CardId>>,
    /// Discard pile, last element = the top (the tap target).
    pub discard: Option<Vec<CardId>>,
    /// `Some(None)` clears it; `None` leaves it alone.
    pub set_aside: Option<Slot>,
}

impl ArrangeSpec {
    pub fn hand(mut self, id: &str, cards: &[CardId]) -> Self {
        self.hands.insert(id.into(), cards.iter().map(|c| Some(*c)).collect());
        self
    }
    pub fn hand_slots(mut self, id: &str, slots: &[Slot]) -> Self {
        self.hands.insert(id.into(), slots.to_vec());
        self
    }
    pub fn deck(mut self, cards: &[CardId]) -> Self {
        self.deck = Some(cards.to_vec());
        self
    }
    pub fn discard(mut self, cards: &[CardId]) -> Self {
        self.discard = Some(cards.to_vec());
        self
    }
}

/// Lay the table out deterministically.
///
/// * A card named in the spec is PLACED where the spec says. If it was sitting
///   somewhere else, it is pulled out of there (an untouched hand it came from
///   is left with a hole, which is a legal hand state).
/// * A zone the spec does NOT mention keeps whatever it holds, minus anything
///   the spec placed elsewhere.
/// * Every remaining card goes into the draw pile in ascending id order, so
///   `spec.deck` pins the next cards drawn and the rest stays accounted for.
///
/// Panics on duplicates and strays, so an impossible table fails loudly instead
/// of quietly duplicating a card.
#[track_caller]
pub fn arrange(table: &mut Table, spec: ArrangeSpec) {
    let total = table.total_cards();
    let state = &mut table.state;

    let mut placed: BTreeMap<CardId, &'static str> = BTreeMap::new();
    let mut place = |id: CardId, where_: &'static str| {
        assert!(usize::from(id) < total, "arrange: card {id} is not in play ({where_})");
        if let Some(prev) = placed.insert(id, where_) {
            panic!("arrange: card {id} placed twice ({prev} and {where_})");
        }
    };
    for hand in spec.hands.values() {
        for id in hand.iter().flatten() {
            place(*id, "hand");
        }
    }
    for id in spec.deck.iter().flatten() {
        place(*id, "deck");
    }
    for id in spec.discard.iter().flatten() {
        place(*id, "discard");
    }
    if let Some(Some(id)) = spec.set_aside {
        place(id, "setAside");
    }

    // Untouched zones keep what the spec did not take from them.
    let mut kept: BTreeSet<CardId> = BTreeSet::new();
    let mut keep = |id: Slot| {
        if let Some(id) = id
            && !placed.contains_key(&id)
        {
            kept.insert(id);
        }
    };
    for p in &state.players {
        if spec.hands.contains_key(&p.id) {
            continue;
        }
        for id in &p.hand {
            keep(*id);
        }
    }
    if spec.discard.is_none() {
        for id in &state.discard {
            keep(Some(*id));
        }
    }
    if spec.set_aside.is_none() {
        keep(state.set_aside);
    }

    let leftover: Vec<CardId> =
        (0..total as CardId).filter(|id| !placed.contains_key(id) && !kept.contains(id)).collect();
    assert_eq!(placed.len() + kept.len() + leftover.len(), total, "arrange: card accounting is wrong");

    for p in &mut state.players {
        if let Some(hand) = spec.hands.get(&p.id) {
            p.hand = hand.clone();
            p.generation = (0..p.hand.len()).map(|i| p.generation.get(i).copied().unwrap_or(0) + 1).collect();
            p.peeked = vec![false; p.hand.len()];
        } else {
            // Give up any card the spec placed elsewhere.
            for slot in &mut p.hand {
                if let Some(id) = slot
                    && placed.contains_key(id)
                {
                    *slot = None;
                }
            }
        }
    }

    let mut deck = spec.deck.clone().unwrap_or_default();
    deck.extend(leftover);
    state.deck = deck;
    match &spec.discard {
        Some(d) => {
            state.discard = d.clone();
            state.discard_seq += 1;
        }
        None => state.discard.retain(|id| !placed.contains_key(id)),
    }
    match spec.set_aside {
        Some(s) => state.set_aside = s,
        None => {
            if let Some(id) = state.set_aside
                && placed.contains_key(&id)
            {
                state.set_aside = None;
            }
        }
    }
}

/// Arrange a table whose draw pile holds EXACTLY `deck`, by parking every other
/// unplaced card on the bottom of the discard pile. `hands` must list every player.
#[track_caller]
pub fn arrange_short_deck(
    table: &mut Table,
    hands: BTreeMap<PlayerId, Vec<Slot>>,
    deck: &[CardId],
    discard_top: &[CardId],
) {
    let total = table.total_cards();
    for p in &table.state.players {
        assert!(hands.contains_key(&p.id), "arrange_short_deck: hands must include every player (missing {})", p.id);
    }
    let mut used: BTreeSet<CardId> = BTreeSet::new();
    for hand in hands.values() {
        used.extend(hand.iter().flatten().copied());
    }
    used.extend(deck.iter().copied());
    used.extend(discard_top.iter().copied());

    let mut discard: Vec<CardId> = (0..total as CardId).filter(|id| !used.contains(id)).collect();
    discard.extend(discard_top.iter().copied());

    arrange(table, ArrangeSpec { hands, deck: Some(deck.to_vec()), discard: Some(discard), set_aside: None });
}

pub fn hands(entries: &[(&str, &[CardId])]) -> BTreeMap<PlayerId, Vec<Slot>> {
    entries.iter().map(|(id, cards)| (id.to_string(), cards.iter().map(|c| Some(*c)).collect())).collect()
}

/// Every card id currently in play, for conservation checks.
pub fn ids_in_play(state: &GameState) -> Vec<CardId> {
    let mut out: Vec<CardId> = state.deck.iter().chain(&state.discard).copied().collect();
    for p in &state.players {
        out.extend(p.hand.iter().flatten().copied());
    }
    out.extend(state.set_aside);
    out.extend(state.held_card_id);
    if let Some(d) = &state.drawn {
        out.push(d.card_id);
    }
    out
}

// ---------------------------------------------------------------------------
// Invariants
// ---------------------------------------------------------------------------

/// Every card face a JSON value contains, found by shape rather than by key.
///
/// A `PublicCard` is the only object in a projection carrying a numeric `id`
/// together with a string `rank`, so this finds the cards a client can see
/// wherever they are in the payload — including inside the log.
pub fn visible_card_ids(value: &Value, out: &mut BTreeSet<u64>) {
    match value {
        Value::Array(items) => items.iter().for_each(|v| visible_card_ids(v, out)),
        Value::Object(obj) => {
            if let (Some(Value::Number(id)), Some(Value::String(_))) = (obj.get("id"), obj.get("rank"))
                && let Some(id) = id.as_u64()
            {
                out.insert(id);
            }
            obj.values().for_each(|v| visible_card_ids(v, out));
        }
        _ => {}
    }
}

/// Card ids this viewer must NOT be able to see, given the state right now.
pub fn hidden_card_ids(state: &GameState, viewer_id: &str) -> BTreeSet<u64> {
    let revealed = matches!(state.phase, Phase::Reveal | Phase::GameOver);
    let mut hidden = BTreeSet::new();
    // Everybody's hand — including your own, which you have to remember.
    if !revealed {
        for p in &state.players {
            hidden.extend(p.hand.iter().flatten().map(|id| u64::from(*id)));
        }
    }
    // The draw pile, always: its order is the secret the whole game rests on.
    hidden.extend(state.deck.iter().map(|id| u64::from(*id)));
    // The declined last card, until the reveal.
    if !revealed && let Some(id) = state.set_aside {
        hidden.insert(u64::from(id));
    }
    // The deck-out card, when it is sitting in front of somebody else.
    if let Some(id) = state.held_card_id
        && state.players[state.turn_seat].id != viewer_id
    {
        hidden.insert(u64::from(id));
    }
    // A card drawn from the DRAW pile, to everyone but the drawer.
    if let Some(d) = &state.drawn
        && d.by != viewer_id
        && d.from == DrawSource::Deck
    {
        hidden.insert(u64::from(d.card_id));
    }
    hidden
}

/// Card ids a viewer can see but must not: the empty vec is the invariant.
pub fn leaked_card_ids(state: &GameState, viewer_id: &str) -> Vec<u64> {
    let json = serde_json::to_value(project(state, viewer_id)).unwrap();
    let mut visible = BTreeSet::new();
    visible_card_ids(&json, &mut visible);
    let hidden = hidden_card_ids(state, viewer_id);
    visible.intersection(&hidden).copied().collect()
}

/// Cards that are lost or in two places at once. The empty vec is the invariant.
pub fn accounting_errors(state: &GameState, total: usize) -> Vec<String> {
    let mut zones: BTreeMap<CardId, Vec<String>> = BTreeMap::new();
    let mut add = |id: CardId, zone: String| zones.entry(id).or_default().push(zone);
    for (i, id) in state.deck.iter().enumerate() {
        add(*id, format!("deck[{i}]"));
    }
    for (i, id) in state.discard.iter().enumerate() {
        add(*id, format!("discard[{i}]"));
    }
    for p in &state.players {
        for (i, id) in p.hand.iter().enumerate() {
            if let Some(id) = id {
                add(*id, format!("{}.hand[{i}]", p.id));
            }
        }
    }
    if let Some(id) = state.set_aside {
        add(id, "setAside".into());
    }
    if let Some(id) = state.held_card_id {
        add(id, "heldCard".into());
    }
    if let Some(d) = &state.drawn {
        add(d.card_id, "drawn".into());
    }

    let mut errors = Vec::new();
    for id in 0..total as CardId {
        match zones.get(&id) {
            None => errors.push(format!("card {id} is in no zone")),
            Some(w) if w.len() > 1 => errors.push(format!("card {id} is in {} zones: {}", w.len(), w.join(", "))),
            _ => {}
        }
    }
    for id in zones.keys() {
        if usize::from(*id) >= total {
            errors.push(format!("card {id} is not in play (deck size {total})"));
        }
    }
    errors
}

// ---------------------------------------------------------------------------
// The fuzzer
// ---------------------------------------------------------------------------

pub const MAX_STEPS: usize = 1500;

fn pick<'a, T>(rng: &mut dyn FnMut() -> f64, items: &'a [T]) -> &'a T {
    &items[((rng() * items.len() as f64).floor() as usize).min(items.len() - 1)]
}

/// A random action for the current state. Deliberately sloppy: out-of-range
/// slots, wrong actors and stale tap sequences are all in scope, because a
/// refused action must leave the invariants intact just as a successful one must.
pub fn random_action(rng: &mut dyn FnMut() -> f64, s: &GameState, ids: &[PlayerId]) -> Option<Action> {
    let turn_id = s.players.get(s.turn_seat).map_or_else(|| ids[0].clone(), |p| p.id.clone());
    // 4 is out of range for a fresh hand: on purpose.
    let slot = |rng: &mut dyn FnMut() -> f64| (rng() * 5.0).floor() as usize;
    let any = |rng: &mut dyn FnMut() -> f64| pick(rng, ids).clone();
    let card_ref = |rng: &mut dyn FnMut() -> f64| CardRef { player_id: pick(rng, ids).clone(), slot: slot(rng) };
    let actor = if rng() < 0.8 { turn_id.clone() } else { any(rng) };

    match s.phase {
        Phase::Peek => {
            return Some(if rng() < 0.6 {
                Action::Peek { player_id: any(rng), slot: *pick(rng, &[0, 1, 2, 3]) }
            } else {
                Action::Ready { player_id: any(rng) }
            });
        }
        Phase::Reveal => return Some(Action::Continue { player_id: any(rng) }),
        Phase::GameOver => return None,
        _ => {}
    }

    if let Some(tap) = &s.tap {
        let tapper = tap.player_id.clone();
        if tap.awaiting_placement {
            return Some(if rng() < 0.5 {
                Action::TapPlace { player_id: tapper, slot: slot(rng) }
            } else {
                Action::TapDeclinePlace { player_id: tapper }
            });
        }
        return Some(Action::TapChoose { player_id: tapper, target: card_ref(rng) });
    }

    // Tapping is open to everyone, out of turn, whenever the discard can be matched.
    if rng() < 0.15 {
        let player_id = any(rng);
        let seen_seq = if rng() < 0.8 { s.discard_seq } else { s.discard_seq.wrapping_sub(1) };
        return Some(Action::Tap { player_id, seen_seq });
    }

    Some(match s.phase {
        Phase::TurnStart => {
            let roll = rng();
            if roll < 0.1 {
                Action::CallKaboo { player_id: actor }
            } else if roll < 0.16 {
                Action::CounterKaboo { player_id: actor }
            } else {
                let from = if rng() < 0.8 { DrawSource::Deck } else { DrawSource::Discard };
                Action::Draw { player_id: actor, from }
            }
        }
        Phase::Drawn => {
            if rng() < 0.7 {
                Action::Swap { player_id: actor, slot: slot(rng) }
            } else {
                Action::Discard { player_id: actor }
            }
        }
        Phase::Effect => {
            let by = s.effect.as_ref().map_or(turn_id.clone(), |e| e.by.clone());
            let rank = s.effect.as_ref().map_or(Rank::Seven, |e| e.rank);
            if s.effect.as_ref().is_some_and(|e| e.awaiting_king_decision) {
                return Some(Action::EffectKingDecide { player_id: by, swap: rng() < 0.5 });
            }
            if rng() < 0.12 {
                return Some(Action::EffectSkip { player_id: by });
            }
            match rank {
                Rank::Seven | Rank::Eight => Action::EffectPeekOwn { player_id: by, slot: slot(rng) },
                Rank::Nine | Rank::Ten => {
                    let target_player_id = any(rng);
                    Action::EffectPeekOther { player_id: by, target_player_id, slot: slot(rng) }
                }
                Rank::J => {
                    let a = card_ref(rng);
                    Action::EffectBlindSwap { player_id: by, a, b: card_ref(rng) }
                }
                Rank::Q => {
                    let own = slot(rng);
                    Action::EffectQueenSwap { player_id: by, slot: own, target: card_ref(rng) }
                }
                _ => {
                    let a = card_ref(rng);
                    Action::EffectKingPeek { player_id: by, a, b: card_ref(rng) }
                }
            }
        }
        Phase::FinalSwap => {
            if rng() < 0.6 {
                Action::FinalSwap { player_id: turn_id, slot: slot(rng) }
            } else {
                Action::FinalDecline { player_id: turn_id }
            }
        }
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// A client's decision function
// ---------------------------------------------------------------------------
//
// Shared by every test that plays a game, so the engine's client-contract suite
// and the room's transport suite drive their games through ONE definition of
// "what a client would do next". The rule this exists to enforce: nothing here
// may read `GameState`. Every argument is a `Projection` — the same bytes a
// phone receives — so if a move can be chosen here, the real UI can choose it
// too. A projection that is safe but incomplete shows up as a `None` return
// and a stalled game.

/// The occupied slots of a player, from the projection alone.
pub fn slots_of(view: &Projection, id: &str) -> Vec<usize> {
    view.players
        .iter()
        .find(|p| p.id == id)
        .map(|p| p.hand.iter().filter(|s| s.occupied).map(|s| s.index).collect())
        .unwrap_or_default()
}

pub fn view_ids(view: &Projection) -> Vec<PlayerId> {
    view.players.iter().map(|p| p.id.clone()).collect()
}

/// One legal move for `player_id`, derived only from what that player can see.
pub fn choose_move(view: &Projection, player_id: &str) -> Option<Action> {
    let legal = |a: ActionType| view.legal_actions.contains(&a);
    let me = player_id.to_string();
    let mine = slots_of(view, player_id);
    let others: Vec<PlayerId> = view_ids(view).into_iter().filter(|id| id != player_id).collect();
    let first_other_slot = || others.first().and_then(|o| slots_of(view, o).first().map(|s| (o.clone(), *s)));

    // --- peek -------------------------------------------------------------
    if view.phase == Phase::Peek {
        // `peekable_slots` is the whole reason this is possible: the UI cannot
        // derive "the bottom two, once each" from `phase`, and must not try.
        if legal(ActionType::Peek)
            && let Some(slot) = view.peekable_slots.first()
        {
            return Some(Action::Peek { player_id: me, slot: *slot });
        }
        if legal(ActionType::Ready) {
            return Some(Action::Ready { player_id: me });
        }
        return None;
    }

    // --- an in-flight tap I own -------------------------------------------
    if let Some(tap) = view.tap.as_ref().filter(|t| t.player_id == player_id) {
        if tap.awaiting_placement {
            return Some(match mine.first() {
                Some(own) => Action::TapPlace { player_id: me, slot: *own },
                None => Action::TapDeclinePlace { player_id: me },
            });
        }
        // Name a card. The UI can see everyone's slot layout and the public discard.
        for id in view_ids(view) {
            if let Some(slot) = slots_of(view, &id).first() {
                return Some(Action::TapChoose { player_id: me, target: CardRef { player_id: id, slot: *slot } });
            }
        }
        return None;
    }

    // --- deck-out ---------------------------------------------------------
    if view.phase == Phase::FinalSwap {
        if legal(ActionType::FinalSwap)
            && let Some(own) = mine.first()
        {
            return Some(Action::FinalSwap { player_id: me, slot: *own });
        }
        return Some(Action::FinalDecline { player_id: me });
    }

    // --- effects ----------------------------------------------------------
    if view.phase == Phase::Effect
        && let Some(effect) = view.effect.as_ref().filter(|e| e.by == player_id)
    {
        if effect.awaiting_king_decision {
            return Some(Action::EffectKingDecide { player_id: me, swap: false });
        }
        let chosen = match effect.rank {
            Rank::Seven | Rank::Eight => {
                mine.first().map(|slot| Action::EffectPeekOwn { player_id: me.clone(), slot: *slot })
            }
            Rank::Nine | Rank::Ten => first_other_slot().map(|(target_player_id, slot)| Action::EffectPeekOther {
                player_id: me.clone(),
                target_player_id,
                slot,
            }),
            Rank::J => match (mine.first(), first_other_slot()) {
                (Some(own), Some((other, slot))) => Some(Action::EffectBlindSwap {
                    player_id: me.clone(),
                    a: CardRef { player_id: me.clone(), slot: *own },
                    b: CardRef { player_id: other, slot },
                }),
                _ => None,
            },
            Rank::Q => match (mine.first(), first_other_slot()) {
                (Some(own), Some((other, slot))) => Some(Action::EffectQueenSwap {
                    player_id: me.clone(),
                    slot: *own,
                    target: CardRef { player_id: other, slot },
                }),
                _ => None,
            },
            Rank::K => match (mine.first(), first_other_slot()) {
                (Some(own), Some((other, slot))) => Some(Action::EffectKingPeek {
                    player_id: me.clone(),
                    a: CardRef { player_id: me.clone(), slot: *own },
                    b: CardRef { player_id: other, slot },
                }),
                _ => None,
            },
            _ => None,
        };
        return Some(chosen.unwrap_or(Action::EffectSkip { player_id: me }));
    }

    // --- the turn spine ---------------------------------------------------
    // A tap is available to ANYONE whenever the discard has a match, so a
    // client must be able to claim one without being the turn player.
    if view.can_tap && view.tap.is_none() && legal(ActionType::Tap) {
        return Some(Action::Tap { player_id: me, seen_seq: view.discard_seq });
    }

    if view.turn_player_id.as_deref() == Some(player_id) {
        if legal(ActionType::Draw) {
            let from = if view.discard_top.is_some() { DrawSource::Discard } else { DrawSource::Deck };
            return Some(Action::Draw { player_id: me, from });
        }
        if legal(ActionType::Swap)
            && let Some(slot) = mine.first()
        {
            return Some(Action::Swap { player_id: me, slot: *slot });
        }
        if legal(ActionType::Discard) {
            return Some(Action::Discard { player_id: me });
        }
    }

    if matches!(view.phase, Phase::Reveal | Phase::GameOver) && legal(ActionType::Continue) {
        return Some(Action::Continue { player_id: me });
    }
    None
}
