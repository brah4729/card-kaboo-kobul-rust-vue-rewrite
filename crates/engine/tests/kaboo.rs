use kaboo_engine::testkit::act::*;
use kaboo_engine::testkit::{ArrangeSpec, Table, accounting_errors, arrange, arrange_short_deck, cid, hands};
use kaboo_engine::{
    DrawSource, EndCondition, EngineErrorCode, EventType, KabooPenaltyMode, Phase, Rank, RuleConfig, Suit,
};

use Rank::*;
use Suit::*;

fn four_queens() -> [u8; 4] {
    [cid(Q, S), cid(Q, H), cid(Q, D), cid(Q, C)]
}

fn playing(n: usize, rules: RuleConfig) -> Table {
    let mut t = Table::with_rules(n, rules);
    t.finish_peek();
    t
}

fn two() -> Table {
    playing(2, RuleConfig::default())
}

fn player<'a>(t: &'a Table, id: &str) -> &'a kaboo_engine::PlayerState {
    t.state.players.iter().find(|p| p.id == id).unwrap()
}

fn half(cards: &[u8]) -> Vec<Option<u8>> {
    let mut v: Vec<Option<u8>> = cards.iter().map(|c| Some(*c)).collect();
    v.resize(4, None);
    v
}

// --- KABOO -------------------------------------------------------------------------

#[test]
fn may_only_be_called_at_the_start_of_your_own_turn() {
    let mut t = two();
    t.fail(kaboo("p1")); // not p1's turn
    t.ok(draw("p2"));
    t.fail(kaboo("p2")); // already drawn
}

#[test]
fn ends_the_callers_turn_immediately() {
    let mut t = two();
    let before = t.state.deck.len();
    t.ok(kaboo("p2"));
    assert_eq!(t.state.deck.len(), before);
    assert!(t.state.drawn.is_none());
}

#[test]
fn gives_every_other_player_exactly_one_final_turn_then_reveals() {
    let mut t = two();
    t.ok(kaboo("p2"));
    assert_eq!(t.state.kaboo_caller_id.as_deref(), Some("p2"));
    assert_eq!(t.state.final_turns_remaining, vec!["p1"]);
    t.ok(draw("p1"));
    t.ok(discard("p1"));
    assert_eq!(t.state.phase, Phase::Reveal);
}

#[test]
fn marks_the_caller_as_having_had_their_final_turn() {
    let mut t = two();
    t.ok(kaboo("p2"));
    assert!(player(&t, "p2").final_turn_done);
    assert!(!player(&t, "p1").final_turn_done);
}

#[test]
fn cycles_every_other_seat_in_order_and_never_returns_to_the_caller() {
    let mut four = playing(4, RuleConfig::default());
    // Low cards, so no discard triggers an effect and final turns follow directly.
    arrange(&mut four, ArrangeSpec::default().deck(&[cid(Two, S), cid(Three, S), cid(Four, S)]));
    four.ok(kaboo("p2"));
    let mut remaining = four.state.final_turns_remaining.clone();
    remaining.sort();
    assert_eq!(remaining, vec!["p1", "p3", "p4"]);

    let mut order = Vec::new();
    for _ in 0..3 {
        let id = four.turn_id();
        assert_ne!(id, "p2");
        order.push(id.clone());
        four.ok(draw(&id));
        four.ok(discard(&id));
    }
    assert_eq!(order, vec!["p3", "p4", "p1"]);
    assert_eq!(four.state.phase, Phase::Reveal);
}

#[test]
fn refuses_a_second_kaboo_call_in_the_same_round() {
    let mut t = two();
    t.ok(kaboo("p2"));
    t.fail(kaboo("p1"));
}

#[test]
fn can_be_countered_and_the_counter_is_scored_as_a_caller() {
    let mut t = two();
    arrange(
        &mut t,
        ArrangeSpec::default()
            .hand("p1", &four_queens()) // 40 — the counter-caller, a certain loser
            .hand("p2", &[cid(A, S), cid(A, H), cid(A, D), cid(A, C)]) // 4
            .deck(&[cid(Nine, C)]),
    );
    t.ok(kaboo("p2"));
    t.ok(counter("p1"));
    assert_eq!(t.state.phase, Phase::Reveal, "skipping consumed p1's final turn");
    assert!(player(&t, "p1").counter_kaboo);
    let result = t.state.round_result.clone().unwrap();
    assert_eq!(result.winner_id, "p2");
    assert_eq!(result.penalty_ids, vec!["p1"]);
    assert_eq!(result.awarded["p1"], 40 + 20);
}

