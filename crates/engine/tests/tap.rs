use kaboo_engine::testkit::act::*;
use kaboo_engine::testkit::{ArrangeSpec, Table, arrange, arrange_short_deck, cid, hands, private_to};
use kaboo_engine::{EngineErrorCode, EventType, Phase, Rank, RuleConfig, Suit};

use Rank::*;
use Suit::*;

fn race() -> Table {
    let mut t = Table::players(3);
    t.finish_peek();
    arrange(
        &mut t,
        ArrangeSpec::default()
            .hand("p1", &[cid(Seven, H), cid(Two, S), cid(Three, S), cid(Four, S)])
            .hand("p3", &[cid(Seven, D), cid(Five, S), cid(Six, S), cid(Eight, S)])
            .discard(&[cid(Seven, C)]),
    );
    t
}

fn claim(t: &mut Table, who: &str) {
    let seq = t.state.discard_seq;
    t.ok(tap(who, seq));
}

fn tapper(t: &Table) -> String {
    t.state.tap.as_ref().expect("a claimed tap").player_id.clone()
}

// --- the race --------------------------------------------------------------------

#[test]
fn lets_any_player_tap_out_of_turn_when_a_card_matches() {
    let mut t = race();
    assert_eq!(t.turn_id(), "p2");
    let seq = t.state.discard_seq;
    let result = t.ok(tap("p1", seq));
    assert_eq!(tapper(&t), "p1");
    assert!(result.events.iter().any(|e| e.kind == EventType::TapClaimed));
}

#[test]
fn gives_the_race_to_whoever_commits_first_and_tells_the_loser_too_late() {
    let mut t = race();
    claim(&mut t, "p1");
    let seq = t.state.discard_seq;
    let loser = t.fail(tap("p3", seq));
    assert_eq!(loser.error.unwrap().code, EngineErrorCode::TooLate);
    assert_eq!(tapper(&t), "p1");
}

#[test]
fn produces_one_winner_regardless_of_arrival_order() {
    // The two orderings a real race can produce once the room has serialised
    // them: only the first claimant ever holds the tap.
    let mut first = Table::players(3);
    first.finish_peek();
    arrange(
        &mut first,
        ArrangeSpec::default()
            .hand("p1", &[cid(Seven, H), cid(Two, S), cid(Three, S), cid(Four, S)])
            .discard(&[cid(Seven, C)]),
    );
    claim(&mut first, "p1");
    let seq = first.state.discard_seq;
    first.fail(tap("p3", seq));

    let mut second = Table::players(3);
    second.finish_peek();
    arrange(
        &mut second,
        ArrangeSpec::default()
            .hand("p3", &[cid(Seven, D), cid(Five, S), cid(Six, S), cid(Eight, S)])
            .discard(&[cid(Seven, C)]),
    );
    claim(&mut second, "p3");
    let seq = second.state.discard_seq;
    second.fail(tap("p1", seq));

    assert_eq!(tapper(&first), "p1");
    assert_eq!(tapper(&second), "p3");
}

#[test]
fn a_non_matching_tap_is_a_wrong_tap_not_an_error() {
    let mut t = race();
    arrange(&mut t, ArrangeSpec::default().hand("p1", &[cid(A, H), cid(Two, S), cid(Three, S), cid(Four, S)]));
    claim(&mut t, "p1");
    let result = t.ok(tap_choose("p1", r("p1", 0)));
    assert!(result.error.is_none());
    assert!(result.events.iter().any(|e| e.kind == EventType::TapWrong));
}

#[test]
fn refuses_a_tap_on_a_stale_view_of_the_discard() {
    let mut t = race();
    let stale = t.state.discard_seq;
    arrange(&mut t, ArrangeSpec::default().discard(&[cid(Nine, C)]));
    let result = t.fail(tap("p1", stale));
    assert_eq!(result.error.unwrap().code, EngineErrorCode::Stale);
}

#[test]
fn refuses_tapping_when_the_discard_pile_is_empty() {
    let mut t = race();
    arrange(&mut t, ArrangeSpec::default().discard(&[]));
    let seq = t.state.discard_seq;
    t.fail(tap("p1", seq));
}

#[test]
fn moves_the_tapped_card_to_the_discard_and_empties_its_slot() {
    let mut t = race();
    claim(&mut t, "p1");
    t.ok(tap_choose("p1", r("p3", 0)));
    assert_eq!(t.hand_of("p3")[0], None);
    assert_eq!(t.state.discard.last().copied(), Some(cid(Seven, D)));
}

#[test]
fn lets_the_tapper_drop_one_of_their_own_cards_into_the_hole() {
    let mut t = race();
    claim(&mut t, "p1");
    t.ok(tap_choose("p1", r("p3", 0)));
    assert!(t.state.tap.as_ref().unwrap().awaiting_placement);

    t.ok(tap_place("p1", 2));
    assert_eq!(t.hand_of("p3")[0], Some(cid(Three, S)), "p1's card now fills p3's slot");
    assert_eq!(t.hand_of("p1")[2], None, "and p1 no longer holds it");
    assert!(t.state.tap.is_none());
}

#[test]
fn lets_the_tapper_decline_to_place() {
    let mut t = race();
    claim(&mut t, "p1");
    t.ok(tap_choose("p1", r("p3", 0)));
    t.ok(tap_decline("p1"));
    assert_eq!(t.hand_of("p3")[0], None);
    assert_eq!(t.hand_of("p1")[2], Some(cid(Three, S)));
    assert!(t.state.tap.is_none());
}

