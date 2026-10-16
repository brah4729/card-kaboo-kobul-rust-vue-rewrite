use std::collections::BTreeSet;

use kaboo_engine::cards::occupied_count;
use kaboo_engine::testkit::act::*;
use kaboo_engine::testkit::{ArrangeSpec, Table, arrange, card_label, cid, private_to, public_events};
use kaboo_engine::{DrawSource, EngineErrorCode, EventType, Phase, Rank, RuleConfig, Suit};

use Rank::*;
use Suit::*;

fn rules() -> RuleConfig {
    RuleConfig::default()
}

fn top(t: &Table) -> Option<u8> {
    t.state.discard.last().copied()
}

// --- round setup -----------------------------------------------------------------

#[test]
fn deals_four_cards_to_each_player_in_fixed_slots() {
    let t = Table::players(2);
    for id in &t.ids {
        let hand = t.hand_of(id);
        assert_eq!(hand.len(), 4);
        assert!(hand.iter().all(Option::is_some));
    }
}

#[test]
fn leaves_the_rest_in_the_draw_pile_and_starts_the_discard_empty() {
    let t = Table::players(2);
    assert_eq!(t.state.deck.len(), 52 - 8);
    assert!(t.state.discard.is_empty());
    assert_eq!(t.state.discard_seq, 0);
}

#[test]
fn opens_in_peek_with_the_turn_on_the_seat_after_the_dealer() {
    let t = Table::players(4);
    assert_eq!(t.state.phase, Phase::Peek);
    assert_eq!(t.state.dealer_seat, 0);
    assert_eq!(t.state.turn_seat, 1);
}

#[test]
fn deals_no_duplicate_cards() {
    let t = Table::players(8);
    let mut all: Vec<u8> = t.state.deck.clone();
    for id in &t.ids {
        all.extend(t.hand_of(id).into_iter().flatten());
    }
    let unique: BTreeSet<_> = all.iter().collect();
    assert_eq!(unique.len(), all.len());
    assert!(!all.is_empty());
}

// --- peek phase --------------------------------------------------------------------

#[test]
fn delivers_the_peeked_card_only_to_the_player_who_peeked() {
    let mut t = Table::players(2);
    let result = t.ok(peek("p1", 2));
    let mine = private_to(&result.events, "p1");
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].kind, EventType::Peeked);
    assert_eq!(mine[0].slot, Some(2));
    assert!(mine[0].card.is_some());
    // p2 must not receive p1's peek.
    assert!(private_to(&result.events, "p2").is_empty());
}

#[test]
fn reveals_the_card_that_is_actually_in_the_slot() {
    let mut t = Table::players(2);
    arrange(&mut t, ArrangeSpec::default().hand("p1", &[cid(A, S), cid(Two, S), cid(Seven, H), cid(K, D)]));
    let result = t.ok(peek("p1", 2));
    assert_eq!(result.events[0].card.unwrap().rank, Seven);
}

#[test]
fn allows_each_bottom_slot_exactly_once() {
    let mut t = Table::players(2);
    t.ok(peek("p1", 2));
    t.fail(peek("p1", 2));
    t.ok(peek("p1", 3));
    t.fail(peek("p1", 3));
}

#[test]
fn refuses_the_top_row() {
    let mut t = Table::players(2);
    t.fail(peek("p1", 0));
    t.fail(peek("p1", 1));
}

#[test]
fn refuses_actions_from_a_player_who_is_already_ready() {
    let mut t = Table::players(2);
    t.ok(ready("p1"));
    t.fail(peek("p1", 2));
}

#[test]
fn begins_turns_once_every_player_is_ready() {
    let mut t = Table::players(2);
    t.ok(ready("p1"));
    assert_eq!(t.state.phase, Phase::Peek);
    t.ok(ready("p2"));
    assert_eq!(t.state.phase, Phase::TurnStart);
}

#[test]
fn auto_readies_everyone_when_the_peek_deadline_passes() {
    let mut t = Table::players(2);
    t.advance(rules().peek_deadline_ms + 1);
    let ticked = t.send(ready("p1"));
    assert!(ticked.version_changed);
    assert_eq!(t.state.phase, Phase::TurnStart);
}

#[test]
fn does_not_let_a_player_ready_another_player() {
    let mut t = Table::players(2);
    t.ok(ready("p1"));
    assert!(!t.state.players.iter().find(|p| p.id == "p2").unwrap().ready);
}

// --- turn flow ---------------------------------------------------------------------

fn playing() -> Table {
    let mut t = Table::players(2);
    t.finish_peek();
    t
}

#[test]
fn gives_the_first_turn_to_the_player_after_the_dealer() {
    let t = playing();
    assert_eq!(t.state.phase, Phase::TurnStart);
    assert_eq!(t.turn_id(), "p2");
}

