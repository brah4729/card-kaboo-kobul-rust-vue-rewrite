use std::collections::BTreeSet;

use kaboo_engine::testkit::act::*;
use kaboo_engine::testkit::{
    ArrangeSpec, Table, arrange, arrange_short_deck, cid, hands, leaked_card_ids, visible_card_ids,
};
use kaboo_engine::{
    Action, ActionType, CardRef, Ctx, DrawSource, EventType, GameState, Phase, Projection, Rank, Suit, apply_action,
    project,
};

use Rank::*;
use Suit::*;

fn visible(view: &Projection) -> BTreeSet<u64> {
    let mut out = BTreeSet::new();
    visible_card_ids(&serde_json::to_value(view).unwrap(), &mut out);
    out
}

fn sees(view: &Projection, id: u8) -> bool {
    visible(view).contains(&u64::from(id))
}

#[track_caller]
fn expect_no_leak(state: &GameState, at: &str) {
    for viewer in &state.players {
        let leaked = leaked_card_ids(state, &viewer.id);
        assert!(leaked.is_empty(), "{at}: {} can see hidden card(s) {leaked:?}", viewer.id);
    }
}

fn playing(n: usize) -> Table {
    let mut t = Table::players(n);
    t.finish_peek();
    t
}

fn deck_out_hands() -> std::collections::BTreeMap<String, Vec<Option<u8>>> {
    hands(&[
        ("p1", &[cid(Five, S), cid(Six, D), cid(Eight, C), cid(Nine, S)]),
        ("p2", &[cid(A, S), cid(Two, D), cid(Three, C), cid(Four, S)]),
    ])
}

// --- the hidden-information boundary ---------------------------------------------

#[test]
fn never_puts_a_card_id_on_a_face_down_slot() {
    let t = playing(3);
    let json = serde_json::to_value(t.view("p1")).unwrap();
    for player in json["players"].as_array().unwrap() {
        for slot in player["hand"].as_array().unwrap() {
            let keys: Vec<&String> = slot.as_object().unwrap().keys().collect();
            let mut keys: Vec<&str> = keys.iter().map(|k| k.as_str()).collect();
            keys.sort();
            assert_eq!(keys, vec!["face", "gen", "index", "occupied"]);
            assert!(slot["face"].is_null());
        }
    }
}

#[test]
fn hides_every_opponent_card_during_play() {
    let mut t = playing(3);
    arrange(
        &mut t,
        ArrangeSpec::default()
            .hand("p1", &[cid(A, S), cid(A, H), cid(Two, S), cid(Three, S)])
            .hand("p2", &[cid(K, C), cid(Q, C), cid(Four, S), cid(Five, S)])
            .hand("p3", &[cid(Six, C), cid(Seven, C), cid(Eight, C), cid(Nine, C)])
            .discard(&[cid(Two, D)]),
    );
    let view = t.view("p1");
    for id in [cid(K, C), cid(Q, C), cid(Six, C), cid(Seven, C)] {
        assert!(!sees(&view, id));
    }
    expect_no_leak(&t.state, "arranged table");
}

#[test]
fn hides_your_own_hand_from_you_too() {
    let t = playing(2);
    let view = t.view("p1");
    for id in t.hand_of("p1").into_iter().flatten() {
        assert!(!sees(&view, id));
    }
}

#[test]
fn never_exposes_the_order_of_the_draw_pile() {
    let t = playing(2);
    let view = t.view("p1");
    for id in &t.state.deck {
        assert!(!sees(&view, *id));
    }
    assert!(view.draw_pile_count > 0);
}

#[test]
fn shows_a_deck_draw_to_its_drawer_and_nobody_else() {
    let mut t = playing(3);
    arrange(&mut t, ArrangeSpec::default().deck(&[cid(K, C), cid(Q, C)]));
    let top = t.state.deck[0];
    t.ok(draw("p2"));
    assert!(sees(&t.view("p2"), top));
    assert!(!sees(&t.view("p1"), top));
    assert!(!sees(&t.view("p3"), top));
    expect_no_leak(&t.state, "after a deck draw");
}