#[test]
fn does_not_offer_placement_after_tapping_your_own_card() {
    let mut t = race();
    claim(&mut t, "p1");
    t.ok(tap_choose("p1", r("p1", 0)));
    assert!(t.state.tap.is_none());
    t.fail(tap_place("p1", 1));
}

#[test]
fn blocks_turn_actions_while_a_tap_is_claimed() {
    let mut t = race();
    claim(&mut t, "p1");
    t.fail(draw("p2"));
    t.fail(take("p2"));
}

#[test]
fn resumes_the_interrupted_turn_once_the_tap_resolves() {
    let mut t = race();
    claim(&mut t, "p1");
    t.ok(tap_choose("p1", r("p1", 0)));
    assert_eq!(t.state.phase, Phase::TurnStart);
    assert_eq!(t.turn_id(), "p2");
    t.ok(draw("p2"));
}

#[test]
fn stops_anyone_tapping_during_the_deck_out_decision() {
    let mut solo = Table::players(2);
    solo.finish_peek();
    arrange_short_deck(
        &mut solo,
        hands(&[
            ("p1", &[cid(Seven, H), cid(Two, S), cid(Three, S), cid(Four, S)]),
            ("p2", &[cid(Five, H), cid(Six, S), cid(Nine, S), cid(J, S)]),
        ]),
        &[cid(A, C)],
        &[cid(Seven, C)],
    );
    solo.ok(draw("p2"));
    assert_eq!(solo.state.phase, Phase::FinalSwap);
    assert!(solo.state.deck.is_empty());
    let seq = solo.state.discard_seq;
    solo.fail(tap("p1", seq));
}

// --- wrong-tap penalty -------------------------------------------------------------

fn penalty_table() -> Table {
    let mut t = Table::players(2);
    t.finish_peek();
    arrange(
        &mut t,
        ArrangeSpec::default()
            .hand("p1", &[cid(A, H), cid(Two, S), cid(Three, S), cid(Four, S)])
            .deck(&[cid(Nine, D), cid(Ten, D), cid(J, D)])
            .discard(&[cid(Seven, C)]),
    );
    t
}

fn wrong_tap(t: &mut Table) -> kaboo_engine::ApplyResult {
    claim(t, "p1");
    t.ok(tap_choose("p1", r("p1", 0)))
}

#[test]
fn draws_a_blind_penalty_card_into_the_tappers_hand() {
    let mut t = penalty_table();
    let before = t.hand_of("p1").len();
    wrong_tap(&mut t);
    assert_eq!(t.hand_of("p1").len(), before + 1);
    assert_eq!(t.hand_of("p1")[before], Some(cid(Nine, D)));
}

#[test]
fn leaves_the_named_card_exactly_where_it_was() {
    let mut t = penalty_table();
    wrong_tap(&mut t);
    assert_eq!(t.hand_of("p1")[0], Some(cid(A, H)));
}

#[test]
fn never_hands_the_penalty_card_to_anyone_but_the_tapper() {
    let mut t = penalty_table();
    let before = t.hand_of("p2").len();
    wrong_tap(&mut t);
    assert_eq!(t.hand_of("p2").len(), before);
}

#[test]
fn does_not_tell_the_tapper_what_the_penalty_card_is() {
    let mut t = penalty_table();
    let result = wrong_tap(&mut t);
    assert!(!private_to(&result.events, "p1").iter().any(|e| e.kind == EventType::Peeked));
    assert!(result.events.iter().all(|e| e.card.is_none()));
}

#[test]
fn frees_the_tap_so_somebody_else_may_try() {
    let mut t = penalty_table();
    wrong_tap(&mut t);
    assert!(t.state.tap.is_none());
    let seq = t.state.discard_seq;
    assert!(t.ok(tap("p2", seq)).error.is_none());
}

#[test]
fn charges_a_penalty_card_when_a_claimed_tap_times_out() {
    let mut t = penalty_table();
    claim(&mut t, "p1");
    let before = t.hand_of("p1").len();
    t.advance(RuleConfig::default().tap_deadline_ms + 1);
    let result = t.send(ready("p1")); // any request drives the clock
    assert!(result.version_changed);
    assert!(t.state.tap.is_none());
    assert_eq!(t.hand_of("p1").len(), before + 1);
}

#[test]
fn cancels_a_timed_out_tap_without_penalty_when_the_rule_is_off() {
    let mut t = Table::with_rules(2, RuleConfig { tap_timeout_is_wrong_tap: false, ..Default::default() });
    t.finish_peek();
    arrange(&mut t, ArrangeSpec::default().discard(&[cid(Seven, C)]).deck(&[cid(Nine, D), cid(Ten, D)]));
    claim(&mut t, "p1");
    let before = t.hand_of("p1").len();
    t.advance(RuleConfig::default().tap_deadline_ms + 1);
    t.send(ready("p1"));
    assert!(t.state.tap.is_none());
    assert_eq!(t.hand_of("p1").len(), before);
}

#[test]
fn tick_alone_resolves_an_abandoned_tap() {
    // New with the alarm: nobody sends anything, and the deadline still fires.
    let mut t = penalty_table();
    claim(&mut t, "p1");
    t.advance(RuleConfig::default().tap_deadline_ms - 1);
    assert!(t.tick().is_none(), "nothing is due yet");
    t.advance(2);
    assert!(t.tick().is_some());
    assert!(t.state.tap.is_none());
    assert_eq!(t.hand_of("p1").len(), 5);
}