#[test]
fn refuses_turn_actions_from_the_player_whose_turn_it_is_not() {
    let mut t = playing();
    let result = t.fail(draw("p1"));
    assert_eq!(result.error.unwrap().code, EngineErrorCode::NotYourTurn);
}

#[test]
fn refuses_taking_from_an_empty_discard_pile() {
    let mut t = playing();
    t.fail(take("p2"));
}

#[test]
fn draws_from_the_deck_and_shows_the_card_only_to_the_drawer() {
    let mut t = playing();
    let next = t.state.deck[0];
    let result = t.ok(draw("p2"));
    assert_eq!(t.state.phase, Phase::Drawn);
    assert_eq!(t.state.drawn.as_ref().unwrap().card_id, next);
    assert!(private_to(&result.events, "p2").iter().any(|e| e.kind == EventType::Drew && e.card.is_some()));
    // The public event must carry no card.
    let public = public_events(&result.events);
    let drew = public.iter().find(|e| e.kind == EventType::Drew).expect("a public drew event");
    assert!(drew.card.is_none());
}

#[test]
fn swaps_the_drawn_card_in_and_pushes_the_replaced_card_to_the_discard() {
    let mut t = playing();
    arrange(
        &mut t,
        ArrangeSpec::default()
            .hand("p2", &[cid(A, S), cid(Two, S), cid(K, D), cid(Five, C)])
            .deck(&[cid(Nine, H)])
            .discard(&[]),
    );
    let replaced = t.hand_of("p2")[1];
    t.ok(draw("p2"));
    t.ok(swap("p2", 1));
    assert_eq!(t.hand_of("p2")[1], Some(cid(Nine, H)));
    assert_eq!(top(&t), replaced);
    assert!(t.state.drawn.is_none());
}

#[test]
fn discards_the_drawn_card_and_passes_the_turn() {
    let mut t = playing();
    arrange(&mut t, ArrangeSpec::default().deck(&[cid(Four, H)]));
    t.ok(draw("p2"));
    t.ok(discard("p2"));
    assert_eq!(top(&t), Some(cid(Four, H)));
    assert_eq!(t.turn_id(), "p1");
    assert_eq!(t.state.phase, Phase::TurnStart);
}

#[test]
fn takes_the_top_of_the_discard() {
    let mut t = playing();
    arrange(&mut t, ArrangeSpec::default().discard(&[cid(Three, S), cid(Eight, D)]));
    t.ok(take("p2"));
    let drawn = t.state.drawn.clone().unwrap();
    assert_eq!(drawn.from, DrawSource::Discard);
    assert_eq!(drawn.card_id, cid(Eight, D));
    assert_eq!(t.state.discard.len(), 1);
    t.ok(discard("p2"));
    assert_eq!(top(&t), Some(cid(Eight, D)));
}

#[test]
fn closes_the_tap_window_when_someone_takes_from_the_discard() {
    let mut t = playing();
    arrange(&mut t, ArrangeSpec::default().discard(&[cid(Three, S)]));
    let before = t.state.discard_seq;
    t.ok(take("p2"));
    assert!(t.state.discard_seq > before);
}

#[test]
fn refuses_swap_and_discard_when_nothing_has_been_drawn() {
    let mut t = playing();
    t.fail(swap("p2", 0));
    t.fail(discard("p2"));
}

#[test]
fn refuses_an_out_of_range_slot() {
    // (A negative slot cannot even be expressed: it fails to deserialize at the
    // room layer, before the engine sees it.)
    let mut t = playing();
    t.ok(draw("p2"));
    t.fail(swap("p2", 4));
}

#[test]
fn lets_a_player_swap_into_an_empty_slot_without_discarding_anything() {
    let mut t = playing();
    arrange(
        &mut t,
        ArrangeSpec::default().hand_slots("p2", &[None, None, None, None]).deck(&[cid(Six, H)]).discard(&[]),
    );
    t.ok(draw("p2"));
    t.ok(swap("p2", 2));
    assert_eq!(t.hand_of("p2")[2], Some(cid(Six, H)));
    assert!(t.state.discard.is_empty());
}

#[test]
fn plays_a_safe_default_when_a_player_runs_out_of_time() {
    let mut t = playing();
    arrange(&mut t, ArrangeSpec::default().deck(&[cid(Four, H)]));
    t.advance(rules().turn_deadline_ms + 1);
    let result = t.send(discard("p2"));
    assert!(result.version_changed);
    // Auto-play drew AND discarded, so the turn has moved on.
    assert_eq!(t.turn_id(), "p1");
    assert_eq!(top(&t), Some(cid(Four, H)));
}

