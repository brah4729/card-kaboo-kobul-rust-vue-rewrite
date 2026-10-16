//! THE HIDDEN-INFORMATION BOUNDARY.
//!
//! Everything a client is allowed to know is built here, and only here.
//!
//! Rules this file must never break:
//!  1. A card the viewer may not see is `{occupied: true, face: null}` — never a
//!     card id. Ids would leak faces, because the discard pile publishes
//!     `{id, rank}` and a client could join the two.
//!  2. `gen` changes when a slot's contents change, so the UI can animate and
//!     the player learns THAT their card was replaced (which they would see at a
//!     real table) without learning WHAT it became.
//!  3. No hand total is ever computed or exposed before `Reveal`.
//!  4. Peeks are events, not state — they appear nowhere in this file.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::cards::{occupied_count, public_card};
use crate::state::{PEEK_SLOTS, get_player};
use crate::tap::can_tap_now;
use crate::types::{
    ActionType, DrawSource, GameResult, GameState, LogEntry, Millis, Phase, PlayerId, PublicCard, Rank, RoundResult,
    RuleConfig,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SlotView {
    pub index: usize,
    pub occupied: bool,
    /// Bumped whenever this slot's contents change. Safe to animate on.
    #[serde(rename = "gen")]
    pub generation: u32,
    /// The card, only when it is genuinely public to this viewer.
    pub face: Option<PublicCard>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlayerView {
    pub id: PlayerId,
    pub name: String,
    pub seat: usize,
    pub connected: bool,
    pub hand: Vec<SlotView>,
    /// How many cards they hold. Public — you can count cards on a table.
    pub hand_count: usize,
    /// Cumulative score across rounds. Public; the round's live total is NOT.
    pub score: i32,
    pub ready: bool,
    pub called_kaboo: bool,
    pub counter_kaboo: bool,
    pub final_turn_done: bool,
    pub is_turn: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DrawnView {
    pub from: DrawSource,
    pub by: PlayerId,
    pub is_yours: bool,
    pub card: Option<PublicCard>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HeldView {
    pub is_yours: bool,
    pub card: Option<PublicCard>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SlotRef {
    pub player_id: PlayerId,
    pub slot: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TapView {
    pub player_id: PlayerId,
    #[ts(type = "number")]
    pub deadline: Millis,
    pub awaiting_placement: bool,
    pub source: Option<SlotRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EffectView {
    pub by: PlayerId,
    pub rank: Rank,
    pub awaiting_king_decision: bool,
    pub a: Option<SlotRef>,
    pub b: Option<SlotRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Deadlines {
    #[ts(type = "number")]
    pub turn: Millis,
    #[ts(type = "number")]
    pub peek: Millis,
    #[ts(type = "number")]
    pub tap: Millis,
    #[ts(type = "number")]
    pub final_swap: Millis,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Projection {
    pub room_code: String,
    pub version: u32,
    pub phase: Phase,
    pub round: u32,
    pub rounds: u32,
    pub you_id: PlayerId,
    pub players: Vec<PlayerView>,
    pub turn_seat: usize,
    pub turn_player_id: Option<PlayerId>,
    pub draw_pile_count: usize,
    pub discard_top: Option<PublicCard>,
    pub discard_count: usize,
    pub discard_recent: Vec<PublicCard>,
    /// How many times the discard top has changed.
    ///
    /// Public, and required: a `TAP` must carry the `seenSeq` it was reacting
    /// to, so a client that cannot read this cannot legitimately tap at all. It
    /// leaks nothing — anyone at the table can see how many cards are on the pile.
    pub discard_seq: u32,
    pub set_aside: Option<PublicCard>,
    pub drawn: Option<DrawnView>,
    pub held_card: Option<HeldView>,
    pub tap: Option<TapView>,
    pub effect: Option<EffectView>,
    pub kaboo_caller_id: Option<PlayerId>,
    pub final_turns_remaining: Vec<PlayerId>,
    pub deadlines: Deadlines,
    /// The slots this viewer may still look at. Empty outside `Peek`, and empty
    /// once they have looked at both — so the UI can highlight exactly the
    /// cards that are legal to tap, instead of re-deriving "the bottom two".
    pub peekable_slots: Vec<usize>,
    pub can_tap: bool,
    pub legal_actions: Vec<ActionType>,
    pub log: Vec<LogEntry>,
    pub round_result: Option<RoundResult>,
    pub game_result: Option<GameResult>,
    pub rules: RuleConfig,
    /// True once every hand is face-up.
    pub revealed: bool,
}

const RECENT_DISCARD: usize = 5;
const LOG_TAIL: usize = 60;

fn slot_ref(r: &crate::types::CardRef) -> SlotRef {
    SlotRef { player_id: r.player_id.clone(), slot: r.slot }
}

/// Which slots this viewer may still peek at, mirroring the PEEK handler exactly.
///
/// The brief's "you may look at your bottom two cards once" is a slot
/// constraint, not just a phase constraint, so the client cannot infer it from
/// `phase` alone.
pub fn peekable_slots(state: &GameState, viewer_id: &str) -> Vec<usize> {
    if state.phase != Phase::Peek {
        return Vec::new();
    }
    let Some(me) = get_player(state, viewer_id) else { return Vec::new() };
    if me.ready {
        return Vec::new();
    }
    PEEK_SLOTS
        .into_iter()
        .filter(|&slot| {
            !me.peeked.get(slot).copied().unwrap_or(false) && me.hand.get(slot).copied().flatten().is_some()
        })
        .collect()
}

/// Which actions this viewer may legally send right now.
///
/// A UI convenience only — `apply_action` remains the single authority and
/// re-checks everything. Shipping it stops the client from re-implementing (and
/// drifting from) the phase machine. It must be HONEST: everything advertised
/// here must be accepted in this state (a test holds it to that).
pub fn legal_actions(state: &GameState, viewer_id: &str) -> Vec<ActionType> {
    let mut out = Vec::new();
    let Some(me) = get_player(state, viewer_id) else { return out };

    let is_turn = state.players.get(state.turn_seat).is_some_and(|p| p.id == viewer_id);
    let tap_open = can_tap_now(state);

    match state.phase {
        Phase::Peek => {
            if !me.ready {
                if !peekable_slots(state, viewer_id).is_empty() {
                    out.push(ActionType::Peek);
                }
                out.push(ActionType::Ready);
            }
            return out;
        }
        Phase::Reveal => return vec![ActionType::Continue],
        Phase::GameOver => return out,
        _ => {}
    }

    // A claimed tap interrupts everything else until it resolves.
    if let Some(tap) = &state.tap {
        if tap.player_id == viewer_id {
            if tap.awaiting_placement {
                out.extend([ActionType::TapPlace, ActionType::TapDeclinePlace]);
            } else {
                out.push(ActionType::TapChoose);
            }
        }
        return out;
    }

    if tap_open {
        out.push(ActionType::Tap);
    }
    if !is_turn {
        return out;
    }

    match state.phase {
        Phase::TurnStart => {
            out.push(ActionType::Draw);
            // KABOO only before you act, and only once.
            if state.kaboo_caller_id.is_none() && !me.called_kaboo && !me.counter_kaboo {
                out.push(ActionType::CallKaboo);
            }
            if state.kaboo_caller_id.is_some() && state.rules.counter_kaboo && !me.counter_kaboo {
                out.push(ActionType::CounterKaboo);
            }
        }
        Phase::Drawn => out.extend([ActionType::Swap, ActionType::Discard]),
        Phase::Effect => {
            if let Some(eff) = state.effect.as_ref().filter(|e| e.by == viewer_id) {
                // A King effect that has already looked at its two cards can
                // only be answered with the swap decision — skipping is no longer
                // on offer, so it must not be advertised.
                if eff.awaiting_king_decision {
                    out.push(ActionType::EffectKingDecide);
                } else {
                    out.push(ActionType::EffectSkip);
                    match eff.rank {
                        Rank::Seven | Rank::Eight => out.push(ActionType::EffectPeekOwn),
                        Rank::Nine | Rank::Ten => out.push(ActionType::EffectPeekOther),
                        Rank::J => out.push(ActionType::EffectBlindSwap),
                        Rank::Q => out.push(ActionType::EffectQueenSwap),
                        Rank::K => out.push(ActionType::EffectKingPeek),
                        _ => {}
                    }
                }
            }
        }
        Phase::FinalSwap => out.extend([ActionType::FinalSwap, ActionType::FinalDecline]),
        _ => {}
    }
    out
}

/// What one viewer is entitled to see. Panics for an unknown viewer: the room
/// layer only projects for seated players.
pub fn project(state: &GameState, viewer_id: &str) -> Projection {
    let revealed = matches!(state.phase, Phase::Reveal | Phase::GameOver);
    assert!(get_player(state, viewer_id).is_some(), "kaboo: cannot project for unknown player {viewer_id}");
    let turn_player = state.players.get(state.turn_seat);
    let turn_player_is_viewer = turn_player.is_some_and(|p| p.id == viewer_id);

    let players = state
        .players
        .iter()
        .map(|p| PlayerView {
            id: p.id.clone(),
            name: p.name.clone(),
            seat: p.seat,
            connected: p.connected,
            hand: p
                .hand
                .iter()
                .enumerate()
                .map(|(index, slot)| SlotView {
                    index,
                    occupied: slot.is_some(),
                    generation: p.generation.get(index).copied().unwrap_or(0),
                    // Faces are public only at the reveal. Your own hand is hidden
                    // from you too: you must remember what you peeked at.
                    face: if revealed { slot.map(public_card) } else { None },
                })
                .collect(),
            hand_count: occupied_count(&p.hand),
            score: p.score,
            ready: p.ready,
            called_kaboo: p.called_kaboo,
            counter_kaboo: p.counter_kaboo,
            final_turn_done: p.final_turn_done,
            is_turn: p.seat == state.turn_seat,
        })
        .collect();

    let recent_from = state.discard.len().saturating_sub(RECENT_DISCARD);
    let log_from = state.log.len().saturating_sub(LOG_TAIL);

    Projection {
        room_code: state.room_code.clone(),
        version: state.version,
        phase: state.phase,
        round: state.round,
        rounds: state.rules.rounds,
        you_id: viewer_id.to_string(),
        players,
        turn_seat: state.turn_seat,
        turn_player_id: turn_player.map(|p| p.id.clone()),
        draw_pile_count: state.deck.len(),
        discard_top: state.discard.last().copied().map(public_card),
        discard_count: state.discard.len(),
        discard_recent: state.discard[recent_from..].iter().copied().map(public_card).collect(),
        discard_seq: state.discard_seq,
        set_aside: if revealed { state.set_aside.map(public_card) } else { None },
        drawn: state.drawn.as_ref().map(|d| DrawnView {
            from: d.from,
            by: d.by.clone(),
            is_yours: d.by == viewer_id,
            // A card drawn from the deck is the drawer's private look; a card
            // taken from the discard was already face-up for everyone.
            card: if d.by == viewer_id || d.from == DrawSource::Discard { Some(public_card(d.card_id)) } else { None },
        }),
        held_card: state.held_card_id.map(|id| HeldView {
            is_yours: turn_player_is_viewer,
            card: if turn_player_is_viewer { Some(public_card(id)) } else { None },
        }),
        tap: state.tap.as_ref().map(|t| TapView {
            player_id: t.player_id.clone(),
            deadline: t.deadline,
            awaiting_placement: t.awaiting_placement,
            source: t.source.as_ref().map(slot_ref),
        }),
        effect: state.effect.as_ref().map(|e| EffectView {
            by: e.by.clone(),
            rank: e.rank,
            awaiting_king_decision: e.awaiting_king_decision,
            a: e.a.as_ref().map(slot_ref),
            b: e.b.as_ref().map(slot_ref),
        }),
        kaboo_caller_id: state.kaboo_caller_id.clone(),
        final_turns_remaining: state.final_turns_remaining.clone(),
        deadlines: Deadlines {
            turn: state.turn_deadline,
            peek: state.peek_deadline,
            tap: state.tap.as_ref().map_or(0, |t| t.deadline),
            final_swap: state.final_swap_deadline,
        },
        peekable_slots: peekable_slots(state, viewer_id),
        can_tap: can_tap_now(state),
        legal_actions: legal_actions(state, viewer_id),
        log: state.log[log_from..].to_vec(),
        round_result: state.round_result.clone(),
        game_result: state.game_result.clone(),
        rules: state.rules.clone(),
        revealed,
    }
}