#[test]
fn refuses_a_counter_when_the_rule_is_off() {
    let mut off = playing(2, RuleConfig { counter_kaboo: false, ..Default::default() });
    off.ok(kaboo("p2"));
    off.fail(counter("p1"));
}

#[test]
fn refuses_a_counter_before_anyone_has_called() {
    let mut t = two();
    t.fail(counter("p2"));
}

#[test]
fn auto_kaboos_the_player_who_loses_their_last_card() {
    let mut t = two();
    arrange(
        &mut t,
        ArrangeSpec::default()
            .hand_slots("p1", &half(&[cid(Seven, H)]))
            .hand("p2", &[cid(Five, S), cid(Six, S), cid(Eight, S), cid(Nine, S)])
            .discard(&[cid(Seven, C)]),
    );
    let seq = t.state.discard_seq;
    t.ok(tap("p1", seq));
    t.ok(tap_choose("p1", r("p1", 0)));

    assert!(t.hand_of("p1").iter().all(Option::is_none));
    assert_eq!(t.state.kaboo_caller_id.as_deref(), Some("p1"));
    assert_eq!(t.state.final_turns_remaining, vec!["p2"]);

    t.ok(draw("p2"));
    t.ok(discard("p2"));
    assert_eq!(t.state.phase, Phase::Reveal);
}

#[test]
fn does_not_auto_kaboo_again_once_a_call_is_live() {
    let mut t = two();
    arrange(
        &mut t,
        ArrangeSpec::default()
            .hand_slots("p1", &half(&[cid(Seven, H)]))
            .hand("p2", &[cid(Five, S), cid(Six, S), cid(Eight, S), cid(Nine, S)])
            .discard(&[cid(Seven, C)]),
    );
    t.ok(kaboo("p2"));
    let seq = t.state.discard_seq;
    t.ok(tap("p1", seq));
    t.ok(tap_choose("p1", r("p1", 0)));
    assert_eq!(t.state.kaboo_caller_id.as_deref(), Some("p2"));
    assert_eq!(t.state.final_turns_remaining, vec!["p1"]);
}

// --- scoring -----------------------------------------------------------------------

/// p2 calls, p1 plays one final low card, the round reveals.
fn call_and_finish(t: &mut Table) {
    t.ok(kaboo("p2"));
    t.ok(draw("p1"));
    t.ok(discard("p1"));
}

fn two_hands(rules: RuleConfig, p1: &[u8], p2: &[u8]) -> Table {
    let mut t = playing(2, rules);
    arrange(
        &mut t,
        ArrangeSpec::default().hand_slots("p1", &half(p1)).hand_slots("p2", &half(p2)).deck(&[cid(Two, C)]),
    );
    t
}

#[test]
fn scores_the_round_winner_zero_and_everyone_else_their_values() {
    let mut t = playing(3, RuleConfig::default());
    arrange(
        &mut t,
        ArrangeSpec::default()
            .hand_slots("p1", &half(&[cid(A, S), cid(A, H)])) // 2
            .hand_slots("p2", &half(&[cid(Five, S), cid(Six, D)])) // 11
            .hand_slots("p3", &half(&[cid(Nine, C), cid(Nine, D)])) // 18
            .deck(&[cid(Two, C)]),
    );
    t.ok(kaboo("p2"));
    t.ok(counter("p3"));
    t.ok(draw("p1"));
    t.ok(discard("p1"));

    let result = t.state.round_result.clone().unwrap();
    assert_eq!(
        result.totals.into_iter().collect::<Vec<_>>(),
        vec![("p1".into(), 2), ("p2".into(), 11), ("p3".into(), 18)]
    );
    assert_eq!(result.winner_id, "p1");
    assert_eq!(result.awarded["p1"], 0);
}

