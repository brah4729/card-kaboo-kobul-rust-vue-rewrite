use std::collections::BTreeSet;

use kaboo_engine::cards::{
    DECK_SIZE, JOKER_IDS, card, hand_total, is_red_king, make_deck, matches_for_tap, rank_value, shuffle,
};
use kaboo_engine::testkit::{cid, mulberry32};
use kaboo_engine::{Rank, RuleConfig, Suit};

fn all_cards() -> Vec<kaboo_engine::Card> {
    (0..=53).map(card).collect()
}

fn jokers(on: bool) -> RuleConfig {
    RuleConfig { jokers: on, ..Default::default() }
}

fn pairs(on: bool) -> RuleConfig {
    RuleConfig { matts_pairs: on, ..Default::default() }
}

fn kings_match(on: bool) -> RuleConfig {
    RuleConfig { red_kings_match_rank: on, ..Default::default() }
}

// --- card values -----------------------------------------------------------

#[test]
fn scores_the_documented_values() {
    assert_eq!(rank_value(Rank::A, false), 1);
    assert_eq!(rank_value(Rank::Five, false), 5);
    assert_eq!(rank_value(Rank::Ten, false), 10);
    assert_eq!(rank_value(Rank::J, false), 10);
    assert_eq!(rank_value(Rank::Q, true), 10);
    assert_eq!(rank_value(Rank::K, false), 10);
    assert_eq!(rank_value(Rank::Joker, false), 0);
}

#[test]
fn makes_both_red_kings_worth_minus_one_and_black_kings_ten() {
    let cards = all_cards();
    let red: Vec<_> = cards.iter().filter(|c| c.rank == Rank::K && c.red).collect();
    let black: Vec<_> = cards.iter().filter(|c| c.rank == Rank::K && !c.red).collect();
    assert_eq!(red.len(), 2);
    assert_eq!(black.len(), 2);
    assert!(red.iter().all(|k| k.value == -1));
    assert!(black.iter().all(|k| k.value == 10));
}

#[test]
fn has_exactly_52_standard_cards_with_unique_ids() {
    let cards = all_cards();
    assert_eq!(cards.iter().filter(|c| c.rank != Rank::Joker).count(), usize::from(DECK_SIZE));
    let unique: BTreeSet<_> = cards.iter().map(|c| c.id).collect();
    assert_eq!(unique.len(), cards.len());
}

#[test]
fn excludes_jokers_from_the_deck_unless_the_flag_is_on() {
    assert_eq!(make_deck(&jokers(false)).len(), 52);
    assert!(!make_deck(&jokers(false)).iter().any(|id| JOKER_IDS.contains(id)));
    assert_eq!(make_deck(&jokers(true)).len(), 54);
    assert!(make_deck(&jokers(true)).contains(&52));
}

// --- shuffle -----------------------------------------------------------------

#[test]
fn shuffle_is_a_permutation() {
    let input = make_deck(&jokers(true));
    let out = shuffle(&input, &mut mulberry32(3));
    assert_eq!(out.len(), input.len());
    let mut sorted = out.clone();
    sorted.sort();
    assert_eq!(sorted, input);
    assert_eq!(input, make_deck(&jokers(true)), "input untouched");
}

#[test]
fn shuffle_is_reproducible_from_a_seed() {
    let a = shuffle(&make_deck(&jokers(false)), &mut mulberry32(99));
    let b = shuffle(&make_deck(&jokers(false)), &mut mulberry32(99));
    assert_eq!(a, b);
}

#[test]
fn shuffle_actually_reorders() {
    let input = make_deck(&jokers(false));
    assert_ne!(shuffle(&input, &mut mulberry32(12345)), input);
}

#[test]
fn shuffle_produces_a_spread_of_orderings_across_seeds() {
    let firsts: BTreeSet<_> =
        (0..200).map(|seed| shuffle(&make_deck(&jokers(false)), &mut mulberry32(seed))[0]).collect();
    // A biased shuffle (or an identity) would collapse this badly.
    assert!(firsts.len() > 30, "only {} distinct first cards", firsts.len());
}

