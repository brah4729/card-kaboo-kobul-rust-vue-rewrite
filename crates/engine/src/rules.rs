//! Every rule flag, its default, and why. These defaults are the answers
//! recorded in AGENTS.md §6 "Open decisions" — change them there first, then here.

use crate::types::{EndCondition, KabooPenaltyMode, RuleConfig, TieBreak};

impl Default for RuleConfig {
    fn default() -> Self {
        RuleConfig {
            // Jokers: off by default, per the brief.
            jokers: false,
            // "Matt's pairs": off by default, per the brief.
            matts_pairs: false,
            // Counter-KABOO is implemented with the minimal reading (AGENTS.md §6).
            counter_kaboo: true,
            // Tapping matches on rank, so red kings match black kings (AGENTS.md §6).
            red_kings_match_rank: true,
            // A tap is stale once the discard top changes (AGENTS.md §6).
            tap_closes_on_discard_change: true,
            // Abandoning a claimed tap draws a penalty card (AGENTS.md §6).
            tap_timeout_is_wrong_tap: true,
            // Deterministic, and leaks nothing, unlike a "closest to the caller" rule.
            tie_break: TieBreak::SeatOrder,
            // Five rounds, then the lowest cumulative score wins (AGENTS.md §6).
            end_condition: EndCondition::FixedRounds,
            rounds: 5,
            score_limit: 100,
            // A failed call costs the caller their hand PLUS the penalty (AGENTS.md §6).
            kaboo_penalty_mode: KabooPenaltyMode::Added,
            kaboo_penalty_points: 20,
            // Guarantees the round terminates even if players ping-pong the discard forever.
            turn_limit: 500,
            // Deadlines are stored timestamps; the room's alarm resolves them.
            turn_deadline_ms: 75_000,
            peek_deadline_ms: 45_000,
            tap_deadline_ms: 8_000,
            final_swap_deadline_ms: 20_000,
            hand_size: 4,
        }
    }
}

/// Turn-order preference for equal totals. Returns a lower number for the
/// player who should win a tie: seat order walking left from the dealer.
pub fn tie_break_rank(seat: usize, dealer_seat: usize, seat_count: usize) -> usize {
    (seat + 2 * seat_count - dealer_seat - 1) % seat_count
}