#[test]
fn shows_a_card_taken_from_the_discard_to_everyone() {
    let mut t = playing(2);
    arrange(&mut t, ArrangeSpec::default().discard(&[cid(Seven, D), cid(Nine, C)]));
    t.ok(take("p2"));
    assert!(sees(&t.view("p2"), cid(Nine, C)));
    assert!(sees(&t.view("p1"), cid(Nine, C)));
    expect_no_leak(&t.state, "after taking the discard");
}

#[test]
fn shows_the_deck_out_card_only_to_the_player_who_drew_it() {
    let mut t = playing(2);
    arrange_short_deck(&mut t, deck_out_hands(), &[cid(K, H)], &[cid(Seven, D)]);
    t.ok(draw("p2"));
    let mine = t.view("p2").held_card.unwrap();
    assert!(mine.is_yours);
    assert_eq!(mine.card.unwrap().rank, K);
    let theirs = t.view("p1").held_card.unwrap();
    assert!(!theirs.is_yours);
    assert!(theirs.card.is_none());
    assert!(!sees(&t.view("p1"), cid(K, H)));
    expect_no_leak(&t.state, "during the deck-out decision");
}

#[test]
fn hides_the_declined_last_card_until_the_reveal() {
    let mut t = playing(2);
    arrange_short_deck(&mut t, deck_out_hands(), &[cid(K, H)], &[cid(Seven, D)]);
    t.ok(draw("p2"));
    t.ok(final_decline("p2"));
    assert_eq!(t.state.phase, Phase::Reveal);
    let view = t.view("p1");
    assert_eq!(view.set_aside.unwrap().rank, K);
    assert!(view.revealed);
}

#[test]
fn hides_a_penalty_card_from_the_tapper() {
    let mut t = playing(2);
    arrange(
        &mut t,
        ArrangeSpec::default()
            .hand("p1", &[cid(A, H), cid(Two, S), cid(Three, S), cid(Four, S)])
            .deck(&[cid(K, C), cid(Q, C)])
            .discard(&[cid(Seven, C)]),
    );
    let penalty = t.state.deck[0];
    let seq = t.state.discard_seq;
    t.ok(tap("p1", seq));
    t.ok(tap_choose("p1", r("p1", 0)));
    assert!(!sees(&t.view("p1"), penalty));
    expect_no_leak(&t.state, "after a wrong tap");
}

#[test]
fn reveals_every_hand_and_no_draw_pile_once_the_round_is_over() {
    let mut t = playing(2);
    arrange(
        &mut t,
        ArrangeSpec::default()
            .hand("p1", &[cid(A, S), cid(A, H), cid(Two, S), cid(Three, S)])
            .hand("p2", &[cid(K, C), cid(Q, C), cid(Four, S), cid(Five, S)])
            .deck(&[cid(Five, D)]), // plain: discarding it ends the turn, no effect
    );
    t.ok(kaboo("p2"));
    t.ok(draw("p1"));
    t.ok(discard("p1"));
    assert_eq!(t.state.phase, Phase::Reveal);

    let view = t.view("p1");
    for player in &view.players {
        for slot in player.hand.iter().filter(|s| s.occupied) {
            assert!(slot.face.is_some());
        }
    }
    assert!(sees(&view, cid(K, C)));
    assert!(sees(&view, cid(Q, C)));
    for id in &t.state.deck {
        assert!(!sees(&view, *id));
    }
}

#[test]
fn never_computes_or_ships_a_hand_total_before_the_reveal() {
    let mut t = playing(3);
    arrange(
        &mut t,
        ArrangeSpec::default()
            .hand("p1", &[cid(A, S), cid(A, H), cid(Two, S), cid(Three, S)])
            .hand("p2", &[cid(K, C), cid(Q, C), cid(Four, S), cid(Five, S)]),
    );
    for player in &t.state.players {
        let view = t.view(&player.id);
        assert!(view.round_result.is_none());
        assert!(view.game_result.is_none());
        // The word "total" must not appear anywhere in a live projection, under
        // any key — the cheapest possible guard against a future leak.
        assert!(!serde_json::to_string(&view).unwrap().contains("total"));
    }
}