#[test]
fn never_auto_calls_kaboo_on_a_timed_out_player() {
    let mut t = playing();
    t.advance(rules().turn_deadline_ms + 1);
    t.send(draw("p2"));
    assert!(t.state.kaboo_caller_id.is_none());
    assert!(t.state.players.iter().all(|p| !p.called_kaboo));
}

// --- effects -----------------------------------------------------------------------

/// Draw a specific card and discard it, landing in the effect phase.
fn discard_card(t: &mut Table, card_id: u8) {
    arrange(t, ArrangeSpec::default().deck(&[card_id]));
    t.ok(draw("p2"));
    t.ok(discard("p2"));
}

fn peeked_to(events: &[kaboo_engine::EngineEvent], player: &str) -> usize {
    private_to(events, player).iter().filter(|e| e.kind == EventType::Peeked).count()
}

#[test]
fn skips_the_effect_phase_for_a_plain_card() {
    let mut t = playing();
    discard_card(&mut t, cid(Four, H));
    assert_eq!(t.state.phase, Phase::TurnStart);
    assert_eq!(t.turn_id(), "p1");
}

#[test]
fn seven_and_eight_look_at_one_of_your_own_cards_privately() {
    let mut t = playing();
    discard_card(&mut t, cid(Seven, H));
    assert_eq!(t.state.phase, Phase::Effect);
    let result = t.ok(peek_own("p2", 0));
    assert_eq!(peeked_to(&result.events, "p2"), 1);
    assert_eq!(peeked_to(&result.events, "p1"), 0);
    assert_eq!(t.turn_id(), "p1");
}

#[test]
fn refuses_a_seven_effect_on_an_opponent_slot() {
    let mut t = playing();
    discard_card(&mut t, cid(Eight, H));
    t.fail(peek_other("p2", "p1", 0));
}

#[test]
fn nine_and_ten_look_at_one_opponent_card() {
    let mut t = playing();
    arrange(&mut t, ArrangeSpec::default().hand("p1", &[cid(Q, S), cid(Two, D), cid(Three, C), cid(Four, S)]));
    discard_card(&mut t, cid(Nine, H));
    let result = t.ok(peek_other("p2", "p1", 0));
    let mine: Vec<_> = private_to(&result.events, "p2").into_iter().filter(|e| e.kind == EventType::Peeked).collect();
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].card.unwrap().rank, Q);
    // The target learns nothing about their own card from this.
    assert_eq!(peeked_to(&result.events, "p1"), 0);
}

#[test]
fn refuses_a_nine_effect_on_your_own_slot() {
    let mut t = playing();
    discard_card(&mut t, cid(Ten, H));
    t.fail(peek_other("p2", "p2", 0));
}

#[test]
fn jack_blind_swaps_two_cards_revealing_neither() {
    let mut t = playing();
    arrange(
        &mut t,
        ArrangeSpec::default()
            .hand("p1", &[cid(A, S), cid(Two, S), cid(Three, S), cid(Four, S)])
            .hand("p2", &[cid(K, H), cid(Six, S), cid(Seven, S), cid(Eight, S)]),
    );
    discard_card(&mut t, cid(J, H));
    let result = t.ok(blind_swap("p2", r("p2", 0), r("p1", 3)));
    assert_eq!(t.hand_of("p2")[0], Some(cid(Four, S)));
    assert_eq!(t.hand_of("p1")[3], Some(cid(K, H)));
    // Nobody is told a face.
    assert!(!result.events.iter().any(|e| e.kind == EventType::Peeked));
}

#[test]
fn refuses_a_jack_swap_of_a_slot_with_itself() {
    let mut t = playing();
    discard_card(&mut t, cid(J, H));
    t.fail(blind_swap("p2", r("p2", 0), r("p2", 0)));
}

#[test]
fn refuses_a_jack_swap_into_an_empty_slot() {
    let mut t = playing();
    arrange(&mut t, ArrangeSpec::default().hand_slots("p1", &[Some(cid(A, S)), None, None, None]));
    discard_card(&mut t, cid(J, H));
    t.fail(blind_swap("p2", r("p2", 0), r("p1", 1)));
}

#[test]
fn queen_swaps_with_an_opponent_and_shows_what_you_received() {
    let mut t = playing();
    arrange(&mut t, ArrangeSpec::default().hand("p1", &[cid(Nine, S), cid(Two, D), cid(Three, C), cid(Four, S)]));
    let mine_before = t.hand_of("p2")[0];
    discard_card(&mut t, cid(Q, H));
    let result = t.ok(queen_swap("p2", 0, r("p1", 0)));
    assert_eq!(t.hand_of("p2")[0], Some(cid(Nine, S)));
    assert_eq!(t.hand_of("p1")[0], mine_before);
    let revealed: Vec<_> =
        private_to(&result.events, "p2").into_iter().filter(|e| e.kind == EventType::Revealed).collect();
    assert_eq!(revealed.len(), 1);
    assert_eq!(revealed[0].card.unwrap().rank, Nine);
}

