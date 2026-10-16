//! Round scoring and the end of the game. Nothing here may run before the
//! round is revealed: a total computed during play is a total that can leak.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use crate::cards::hand_total;
use crate::rules::tie_break_rank;
use crate::types::{
    EndCondition, GameResult, GameState, KabooPenaltyMode, PlayerState, RoundResult, Standing, TieBreak,
};

/// Total of one player's hand. NEVER call this before the round is revealed.
pub fn hand_score(state: &GameState, player: &PlayerState) -> i32 {
    hand_total(&player.hand, &state.rules)
}

/// Lowest total wins; ties break by seat order walking left from the dealer.
fn compare_players(state: &GameState, a: &PlayerState, b: &PlayerState) -> Ordering {
    let n = state.players.len();
    hand_score(state, a).cmp(&hand_score(state, b)).then_with(|| match state.rules.tie_break {
        TieBreak::SeatOrder => {
            tie_break_rank(a.seat, state.dealer_seat, n).cmp(&tie_break_rank(b.seat, state.dealer_seat, n))
        }
        TieBreak::None => a.seat.cmp(&b.seat),
    })
}

/// Score the round and fold it into the cumulative totals.
///
/// Mutates every player's `score` — called exactly once, from `reveal_round`.
pub fn score_round(state: &mut GameState) -> RoundResult {
    let totals: BTreeMap<String, i32> = state.players.iter().map(|p| (p.id.clone(), hand_score(state, p))).collect();

    let mut ranked: Vec<&PlayerState> = state.players.iter().collect();
    ranked.sort_by(|a, b| compare_players(state, a, b));
    let winner_id = ranked[0].id.clone();

    // A caller who did not win eats the penalty. Counter-callers are scored the
    // same way — that is the minimal reading of counter-KABOO (AGENTS.md §6).
    let penalty_ids: Vec<String> = state
        .players
        .iter()
        .filter(|p| (p.called_kaboo || p.counter_kaboo) && p.id != winner_id)
        .map(|p| p.id.clone())
        .collect();

    let mut awarded: BTreeMap<String, i32> = BTreeMap::new();
    for p in &state.players {
        let total = totals[&p.id];
        let points = if p.id == winner_id {
            0
        } else if penalty_ids.contains(&p.id) {
            match state.rules.kaboo_penalty_mode {
                KabooPenaltyMode::Flat => state.rules.kaboo_penalty_points,
                KabooPenaltyMode::Added => total + state.rules.kaboo_penalty_points,
            }
        } else {
            total
        };
        awarded.insert(p.id.clone(), points);
    }

    for p in &mut state.players {
        p.score += awarded[&p.id];
    }

    RoundResult { round: state.round, totals, winner_id, penalty_ids, awarded }
}

pub fn should_game_end(state: &GameState) -> bool {
    if state.round == 0 {
        return false;
    }
    match state.rules.end_condition {
        EndCondition::FixedRounds => state.round >= state.rules.rounds,
        // A score threshold ends the game as soon as anyone reaches it. Highest
        // score is worst, so there is no incentive to "win" by hitting it.
        EndCondition::ScoreThreshold => state.players.iter().any(|p| p.score >= state.rules.score_limit),
    }
}

pub fn build_game_result(state: &GameState) -> GameResult {
    let n = state.players.len();
    let mut ranked: Vec<&PlayerState> = state.players.iter().collect();
    ranked.sort_by(|a, b| {
        a.score.cmp(&b.score).then_with(|| {
            tie_break_rank(a.seat, state.dealer_seat, n).cmp(&tie_break_rank(b.seat, state.dealer_seat, n))
        })
    });
    let standings: Vec<Standing> =
        ranked.iter().map(|p| Standing { player_id: p.id.clone(), score: p.score }).collect();

    GameResult { winner_id: standings[0].player_id.clone(), standings, reason: state.rules.end_condition }
}