#[test]
fn reports_a_peek_as_a_private_event_never_as_stored_state() {
    let mut t = Table::players(2);
    let result = t.ok(peek("p1", 2));
    let peeked: Vec<_> = result.events.iter().filter(|e| e.kind == EventType::Peeked).collect();
    assert_eq!(peeked.len(), 1);
    assert_eq!(peeked[0].to.as_deref(), Some("p1"));
    let face = peeked[0].card.expect("the peeked face");

    // Even the peeker's own projection shows no face — they have to remember it.
    let view = t.view("p1");
    let mine = view.players.iter().find(|p| p.id == "p1").unwrap();
    assert!(mine.hand[2].face.is_none());

    // The opponent learns nothing at all.
    assert!(!sees(&t.view("p2"), face.id));

    // The log records that a look HAPPENED, and never what was seen.
    let log = serde_json::to_value(&t.view("p2").log).unwrap();
    for entry in log.as_array().unwrap() {
        assert!(!entry.as_object().unwrap().contains_key("card"));
    }
}

#[test]
fn never_advertises_a_peek_at_a_slot_the_rules_do_not_allow() {
    let mut t = Table::players(2);
    let view = t.view("p1");
    assert_eq!(view.peekable_slots, vec![2, 3]);
    assert!(view.legal_actions.contains(&ActionType::Peek));

    t.ok(peek("p1", 2));
    assert_eq!(t.view("p1").peekable_slots, vec![3]);

    t.ok(peek("p1", 3));
    let view = t.view("p1");
    assert!(view.peekable_slots.is_empty());
    assert!(!view.legal_actions.contains(&ActionType::Peek));
    assert!(view.legal_actions.contains(&ActionType::Ready));
    t.fail(peek("p1", 0));
}

#[test]
fn keeps_the_tap_target_public_but_the_surrounding_hands_hidden() {
    let mut t = playing(3);
    arrange(
        &mut t,
        ArrangeSpec::default()
            .hand("p1", &[cid(Seven, H), cid(Two, S), cid(Three, S), cid(Four, S)])
            .hand("p3", &[cid(Seven, D), cid(Five, S), cid(Six, S), cid(Eight, S)])
            .discard(&[cid(Seven, C)]),
    );
    let seq = t.state.discard_seq;
    t.ok(tap("p1", seq));
    t.ok(tap_choose("p1", r("p3", 0)));
    let top = t.view("p2").discard_top.unwrap();
    assert_eq!((top.rank, top.suit), (Seven, Some(D)));
    expect_no_leak(&t.state, "after a tap");
}

#[test]
fn publishes_discard_seq_because_a_tap_cannot_be_built_without_it() {
    let mut t = playing(2);
    arrange(
        &mut t,
        ArrangeSpec::default()
            .hand("p1", &[cid(Seven, H), cid(Two, S), cid(Three, S), cid(Four, S)])
            .discard(&[cid(Seven, C)]),
    );
    let view = t.view("p1");
    let before = view.discard_seq;
    // Build the tap from the projection ALONE — never from the state.
    assert!(view.can_tap);
    t.ok(tap("p1", view.discard_seq));
    t.ok(tap_choose("p1", r("p1", 0)));
    assert!(t.view("p1").discard_seq > before);
}

// --- legal_actions is honest -------------------------------------------------------

fn first_occupied(hand: &[Option<u8>]) -> Option<usize> {
    hand.iter().position(Option::is_some)
}

fn occupied(hand: &[Option<u8>]) -> Vec<usize> {
    hand.iter().enumerate().filter(|(_, s)| s.is_some()).map(|(i, _)| i).collect()
}

