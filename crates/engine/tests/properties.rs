//! PROPERTY TESTS: random games, driven by a seeded RNG, asserted against the
//! three invariants the brief names — card conservation, no hidden-card
//! leakage, and termination.
//!
//! The generator's own seed is PINNED (`FC_SEED`), so a red build is
//! reproducible and a reported counterexample replays exactly. The seed in a
//! failure message is all you need: paste it into the regression block below.

use std::collections::BTreeSet;

use proptest::prelude::*;
use proptest::test_runner::{Config, RngSeed};

use kaboo_engine::testkit::act::*;
use kaboo_engine::testkit::{
    MAX_STEPS, Table, TableOptions, accounting_errors, leaked_card_ids, mulberry32, random_action,
};
use kaboo_engine::{Phase, RuleConfig};

/// Pinned generator seed: same cases, same order, every run.
const FC_SEED: u64 = 20_261_015;

fn config(cases: u32) -> Config {
    Config { cases, rng_seed: RngSeed::Fixed(FC_SEED), failure_persistence: None, ..Config::default() }
}

fn finished(phase: Phase) -> bool {
    matches!(phase, Phase::Reveal | Phase::GameOver)
}

/// Play one round at random, checking every invariant after every single
/// action. Returns the phase the round finished in.
fn fuzz_round(seed: u32, players: usize, jokers: bool) -> Phase {
    let mut rng = mulberry32(seed);
    let mut t = Table::new(TableOptions {
        players: Some(players),
        seed: Some(seed ^ 0x5f37_59df),
        rules: Some(RuleConfig { jokers, ..Default::default() }),
    });
    let total = t.total_cards();
    let at = format!("seed {seed}, {players} players, jokers={jokers}");

    let check = |t: &Table, step: usize, context: &str| {
        let errors = accounting_errors(&t.state, total);
        assert!(errors.is_empty(), "{at} (step {step}): card accounting after {context}: {errors:?}");
        for p in &t.state.players {
            let leaked = leaked_card_ids(&t.state, &p.id);
            assert!(leaked.is_empty(), "{at} (step {step}): {} can see hidden cards {leaked:?} after {context}", p.id);
        }
    };

    check(&t, 0, "the deal");
    let mut steps = 0;
    let mut seen = BTreeSet::new();
    while !finished(t.state.phase) && steps < MAX_STEPS {
        // Occasionally let time pass, so the deadline paths get exercised.
        if rng() < 0.08 {
            t.advance((rng() * 90_000.0).floor() as i64);
        }
        let action = random_action(&mut rng, &t.state, &t.ids.clone());
        let context = match &action {
            Some(a) => serde_json::to_string(a).unwrap(),
            None => "a clock tick".into(),
        };
        match action {
            Some(a) => {
                t.send(a);
            }
            None => t.advance(5_000), // nothing legal to try: just move the clock
        }

        steps += 1;
        seen.insert(format!("{:?}", t.state.phase));
        if t.state.kaboo_caller_id.is_some() {
            seen.insert("kabooLive".into());
        }
        if t.state.tap.is_some() {
            seen.insert("tapLive".into());
        }
        if t.state.effect.is_some() {
            seen.insert("effectLive".into());
        }
        if t.state.held_card_id.is_some() {
            seen.insert("deckOut".into());
        }
        check(&t, steps, &context);
    }
    if std::env::var_os("FUZZ_REPORT").is_some() {
        println!("  {at}: {steps} steps, ended in {:?}, saw {seen:?}", t.state.phase);
    }
    assert!(finished(t.state.phase), "{at}: the round did not terminate within {MAX_STEPS} actions");
    t.state.phase
}