#[test]
fn refuses_a_queen_swap_with_yourself() {
    let mut t = playing();
    discard_card(&mut t, cid(Q, H));
    t.fail(queen_swap("p2", 0, r("p2", 1)));
}

#[test]
fn king_peeks_two_then_decides_and_only_the_decider_sees_faces() {
    let mut t = playing();
    arrange(
        &mut t,
        ArrangeSpec::default()
            .hand("p1", &[cid(A, S), cid(Two, D), cid(Three, C), cid(Four, S)])
            .hand("p2", &[cid(K, S), cid(Six, S), cid(Seven, S), cid(Eight, S)]),
    );
    discard_card(&mut t, cid(K, H));
    let result = t.ok(king_peek("p2", r("p2", 0), r("p1", 0)));
    assert_eq!(peeked_to(&result.events, "p2"), 2);
    assert_eq!(peeked_to(&result.events, "p1"), 0);
    assert!(t.state.effect.as_ref().unwrap().awaiting_king_decision);
    assert_eq!(t.state.phase, Phase::Effect);

    t.ok(king_decide("p2", true));
    assert_eq!(t.hand_of("p2")[0], Some(cid(A, S)));
    assert_eq!(t.hand_of("p1")[0], Some(cid(K, S)));
    assert_eq!(t.turn_id(), "p1");
}

#[test]
fn king_can_decline_the_swap_after_looking() {
    let mut t = playing();
    arrange(&mut t, ArrangeSpec::default().hand("p2", &[cid(K, S), cid(Six, S), cid(Seven, S), cid(Eight, S)]));
    discard_card(&mut t, cid(K, H));
    t.ok(king_peek("p2", r("p2", 0), r("p2", 1)));
    t.ok(king_decide("p2", false));
    assert_eq!(t.hand_of("p2")[0], Some(cid(K, S)));
}

#[test]
fn requires_a_king_decision_before_the_turn_can_end() {
    let mut t = playing();
    discard_card(&mut t, cid(K, H));
    t.ok(king_peek("p2", r("p2", 0), r("p2", 1)));
    t.fail(discard("p2"));
    t.fail(peek_own("p2", 0));
}

#[test]
fn only_the_triggering_player_may_resolve_the_effect() {
    let mut t = playing();
    discard_card(&mut t, cid(Seven, H));
    t.fail(peek_own("p1", 0));
    t.fail(skip("p1"));
}

#[test]
fn lets_a_player_decline_their_own_effect() {
    let mut t = playing();
    discard_card(&mut t, cid(J, H));
    t.ok(skip("p2"));
    assert_eq!(t.turn_id(), "p1");
}

#[test]
fn never_triggers_an_effect_from_a_swap() {
    let mut t = playing();
    arrange(&mut t, ArrangeSpec::default().deck(&[cid(Seven, H)]));
    t.ok(draw("p2"));
    t.ok(swap("p2", 1));
    assert_eq!(t.state.phase, Phase::TurnStart);
    assert_eq!(t.turn_id(), "p1");
}

#[test]
fn never_triggers_an_effect_from_a_tap_discard() {
    // The tapped card is a 7 — a rank that WOULD open an effect phase if it had
    // been discarded during a turn. A tap must not.
    let mut t = playing();
    arrange(
        &mut t,
        ArrangeSpec::default()
            .hand("p1", &[cid(Seven, H), cid(Two, S), cid(Three, S), cid(Four, S)])
            .discard(&[cid(Seven, C)]),
    );
    let seq = t.state.discard_seq;
    t.ok(tap("p1", seq));
    t.ok(tap_choose("p1", r("p1", 0)));

    assert_eq!(t.hand_of("p1")[0], None);
    assert_eq!(top(&t), Some(cid(Seven, H)));
    assert_eq!(t.state.phase, Phase::TurnStart);
    assert!(t.state.effect.is_none());
}

// --- diagnostics -------------------------------------------------------------------

#[test]
fn renders_a_readable_label() {
    assert_eq!(card_label(Some(cid(K, H))), "KH");
    assert_eq!(card_label(Some(cid(Ten, D))), "10D");
    assert_eq!(card_label(Some(52)), "JOKER");
    assert_eq!(card_label(None), "--");
}

#[test]
fn counts_occupied_slots() {
    assert_eq!(occupied_count(&[Some(1), None, Some(3), None]), 2);
}