fn build(kind: ActionType, state: &GameState, viewer: &str, peekable: &[usize]) -> Option<Action> {
    let me = state.players.iter().find(|p| p.id == viewer).unwrap();
    let p = viewer.to_string();
    let others = || state.players.iter().filter(|o| o.id != viewer);
    let cr = |player_id: &str, slot: usize| CardRef { player_id: player_id.into(), slot };
    Some(match kind {
        ActionType::Peek => Action::Peek { player_id: p, slot: *peekable.first()? },
        ActionType::Ready => Action::Ready { player_id: p },
        ActionType::Continue => Action::Continue { player_id: p },
        ActionType::CallKaboo => Action::CallKaboo { player_id: p },
        ActionType::CounterKaboo => Action::CounterKaboo { player_id: p },
        ActionType::Discard => Action::Discard { player_id: p },
        ActionType::EffectSkip => Action::EffectSkip { player_id: p },
        ActionType::FinalDecline => Action::FinalDecline { player_id: p },
        ActionType::TapDeclinePlace => Action::TapDeclinePlace { player_id: p },
        ActionType::Draw => Action::Draw {
            player_id: p,
            from: if state.deck.is_empty() { DrawSource::Discard } else { DrawSource::Deck },
        },
        ActionType::Swap => Action::Swap { player_id: p, slot: 0 },
        ActionType::FinalSwap => Action::FinalSwap { player_id: p, slot: 0 },
        ActionType::Tap => Action::Tap { player_id: p, seen_seq: state.discard_seq },
        ActionType::TapChoose => Action::TapChoose { target: cr(viewer, first_occupied(&me.hand)?), player_id: p },
        ActionType::TapPlace => Action::TapPlace { player_id: p, slot: first_occupied(&me.hand)? },
        ActionType::EffectPeekOwn => Action::EffectPeekOwn { player_id: p, slot: first_occupied(&me.hand)? },
        ActionType::EffectPeekOther => {
            let (other, slot) = others().find_map(|o| first_occupied(&o.hand).map(|s| (o.id.clone(), s)))?;
            Action::EffectPeekOther { player_id: p, target_player_id: other, slot }
        }
        ActionType::EffectBlindSwap | ActionType::EffectKingPeek => {
            let mine = occupied(&me.hand);
            let pair = others()
                .find_map(|o| {
                    let theirs = occupied(&o.hand);
                    (!mine.is_empty() && !theirs.is_empty()).then(|| (cr(viewer, mine[0]), cr(&o.id, theirs[0])))
                })
                .or_else(|| (mine.len() >= 2).then(|| (cr(viewer, mine[0]), cr(viewer, mine[1]))))?;
            if kind == ActionType::EffectBlindSwap {
                Action::EffectBlindSwap { player_id: p, a: pair.0, b: pair.1 }
            } else {
                Action::EffectKingPeek { player_id: p, a: pair.0, b: pair.1 }
            }
        }
        ActionType::EffectQueenSwap => {
            let slot = first_occupied(&me.hand)?;
            let (other, theirs) = others().find_map(|o| first_occupied(&o.hand).map(|s| (o.id.clone(), s)))?;
            Action::EffectQueenSwap { player_id: p, slot, target: cr(&other, theirs) }
        }
        ActionType::EffectKingDecide => Action::EffectKingDecide { player_id: p, swap: true },
        ActionType::Leave => return None,
    })
}

/// Every advertised action must be accepted (unconstructible ones excepted).
#[track_caller]
fn assert_honest(t: &Table, at: &str) {
    for viewer in &t.state.players {
        let view = project(&t.state, &viewer.id);
        for kind in &view.legal_actions {
            let Some(action) = build(*kind, &t.state, &viewer.id, &view.peekable_slots) else { continue };
            // Pure: inspected and thrown away, so the table is intact for the next one.
            let mut rng = || 0.5;
            let result = apply_action(&t.state, &action, &mut Ctx { now: t.now, rng: &mut rng });
            assert!(
                result.error.is_none(),
                "{at}: legal_actions offered {kind:?} to {} but the engine refused it",
                viewer.id
            );
        }
    }
}