#[test]
fn penalises_a_caller_who_did_not_win_on_top_of_their_hand() {
    let mut t = two_hands(RuleConfig::default(), &[cid(A, S), cid(A, H)], &[cid(Five, S), cid(Six, D)]);
    call_and_finish(&mut t);
    let result = t.state.round_result.clone().unwrap();
    assert_eq!(result.winner_id, "p1");
    assert_eq!(result.penalty_ids, vec!["p2"]);
    assert_eq!(result.awarded["p1"], 0);
    assert_eq!(result.awarded["p2"], 11 + 20);
}

#[test]
fn charges_a_flat_penalty_when_the_rule_says_flat() {
    let rules = RuleConfig { kaboo_penalty_mode: KabooPenaltyMode::Flat, ..Default::default() };
    let mut t = two_hands(rules, &[cid(A, S), cid(A, H)], &[cid(Five, S), cid(Six, D)]);
    call_and_finish(&mut t);
    assert_eq!(t.state.round_result.as_ref().unwrap().awarded["p2"], 20);
}

#[test]
fn gives_a_caller_who_did_win_a_clean_zero() {
    let mut t = two_hands(RuleConfig::default(), &[cid(Five, S), cid(Six, D)], &[cid(A, S), cid(A, H)]);
    call_and_finish(&mut t);
    let result = t.state.round_result.clone().unwrap();
    assert_eq!(result.winner_id, "p2");
    assert!(result.penalty_ids.is_empty());
    assert_eq!(result.awarded["p2"], 0);
}

#[test]
fn scores_both_red_kings_as_minus_one() {
    let mut t = two_hands(RuleConfig::default(), &[cid(K, H), cid(K, D)], &[cid(A, S), cid(A, H)]);
    call_and_finish(&mut t);
    let result = t.state.round_result.clone().unwrap();
    assert_eq!(result.totals["p1"], -2);
    assert_eq!(result.winner_id, "p1");
}

#[test]
fn breaks_a_tie_in_favour_of_the_seat_nearest_the_dealer_walking_left() {
    let mut t = playing(3, RuleConfig::default()); // dealer seat 0, so p2 plays first
    arrange(
        &mut t,
        ArrangeSpec::default()
            .hand_slots("p1", &half(&[cid(A, S), cid(A, H)]))
            .hand_slots("p2", &half(&[cid(A, D), cid(A, C)]))
            .hand_slots("p3", &half(&[cid(Nine, C), cid(Nine, D)]))
            .deck(&[cid(Two, C)]),
    );
    t.ok(kaboo("p2"));
    t.ok(counter("p3"));
    t.ok(draw("p1"));
    t.ok(discard("p1"));
    let result = t.state.round_result.clone().unwrap();
    assert_eq!(result.totals["p1"], result.totals["p2"]);
    assert_eq!(result.winner_id, "p2", "seat 1 is immediately left of dealer seat 0");
}

#[test]
fn folds_the_round_into_the_cumulative_score_and_deals_the_next() {
    let rules = RuleConfig { rounds: 3, ..Default::default() };
    let mut t = two_hands(rules, &[cid(A, S), cid(A, H)], &[cid(Five, S), cid(Six, D)]);
    call_and_finish(&mut t);
    assert_eq!(player(&t, "p2").score, 31);
    t.ok(cont("p1"));
    assert_eq!(t.state.round, 2);
    assert_eq!(t.state.phase, Phase::Peek);
    assert_eq!(player(&t, "p2").score, 31);
}

#[test]
fn ends_after_the_configured_number_of_rounds() {
    let rules = RuleConfig { rounds: 1, ..Default::default() };
    let mut t = two_hands(rules, &[cid(A, S), cid(A, H)], &[cid(Five, S), cid(Six, D)]);
    call_and_finish(&mut t);
    t.ok(cont("p1"));
    assert_eq!(t.state.phase, Phase::GameOver);
    let result = t.state.game_result.clone().unwrap();
    assert_eq!(result.winner_id, "p1");
    assert_eq!(result.reason, EndCondition::FixedRounds);
}

#[test]
fn supports_a_score_threshold_ending() {
    let rules = RuleConfig { end_condition: EndCondition::ScoreThreshold, score_limit: 10, ..Default::default() };
    let mut t = two_hands(rules, &[cid(A, S), cid(A, H)], &[cid(Nine, S), cid(Nine, D)]);
    call_and_finish(&mut t);
    t.ok(cont("p1")); // p2 is on 18 + 20 = 38, past the limit
    assert_eq!(t.state.phase, Phase::GameOver);
    assert_eq!(t.state.game_result.as_ref().unwrap().reason, EndCondition::ScoreThreshold);
}

