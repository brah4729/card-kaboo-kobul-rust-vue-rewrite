//! The canonical card table. Ids are stable and contiguous:
//!
//! ```text
//!   0–12   spades   A…K
//!   13–25  hearts   A…K   (red)
//!   26–38  diamonds A…K   (red)
//!   39–51  clubs    A…K
//!   52–53  jokers         (only dealt when `rules.jokers` is on)
//! ```
//!
//! The table is computed from the id rather than stored in `GameState`: it
//! never changes, so storing it per room would only bloat the stored record.

use crate::types::{Card, CardId, PublicCard, Rank, RuleConfig, Slot, Suit};

pub const SUITS: [Suit; 4] = [Suit::S, Suit::H, Suit::D, Suit::C];
pub const RANKS: [Rank; 13] = [
    Rank::A,
    Rank::Two,
    Rank::Three,
    Rank::Four,
    Rank::Five,
    Rank::Six,
    Rank::Seven,
    Rank::Eight,
    Rank::Nine,
    Rank::Ten,
    Rank::J,
    Rank::Q,
    Rank::K,
];

pub const DECK_SIZE: CardId = 52;
pub const JOKER_IDS: [CardId; 2] = [52, 53];
pub const MAX_CARD_ID: CardId = 53;

pub fn rank_value(rank: Rank, red: bool) -> i32 {
    match rank {
        Rank::A => 1,
        Rank::Two => 2,
        Rank::Three => 3,
        Rank::Four => 4,
        Rank::Five => 5,
        Rank::Six => 6,
        Rank::Seven => 7,
        Rank::Eight => 8,
        Rank::Nine => 9,
        Rank::Ten | Rank::J | Rank::Q => 10,
        // Both red kings are worth -1; black kings are worth the usual 10.
        Rank::K if red => -1,
        Rank::K => 10,
        Rank::Joker => 0,
    }
}

/// Look up a card. Panics on an unknown id — that is an engine bug, never a
/// user error, so it must not be swallowed into an `EngineError`.
pub fn card(id: CardId) -> Card {
    assert!(id <= MAX_CARD_ID, "kaboo: unknown card id {id}");
    if id >= DECK_SIZE {
        return Card { id, suit: None, rank: Rank::Joker, red: false, value: 0 };
    }
    let suit = SUITS[usize::from(id / 13)];
    let rank = RANKS[usize::from(id % 13)];
    let red = matches!(suit, Suit::H | Suit::D);
    Card { id, suit: Some(suit), rank, red, value: rank_value(rank, red) }
}

pub fn to_public_card(c: Card) -> PublicCard {
    PublicCard { id: c.id, rank: c.rank, suit: c.suit, red: c.red, value: c.value }
}

pub fn public_card(id: CardId) -> PublicCard {
    to_public_card(card(id))
}

/// The unshuffled deck for the given rules, top-of-pile first.
pub fn make_deck(rules: &RuleConfig) -> Vec<CardId> {
    let mut ids: Vec<CardId> = (0..DECK_SIZE).collect();
    if rules.jokers {
        ids.extend(JOKER_IDS);
    }
    ids
}

/// Fisher–Yates, driven by an injected uniform [0, 1) generator.
///
/// Never replace this with a comparator-based "random sort": that shuffle is
/// measurably biased and, worse, cannot be seeded, which would make the engine
/// untestable.
pub fn shuffle<T: Clone>(items: &[T], rng: &mut dyn FnMut() -> f64) -> Vec<T> {
    let mut out = items.to_vec();
    for i in (1..out.len()).rev() {
        // `min` guards an rng that returns exactly 1.0, which would index past the end.
        let j = ((rng() * (i as f64 + 1.0)).floor() as usize).min(i);
        out.swap(i, j);
    }
    out
}

/// A red king — the only card that scores negative, and the one card excluded from pairs.
pub fn is_red_king(c: Card) -> bool {
    c.rank == Rank::K && c.red
}

/// Sum of the card values in a hand. Empty slots contribute nothing.
pub fn hand_total(hand: &[Slot], rules: &RuleConfig) -> i32 {
    let mut total = 0;
    let mut seen: Vec<(Rank, CardId)> = Vec::new();
    let mut paired: Vec<CardId> = Vec::new();

    for id in hand.iter().flatten().copied() {
        let c = card(id);
        total += c.value;

        if !rules.matts_pairs || c.suit.is_none() {
            continue;
        }
        // "Matt's pairs": a matching pair in a hand counts 0 — red kings excepted.
        if is_red_king(c) {
            continue;
        }
        match seen.iter().find(|(rank, _)| *rank == c.rank) {
            Some(&(_, prev)) => {
                for x in [prev, id] {
                    if !paired.contains(&x) {
                        paired.push(x);
                    }
                }
            }
            None => seen.push((c.rank, id)),
        }
    }

    for id in paired {
        total -= card(id).value;
    }
    total
}

pub fn occupied_count(hand: &[Slot]) -> usize {
    hand.iter().filter(|s| s.is_some()).count()
}

pub fn is_empty_hand(hand: &[Slot]) -> bool {
    occupied_count(hand) == 0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectKind {
    OwnPeek,
    OtherPeek,
    BlindSwap,
    QueenSwap,
    KingPeek,
}

/// The effect a card triggers when discarded, if any.
pub fn effect_for(c: Card) -> Option<EffectKind> {
    match c.rank {
        Rank::Seven | Rank::Eight => Some(EffectKind::OwnPeek),
        Rank::Nine | Rank::Ten => Some(EffectKind::OtherPeek),
        Rank::J => Some(EffectKind::BlindSwap),
        Rank::Q => Some(EffectKind::QueenSwap),
        Rank::K => Some(EffectKind::KingPeek),
        _ => None,
    }
}

/// Does `candidate` match the top discard for tapping purposes?
///
/// Matching is by RANK, so any king matches any king (including red ones) when
/// `rules.red_kings_match_rank` is on. Jokers only ever match jokers.
pub fn matches_for_tap(candidate: Card, top: Card, rules: &RuleConfig) -> bool {
    if candidate.rank == Rank::Joker || top.rank == Rank::Joker {
        return candidate.rank == top.rank;
    }
    if candidate.rank != top.rank {
        return false;
    }
    if candidate.rank == Rank::K && !rules.red_kings_match_rank {
        // Fall back to colour-exact matching when the rule is off.
        return candidate.red == top.red;
    }
    true
}