#[test]
fn mulberry32_matches_the_typescript_generator() {
    // Reference values computed with the TS `seededRng` and `shuffle`, so a
    // seed keeps meaning the same deal it did before the port.
    let mut rng = mulberry32(7);
    let got: Vec<u64> = (0..3).map(|_| (rng() * 4_294_967_296.0) as u64).collect();
    assert_eq!(got, vec![50_271_532, 266_108_690, 4_195_786_334]);

    let deck: Vec<u8> = (0..52).collect();
    let shuffled = shuffle(&deck, &mut mulberry32(99));
    assert_eq!(&shuffled[..10], &[3, 31, 1, 32, 45, 20, 19, 18, 21, 46]);
}

// --- hand_total ----------------------------------------------------------------

#[test]
fn sums_plain_values() {
    assert_eq!(
        hand_total(&[Some(cid(Rank::A, Suit::S)), Some(cid(Rank::Five, Suit::H)), None, None], &pairs(false)),
        6
    );
}

#[test]
fn counts_red_kings_as_minus_one() {
    assert_eq!(hand_total(&[Some(cid(Rank::K, Suit::H)), Some(cid(Rank::K, Suit::D))], &pairs(false)), -2);
    assert_eq!(hand_total(&[Some(cid(Rank::K, Suit::H)), Some(cid(Rank::K, Suit::S))], &pairs(false)), 9);
}

#[test]
fn ignores_empty_slots() {
    assert_eq!(hand_total(&[None, None, None, None], &pairs(false)), 0);
    assert_eq!(hand_total(&[Some(cid(Rank::A, Suit::S)), None], &pairs(false)), 1);
}

#[test]
fn applies_matts_pairs_only_when_the_flag_is_on() {
    let hand = [Some(cid(Rank::Four, Suit::S)), Some(cid(Rank::Four, Suit::H))];
    assert_eq!(hand_total(&hand, &pairs(false)), 8);
    assert_eq!(hand_total(&hand, &pairs(true)), 0);
}

#[test]
fn excludes_red_kings_from_matts_pairs() {
    // A pair of red kings is worth -2 and stays -2 even with pairs on.
    assert_eq!(hand_total(&[Some(cid(Rank::K, Suit::H)), Some(cid(Rank::K, Suit::D))], &pairs(true)), -2);
    // A red king does not zero out a black king either.
    assert_eq!(hand_total(&[Some(cid(Rank::K, Suit::H)), Some(cid(Rank::K, Suit::S))], &pairs(true)), 9);
}

#[test]
fn zeroes_only_the_paired_cards_not_the_whole_hand() {
    let hand = [Some(cid(Rank::Four, Suit::S)), Some(cid(Rank::Four, Suit::H)), Some(cid(Rank::Nine, Suit::C)), None];
    assert_eq!(hand_total(&hand, &pairs(true)), 9);
}

// --- tap matching ----------------------------------------------------------------

#[test]
fn matches_on_rank() {
    let r = kings_match(true);
    assert!(matches_for_tap(card(cid(Rank::Seven, Suit::S)), card(cid(Rank::Seven, Suit::H)), &r));
    assert!(!matches_for_tap(card(cid(Rank::Seven, Suit::S)), card(cid(Rank::Eight, Suit::H)), &r));
}

#[test]
fn matches_kings_regardless_of_colour_by_default() {
    assert!(matches_for_tap(card(cid(Rank::K, Suit::H)), card(cid(Rank::K, Suit::S)), &RuleConfig::default()));
}

#[test]
fn falls_back_to_colour_exact_kings_when_the_flag_is_off() {
    let r = kings_match(false);
    assert!(!matches_for_tap(card(cid(Rank::K, Suit::H)), card(cid(Rank::K, Suit::S)), &r));
    assert!(matches_for_tap(card(cid(Rank::K, Suit::H)), card(cid(Rank::K, Suit::D)), &r));
}

#[test]
fn only_matches_jokers_with_jokers() {
    let r = kings_match(true);
    assert!(matches_for_tap(card(52), card(53), &r));
    assert!(!matches_for_tap(card(52), card(cid(Rank::A, Suit::S)), &r));
}

#[test]
fn treats_a_red_king_as_a_king_for_tapping_even_though_it_scores_minus_one() {
    let red_king = card(cid(Rank::K, Suit::H));
    assert!(is_red_king(red_king));
    assert!(matches_for_tap(red_king, card(cid(Rank::K, Suit::C)), &kings_match(true)));
}