#[test]
fn refuses_every_action_once_the_game_is_over() {
    let mut t = playing(2, RuleConfig { rounds: 1, ..Default::default() });
    call_and_finish(&mut t);
    t.ok(cont("p1"));
    assert_eq!(t.state.phase, Phase::GameOver);
    let result = t.fail(draw("p1"));
    assert_eq!(result.error.unwrap().code, EngineErrorCode::GameOver);
}

// --- deck out (house rule) -----------------------------------------------------------

/// A table whose draw pile holds exactly one card: the red king of hearts.
fn deck_out_table() -> Table {
    let mut t = two();
    arrange_short_deck(
        &mut t,
        hands(&[
            ("p1", &[cid(Five, S), cid(Six, D), cid(Eight, C), cid(Nine, S)]), // 28
            ("p2", &[cid(A, S), cid(Two, D), cid(Three, C), cid(Four, S)]),    // 10
        ]),
        &[cid(K, H)],
        &[cid(Seven, D)],
    );
    t
}

#[test]
fn ends_the_round_the_moment_the_last_card_is_drawn() {
    let mut t = deck_out_table();
    assert_eq!(t.state.deck.len(), 1);
    t.ok(draw("p2"));
    assert_eq!(t.state.phase, Phase::FinalSwap);
    assert_eq!(t.state.held_card_id, Some(cid(K, H)));
    assert!(t.state.drawn.is_none());
}

#[test]
fn shows_the_last_card_only_to_the_player_who_drew_it() {
    let mut t = deck_out_table();
    let result = t.ok(draw("p2"));
    let theirs: Vec<_> =
        result.events.iter().filter(|e| e.to.as_deref() == Some("p2") && e.kind == EventType::DeckOut).collect();
    assert_eq!(theirs.len(), 1);
    assert_eq!(theirs[0].card.unwrap().rank, K);
    let public: Vec<_> = result.events.iter().filter(|e| e.to.is_none() && e.kind == EventType::DeckOut).collect();
    assert_eq!(public.len(), 1);
    assert!(public[0].card.is_none());
}

#[test]
fn lets_the_drawer_swap_the_last_card_in_and_scores_it() {
    let mut t = deck_out_table();
    t.ok(draw("p2"));
    t.ok(final_swap("p2", 3));
    assert_eq!(t.hand_of("p2")[3], Some(cid(K, H)));
    // The replaced card goes to the discard pile, so no card id disappears.
    assert_eq!(t.state.discard.last().copied(), Some(cid(Four, S)));
    assert!(t.state.set_aside.is_none());
    assert_eq!(t.state.phase, Phase::Reveal);
    // A swapped-in red king counts toward the drawer's score: 1 + 2 + 3 - 1.
    assert_eq!(t.state.round_result.as_ref().unwrap().totals["p2"], 5);
}

#[test]
fn sets_the_last_card_aside_unscored_when_the_drawer_declines() {
    let mut t = deck_out_table();
    t.ok(draw("p2"));
    t.ok(final_decline("p2"));
    assert_eq!(t.state.set_aside, Some(cid(K, H)));
    assert_eq!(t.hand_of("p2"), half(&[cid(A, S), cid(Two, D), cid(Three, C), cid(Four, S)]));
    assert_eq!(t.state.round_result.as_ref().unwrap().totals["p2"], 10);
    assert_eq!(t.state.phase, Phase::Reveal);
}

#[test]
fn auto_declines_when_the_drawer_never_answers() {
    let mut t = deck_out_table();
    t.ok(draw("p2"));
    t.advance(RuleConfig::default().final_swap_deadline_ms + 1);
    t.send(ready("p1")); // any request drives the clock
    assert_eq!(t.state.set_aside, Some(cid(K, H)));
    assert!(t.state.held_card_id.is_none());
    assert_eq!(t.state.phase, Phase::Reveal);
}

#[test]
fn offers_no_discard_no_effect_and_no_tap_during_the_decision() {
    let mut t = deck_out_table();
    t.ok(draw("p2"));
    t.fail(discard("p2"));
    let seq = t.state.discard_seq;
    t.fail(tap("p1", seq));
    assert_eq!(t.state.phase, Phase::FinalSwap);
}

