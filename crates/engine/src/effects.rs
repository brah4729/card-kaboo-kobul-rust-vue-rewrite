//! The power cards: 7/8 look at your own, 9/10 look at an opponent's, J blind
//! swap, Q swap-and-look, K look-at-two-then-decide.

use crate::cards::public_card;
use crate::errors::{Handled, illegal};
use crate::events::{EngineEvent, EventType, emit, emit_private, ev};
use crate::state::{add_log, is_slot_occupied, set_slot, slot_card};
use crate::turn::advance_turn;
use crate::types::{Action, CardRef, Ctx, GameState, LogEntry, LogKind, Rank};

/// Move the contents of two slots into each other. Both must be occupied.
fn swap_cards(state: &mut GameState, a: &CardRef, b: &CardRef) -> bool {
    let (Some(ca), Some(cb)) = (slot_card(state, a), slot_card(state, b)) else {
        return false;
    };
    set_slot(state, &a.player_id, a.slot, cb);
    set_slot(state, &b.player_id, b.slot, ca);
    true
}

fn finish_effect(state: &mut GameState, ctx: &mut Ctx, events: &mut Vec<EngineEvent>) {
    state.effect = None;
    advance_turn(state, ctx, events);
}

fn effect_log(state: &mut GameState, me: &str, target: Option<&str>, text: &str) {
    let mut entry = LogEntry::new(state.round, LogKind::Effect).actor(me).text(text);
    if let Some(t) = target {
        entry = entry.target(t);
    }
    add_log(state, entry);
}