proptest! {
    #![proptest_config(config(200))]

    /// A round always terminates in random play, at every table size, with and
    /// without jokers — and every step keeps every card and leaks none.
    #[test]
    fn a_round_always_terminates(seed in 1u32..i32::MAX as u32, players in 2usize..=4, jokers: bool) {
        fuzz_round(seed, players, jokers);
    }

    /// Every card id is accounted for exactly once after dealing.
    #[test]
    fn the_deal_accounts_for_every_card(seed in 1u32..i32::MAX as u32, players in 2usize..=8, jokers: bool) {
        let t = Table::new(TableOptions {
            players: Some(players),
            seed: Some(seed),
            rules: Some(RuleConfig { jokers, ..Default::default() }),
        });
        let total = t.total_cards();
        prop_assert!(accounting_errors(&t.state, total).is_empty(), "seed {seed}, {players} players");
        let hand_size = t.state.rules.hand_size;
        for p in &t.state.players {
            prop_assert_eq!(p.hand.len(), hand_size);
            prop_assert!(p.hand.iter().all(Option::is_some));
        }
        prop_assert_eq!(t.state.deck.len(), total - players * hand_size);
    }

    /// Every player looking at their two cards leaves every card accounted for.
    #[test]
    fn peeking_accounts_for_every_card(seed in 1u32..i32::MAX as u32, players in 2usize..=6) {
        let mut t = Table::new(TableOptions { players: Some(players), seed: Some(seed), ..Default::default() });
        for id in t.ids.clone() {
            t.send(peek(&id, 2));
            prop_assert!(accounting_errors(&t.state, 52).is_empty());
            t.send(ready(&id));
            prop_assert!(accounting_errors(&t.state, 52).is_empty());
        }
        prop_assert_eq!(t.state.phase, Phase::TurnStart);
    }

    /// The first turn of a fresh round leaks nothing, for every seat.
    #[test]
    fn the_first_turn_leaks_nothing(seed in 1u32..i32::MAX as u32, players in 2usize..=8, slot in 0usize..=3) {
        let mut t = Table::new(TableOptions { players: Some(players), seed: Some(seed), ..Default::default() });
        for id in t.ids.clone() {
            t.send(peek(&id, 2));
            t.send(peek(&id, 3));
            t.send(ready(&id));
        }
        let turn = t.turn_id();
        t.send(draw(&turn));
        if slot % 2 == 0 {
            t.send(discard(&turn));
        } else {
            t.send(swap(&turn, slot));
        }
        for p in &t.state.players {
            prop_assert!(leaked_card_ids(&t.state, &p.id).is_empty(), "seed {seed}, {players} players");
        }
    }
}

proptest! {
    #![proptest_config(config(150))]

    /// No projection leaks a hidden card at any point of a round, for any viewer.
    #[test]
    fn no_projection_leaks_at_any_point(seed in 1u32..i32::MAX as u32, players in 2usize..=4) {
        fuzz_round(seed, players, false);
    }
}

/// With nobody acting at all, the deadlines alone (the room's alarm) must
/// finish the round — a table where every player is gone still ends.
#[test]
fn a_silent_table_finishes_from_deadlines_alone() {
    for players in [2, 3, 8] {
        let mut t = Table::players(players);
        let mut guard = 0;
        while !finished(t.state.phase) && guard < 5000 {
            t.advance(120_000); // far past every deadline
            t.tick();
            guard += 1;
        }
        assert_eq!(t.state.phase, Phase::Reveal, "{players} silent players never finished the round");
        assert!(accounting_errors(&t.state, 52).is_empty());
    }
}

/// `next_deadline` must never sleep past a deadline `tick` would act on: at the
/// instant it names, a tick does something (or the round has already ended).
#[test]
fn next_deadline_is_never_late() {
    for players in [2, 4] {
        let mut t = Table::players(players);
        let mut guard = 0;
        while !finished(t.state.phase) && guard < 5000 {
            let due = kaboo_engine::next_deadline(&t.state).expect("a live round always has a deadline");
            t.now = due;
            assert!(t.tick().is_some(), "{players} players: nothing happened at the announced deadline {due}");
            guard += 1;
        }
        assert!(finished(t.state.phase));
    }
}

// --- regression: seeds that once broke an invariant ----------------------------------

/// Step 118: a wrong tap drew the LAST card of the pile as its penalty, which
/// ends the round while the turn player still held the card they had drawn.
/// Clearing the field outright deleted that card from the game.
///
/// The seed replays the same deal as in the TS suite (the RNG is a faithful
/// port), but the fuzzer's action stream is not bit-identical, so the scenario
/// itself is pinned separately in `kaboo.rs`.
#[test]
fn regression_seed_1050406591_keeps_every_card() {
    fuzz_round(1_050_406_591, 2, false);
}
