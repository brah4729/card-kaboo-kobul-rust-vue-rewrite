//! Engine events: what just happened, for the transport to route.
//!
//! An event with `to` set is PRIVATE: the room delivers it only to that
//! player. Without `to` it is public, and public events never leave the server —
//! the same facts already reach every client through `Projection::log`.
//!
//! One flat struct with optional fields, rather than an enum per event, so the
//! wire shape stays the one the browser already reads (`{ type, to, card, ... }`).
//! `card` is the one payload field that decides whether information is private:
//! a peek or a private deck-out look ships its face here and nowhere else.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::types::{CardRef, DrawSource, GameResult, Phase, PlayerId, PublicCard, Rank, RoundResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum EventType {
    Peeked,
    Revealed,
    Drew,
    TookDiscard,
    Swapped,
    Discarded,
    EffectUsed,
    EffectSkipped,
    TapClaimed,
    TapResolved,
    TapWrong,
    TapTimeout,
    Placed,
    Kaboo,
    CounterKaboo,
    AutoKaboo,
    FinalSwap,
    FinalDecline,
    DeckOut,
    TurnLimit,
    Reveal,
    RoundEnd,
    GameEnd,
    TurnChanged,
    #[default]
    PhaseChanged,
}

macro_rules! optional_fields {
    ($(#[$meta:meta])* pub struct $name:ident { $($(#[$fmeta:meta])* $field:ident : $ty:ty),* $(,)? }) => {
        $(#[$meta])*
        pub struct $name {
            #[serde(rename = "type")]
            pub kind: EventType,
            $(
                $(#[$fmeta])*
                #[serde(default, skip_serializing_if = "Option::is_none")]
                #[ts(optional)]
                pub $field: Option<$ty>,
            )*
        }
    };
}

optional_fields! {
    #[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
    #[serde(rename_all = "camelCase")]
    #[ts(export)]
    pub struct EngineEvent {
        /// Present ⇒ PRIVATE: route only to this player. Absent ⇒ public.
        to: PlayerId,
        /// A face carried for the recipient of THIS event only.
        card: PublicCard,
        by: PlayerId,
        player_id: PlayerId,
        slot: usize,
        reason: String,
        rank: Rank,
        decision: String,
        target_player_id: PlayerId,
        target_slot: usize,
        own_slot: usize,
        target: CardRef,
        a: CardRef,
        b: CardRef,
        from: DrawSource,
        /// Where a tapped card came from.
        source: CardRef,
        into: CardRef,
        hidden: bool,
        seat: usize,
        phase: Phase,
        ready_id: PlayerId,
        left_id: PlayerId,
        round: u32,
        limit: u32,
        hands: BTreeMap<PlayerId, Vec<Option<PublicCard>>>,
        set_aside: Option<PublicCard>,
        totals: BTreeMap<PlayerId, i32>,
        winner_id: PlayerId,
        penalty_ids: Vec<PlayerId>,
        round_result: RoundResult,
        game_result: GameResult,
    }
}

/// A fresh event of `kind` with every payload field empty. Fill it with struct
/// update syntax: `EngineEvent { by: Some(id), ..ev(EventType::Kaboo) }`.
pub fn ev(kind: EventType) -> EngineEvent {
    EngineEvent { kind, ..Default::default() }
}

/// Append a PUBLIC event.
pub fn emit(events: &mut Vec<EngineEvent>, event: EngineEvent) {
    events.push(event);
}

/// Append a PRIVATE event — the transport must route it only to `to`.
///
/// Peeks and other one-off looks travel this way precisely because they are not
/// stored anywhere: if the event is missed, the information is gone, exactly as
/// it would be at a real table.
pub fn emit_private(events: &mut Vec<EngineEvent>, to: &str, event: EngineEvent) {
    events.push(EngineEvent { to: Some(to.to_string()), ..event });
}