/// Resolve the pending effect. Only the player who triggered it may act, and
/// every branch but the King's first step ends the turn.
///
/// A player may always `EFFECT_SKIP` their own power. Nothing in the brief
/// makes an effect compulsory, and forcing a blind swap on a player who does
/// not want one would be punishing them for their own card. See AGENTS.md §7.
pub fn resolve_effect(state: &mut GameState, action: &Action, ctx: &mut Ctx, events: &mut Vec<EngineEvent>) -> Handled {
    let Some(eff) = state.effect.clone() else { return Err(illegal()) };
    let me = action.player_id().to_string();
    if eff.by != me {
        return Err(illegal());
    }
    let rank = eff.rank;
    let used = |extra: EngineEvent| EngineEvent { by: Some(me.clone()), rank: Some(rank), ..extra };

    // --- King, second step: the peek is done, now decide whether to swap. ---
    if eff.awaiting_king_decision {
        let Action::EffectKingDecide { swap, .. } = action else { return Err(illegal()) };
        let (Some(a), Some(b)) = (eff.a.as_ref(), eff.b.as_ref()) else { return Err(illegal()) };
        if *swap {
            if !swap_cards(state, a, b) {
                return Err(illegal());
            }
            effect_log(state, &me, None, "kingSwap");
            emit(events, used(EngineEvent { decision: Some("swap".into()), ..ev(EventType::EffectUsed) }));
        } else {
            effect_log(state, &me, None, "kingKeep");
            emit(events, used(EngineEvent { decision: Some("keep".into()), ..ev(EventType::EffectUsed) }));
        }
        // The caller announced the decision; the targets see their slots change
        // through `gen` alone, never through a card face.
        finish_effect(state, ctx, events);
        return Ok(());
    }

    match action {
        Action::EffectKingDecide { .. } => Err(illegal()),

        Action::EffectSkip { .. } => {
            effect_log(state, &me, None, "skipped");
            emit(events, used(ev(EventType::EffectSkipped)));
            finish_effect(state, ctx, events);
            Ok(())
        }

        // --- 7 / 8: look at one of your own cards -----------------------------
        Action::EffectPeekOwn { slot, .. } => {
            if !matches!(rank, Rank::Seven | Rank::Eight) {
                return Err(illegal());
            }
            let r = CardRef { player_id: me.clone(), slot: *slot };
            let Some(id) = slot_card(state, &r) else { return Err(illegal()) };
            emit_private(
                events,
                &me,
                EngineEvent {
                    player_id: Some(me.clone()),
                    slot: Some(*slot),
                    card: Some(public_card(id)),
                    reason: Some("effect".into()),
                    ..ev(EventType::Peeked)
                },
            );
            effect_log(state, &me, None, "ownPeek");
            emit(
                events,
                used(EngineEvent {
                    target_player_id: Some(me.clone()),
                    target_slot: Some(*slot),
                    ..ev(EventType::EffectUsed)
                }),
            );
            finish_effect(state, ctx, events);
            Ok(())
        }

        // --- 9 / 10: look at one opponent card --------------------------------
        Action::EffectPeekOther { target_player_id, slot, .. } => {
            if !matches!(rank, Rank::Nine | Rank::Ten) {
                return Err(illegal());
            }
            if *target_player_id == me {
                return Err(illegal());
            }
            let r = CardRef { player_id: target_player_id.clone(), slot: *slot };
            let Some(id) = slot_card(state, &r) else { return Err(illegal()) };
            emit_private(
                events,
                &me,
                EngineEvent {
                    player_id: Some(target_player_id.clone()),
                    slot: Some(*slot),
                    card: Some(public_card(id)),
                    reason: Some("effect".into()),
                    ..ev(EventType::Peeked)
                },
            );
            effect_log(state, &me, Some(target_player_id), "otherPeek");
            emit(
                events,
                used(EngineEvent {
                    target_player_id: Some(target_player_id.clone()),
                    target_slot: Some(*slot),
                    ..ev(EventType::EffectUsed)
                }),
            );
            finish_effect(state, ctx, events);
            Ok(())
        }

        // --- Jack: blind-swap any two cards -----------------------------------
        Action::EffectBlindSwap { a, b, .. } => {
            if rank != Rank::J || a == b {
                return Err(illegal());
            }
            if !is_slot_occupied(state, a) || !is_slot_occupied(state, b) {
                return Err(illegal());
            }
            if !swap_cards(state, a, b) {
                return Err(illegal());
            }
            let target = if a.player_id == me { &b.player_id } else { &a.player_id };
            effect_log(state, &me, Some(target), "blindSwap");
            // Both slots' owners learn THAT they changed, never what they became.
            emit(events, used(EngineEvent { a: Some(a.clone()), b: Some(b.clone()), ..ev(EventType::EffectUsed) }));
            finish_effect(state, ctx, events);
            Ok(())
        }

        // --- Queen: swap one of yours with any other card, see what you receive
        Action::EffectQueenSwap { slot, target, .. } => {
            if rank != Rank::Q {
                return Err(illegal());
            }
            let own = CardRef { player_id: me.clone(), slot: *slot };
            if !is_slot_occupied(state, &own) || target.player_id == me {
                return Err(illegal());
            }
            let Some(received) = slot_card(state, target) else { return Err(illegal()) };
            if !swap_cards(state, &own, target) {
                return Err(illegal());
            }
            // "you may look at the card you receive" — auto-revealed: a free look
            // is never disadvantageous, so prompting for it would only cost a
            // round trip.
            emit_private(
                events,
                &me,
                EngineEvent {
                    reason: Some("queen".into()),
                    player_id: Some(target.player_id.clone()),
                    slot: Some(target.slot),
                    card: Some(public_card(received)),
                    ..ev(EventType::Revealed)
                },
            );
            effect_log(state, &me, Some(&target.player_id), "queenSwap");
            emit(
                events,
                used(EngineEvent { own_slot: Some(*slot), target: Some(target.clone()), ..ev(EventType::EffectUsed) }),
            );
            finish_effect(state, ctx, events);
            Ok(())
        }

        // --- King: look at any two cards, then decide whether to swap them ----
        Action::EffectKingPeek { a, b, .. } => {
            if rank != Rank::K || a == b {
                return Err(illegal());
            }
            let (Some(ca), Some(cb)) = (slot_card(state, a), slot_card(state, b)) else {
                return Err(illegal());
            };
            for (r, id) in [(a, ca), (b, cb)] {
                emit_private(
                    events,
                    &me,
                    EngineEvent {
                        reason: Some("king".into()),
                        player_id: Some(r.player_id.clone()),
                        slot: Some(r.slot),
                        card: Some(public_card(id)),
                        ..ev(EventType::Peeked)
                    },
                );
            }
            if let Some(e) = state.effect.as_mut() {
                e.a = Some(a.clone());
                e.b = Some(b.clone());
                e.awaiting_king_decision = true;
            }
            emit(
                events,
                used(EngineEvent {
                    a: Some(a.clone()),
                    b: Some(b.clone()),
                    decision: Some("pending".into()),
                    ..ev(EventType::EffectUsed)
                }),
            );
            Ok(())
        }

        _ => Err(illegal()),
    }
}