#[test]
fn is_honest_through_peek_turn_draw_and_effect_phases() {
    let mut t = Table::players(2);
    assert_honest(&t, "peek");
    t.finish_peek();
    assert_honest(&t, "turnStart");
    t.ok(draw("p2"));
    assert_honest(&t, "drawn");
    t.ok(discard("p2"));
    assert_honest(&t, "turnStart again");
}

#[test]
fn is_honest_in_the_effect_phase() {
    let mut t = playing(2);
    arrange(&mut t, ArrangeSpec::default().deck(&[cid(Seven, D), cid(Three, C)]));
    t.ok(draw("p2"));
    t.ok(discard("p2"));
    assert_eq!(t.state.phase, Phase::Effect);
    assert_honest(&t, "effect");
    t.ok(peek_own("p2", 3));
    assert_eq!(t.state.phase, Phase::TurnStart);
    assert_honest(&t, "after an effect resolved");
}

#[test]
fn is_honest_through_a_kings_two_step_decision() {
    let mut t = playing(2);
    arrange(&mut t, ArrangeSpec::default().deck(&[cid(K, D), cid(Three, C)]));
    t.ok(draw("p2"));
    t.ok(discard("p2"));
    assert_eq!(t.state.phase, Phase::Effect);
    t.ok(king_peek("p2", r("p1", 0), r("p2", 0)));
    assert!(t.state.effect.as_ref().unwrap().awaiting_king_decision);
    assert_honest(&t, "awaiting the King decision");
}

#[test]
fn is_honest_while_a_tap_is_claimed() {
    let mut t = playing(3);
    arrange(
        &mut t,
        ArrangeSpec::default()
            .hand("p3", &[cid(Seven, D), cid(Five, S), cid(Six, S), cid(Eight, S)])
            .discard(&[cid(Seven, C)]),
    );
    let seq = t.state.discard_seq;
    t.ok(tap("p1", seq));
    assert_honest(&t, "tap claimed");
    t.ok(tap_choose("p1", r("p3", 0)));
    assert!(t.state.tap.as_ref().unwrap().awaiting_placement);
    assert_honest(&t, "tap awaiting placement");
}

#[test]
fn is_honest_during_the_deck_out_decision() {
    let mut t = playing(2);
    arrange_short_deck(&mut t, deck_out_hands(), &[cid(K, H)], &[cid(Seven, D)]);
    t.ok(draw("p2"));
    assert_honest(&t, "finalSwap");
}

// --- the boundary does not leak nothing by shipping nothing ------------------------

#[test]
fn publishes_the_discard_pile_face_up() {
    let mut t = playing(2);
    arrange(&mut t, ArrangeSpec::default().discard(&[cid(Seven, D), cid(Nine, C)]));
    let view = t.view("p1");
    let top = view.discard_top.unwrap();
    assert_eq!((top.rank, top.suit), (Nine, Some(C)));
    assert!(view.discard_recent.iter().any(|c| c.id == cid(Seven, D)));
    assert_eq!(view.discard_count, 2);
}

#[test]
fn counts_cards_in_every_hand_without_naming_them() {
    let t = playing(3);
    let view = t.view("p1");
    for p in &view.players {
        assert_eq!(p.hand_count, 4);
        assert_eq!(p.hand.iter().filter(|s| s.occupied).count(), 4);
    }
}

#[test]
fn tells_everyone_whose_turn_it_is_and_who_called_kaboo() {
    let mut t = playing(2);
    t.ok(kaboo("p2"));
    let view = t.view("p1");
    assert_eq!(view.kaboo_caller_id.as_deref(), Some("p2"));
    assert_eq!(view.turn_player_id.as_deref(), Some("p1"));
    assert!(view.players.iter().find(|p| p.id == "p2").unwrap().called_kaboo);
}