#[test]
fn lets_only_the_drawer_decide() {
    let mut t = deck_out_table();
    t.ok(draw("p2"));
    t.fail(final_decline("p1"));
    t.fail(final_swap("p1", 0));
}

#[test]
fn ends_the_round_with_no_swap_when_a_wrong_tap_penalty_takes_the_last_card() {
    let mut t = two();
    arrange_short_deck(
        &mut t,
        hands(&[
            // Nothing in p1's hand matches the 7 on the discard.
            ("p1", &[cid(A, S), cid(Two, D), cid(Three, C), cid(Four, S)]),
            ("p2", &[cid(Five, S), cid(Six, D), cid(Eight, C), cid(Nine, S)]),
        ]),
        &[cid(K, H)],
        &[cid(Seven, D)],
    );
    let seq = t.state.discard_seq;
    t.ok(tap("p1", seq));
    t.ok(tap_choose("p1", r("p1", 0)));

    let hand = t.hand_of("p1");
    assert_eq!(hand.len(), 5, "the penalty card always lands in the hand");
    assert_eq!(hand[4], Some(cid(K, H)));
    assert_eq!(t.state.phase, Phase::Reveal);
    assert!(t.state.held_card_id.is_none());
    assert!(t.state.set_aside.is_none());
}

#[test]
fn never_reshuffles_the_discard_back_into_the_draw_pile() {
    let mut t = deck_out_table();
    let before = t.state.discard.len();
    t.ok(draw("p2"));
    t.ok(final_decline("p2"));
    assert!(t.state.deck.is_empty());
    assert_eq!(t.state.discard.len(), before);
}

/// Regression (fuzz seed 1050406591): a wrong-tap penalty drawing the LAST card
/// ends the round while the turn player is mid-draw; that card must not vanish.
#[test]
fn keeps_the_turn_players_drawn_card_when_a_wrong_tap_penalty_ends_the_round() {
    let mut t = two();
    arrange_short_deck(
        &mut t,
        hands(&[
            ("p1", &[cid(Five, S), cid(Six, D), cid(Eight, C), cid(Nine, S)]), // nothing matches the 7
            ("p2", &[cid(A, S), cid(Two, D), cid(Three, C), cid(Four, S)]),
        ]),
        &[cid(K, H), cid(Q, H)], // p2 draws the king; the queen is the last card
        &[cid(Seven, D)],
    );
    t.ok(draw("p2"));
    assert_eq!(t.state.deck.len(), 1, "one card left, so not a deck-out yet");

    let seq = t.state.discard_seq;
    t.ok(tap("p1", seq));
    t.ok(tap_choose("p1", r("p1", 0)));

    assert_eq!(t.state.phase, Phase::Reveal);
    assert!(accounting_errors(&t.state, 52).is_empty(), "{:?}", accounting_errors(&t.state, 52));
    assert_eq!(t.hand_of("p2")[4], Some(cid(K, H)));
    assert_eq!(t.state.round_result.as_ref().unwrap().totals["p2"], 1 + 2 + 3 + 4 - 1);
}

#[test]
fn keeps_a_drawn_card_taken_from_the_discard_when_the_round_is_cut_short() {
    let mut t = two();
    arrange_short_deck(
        &mut t,
        hands(&[
            ("p1", &[cid(Five, S), cid(Six, D), cid(Eight, C), cid(Nine, S)]),
            ("p2", &[cid(A, S), cid(Two, D), cid(Three, C), cid(Four, S)]),
        ]),
        &[cid(Q, H)],     // the last card of the pile
        &[cid(Seven, D)], // p2 takes this one
    );
    t.ok(take("p2"));
    assert_eq!(t.state.drawn.as_ref().unwrap().from, DrawSource::Discard);

    let seq = t.state.discard_seq;
    t.ok(tap("p1", seq));
    t.ok(tap_choose("p1", r("p1", 0)));

    assert_eq!(t.state.phase, Phase::Reveal);
    assert!(accounting_errors(&t.state, 52).is_empty());
    assert_eq!(t.hand_of("p2")[4], Some(cid(Seven, D)));
}
