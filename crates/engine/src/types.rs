//! Core domain types for the Kaboo engine.
//!
//! This crate is PURE: no framework, no IO, no clock, no randomness of its own.
//! Time and randomness arrive through [`Ctx`]. That keeps `apply_action` a pure,
//! testable function that runs the same natively (tests) and in WASM (the Worker).
//!
//! Every type that crosses the wire derives `TS`, so the browser's types are
//! generated from these definitions rather than written a second time.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Epoch milliseconds. A plain `number` on the wire.
pub type Millis = i64;

pub type PlayerId = String;

/// Ids are stable and contiguous; see `cards.rs`.
pub type CardId = u8;

/// A hand slot holds a card id, or `None` when empty.
///
/// Slots are FIXED: they are never compacted, because players remember
/// positions. A hand can grow past 4 (penalty cards are appended) and can
/// contain holes (a tapped card removed by someone else leaves a gap).
pub type Slot = Option<CardId>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum Suit {
    S,
    H,
    D,
    C,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum Rank {
    A,
    #[serde(rename = "2")]
    #[ts(rename = "2")]
    Two,
    #[serde(rename = "3")]
    #[ts(rename = "3")]
    Three,
    #[serde(rename = "4")]
    #[ts(rename = "4")]
    Four,
    #[serde(rename = "5")]
    #[ts(rename = "5")]
    Five,
    #[serde(rename = "6")]
    #[ts(rename = "6")]
    Six,
    #[serde(rename = "7")]
    #[ts(rename = "7")]
    Seven,
    #[serde(rename = "8")]
    #[ts(rename = "8")]
    Eight,
    #[serde(rename = "9")]
    #[ts(rename = "9")]
    Nine,
    #[serde(rename = "10")]
    #[ts(rename = "10")]
    Ten,
    J,
    Q,
    K,
    #[serde(rename = "JOKER")]
    #[ts(rename = "JOKER")]
    Joker,
}

/// One entry of the immutable card table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Card {
    pub id: CardId,
    /// `None` for jokers.
    pub suit: Option<Suit>,
    pub rank: Rank,
    pub red: bool,
    /// Ace = 1, 2–10 face value, J/Q/K = 10, RED king = -1, joker = 0.
    pub value: i32,
}

/// A card as the wire shows it — only ever for a card the viewer may see.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PublicCard {
    pub id: CardId,
    pub rank: Rank,
    pub suit: Option<Suit>,
    pub red: bool,
    pub value: i32,
}

/// A reference to one specific slot on the table, for effects and taps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CardRef {
    pub player_id: PlayerId,
    pub slot: usize,
}

/// The turn spine. Every phase is meaningful and reachable.
///
/// There is no `lobby` phase: gathering players into a room happens BEFORE the
/// engine is involved, because the roster is fixed at `create_game`. A claimed
/// tap is orthogonal to whose turn it is, so it lives in `GameState::tap`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Phase {
    Peek,
    TurnStart,
    Drawn,
    Effect,
    FinalSwap,
    Reveal,
    GameOver,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerState {
    pub id: PlayerId,
    pub name: String,
    pub seat: usize,
    /// Server-only view of the hand. NEVER sent to a client wholesale — see project.rs.
    pub hand: Vec<Slot>,
    /// `gen[i]` increments every time hand slot `i` changes contents, for UI animation.
    #[serde(rename = "gen")]
    pub generation: Vec<u32>,
    /// Cumulative score across rounds. Public.
    pub score: i32,
    pub connected: bool,
    /// Peek phase complete for this player.
    pub ready: bool,
    /// Per-slot: has this player used their one peek on this slot?
    pub peeked: Vec<bool>,
    /// This player called KABOO (original caller, not a counter).
    pub called_kaboo: bool,
    pub counter_kaboo: bool,
    /// Has this player taken their single final turn after a KABOO?
    pub final_turn_done: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TapState {
    pub player_id: PlayerId,
    /// The `discard_seq` this claim was made against; a stale claim is rejected.
    pub seq: u32,
    pub claimed_at: Millis,
    pub deadline: Millis,
    /// Set once the tapper has named a card and it came from another player's hand.
    pub awaiting_placement: bool,
    /// Where the tapped card came from, once chosen.
    pub source: Option<CardRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectState {
    pub by: PlayerId,
    pub rank: Rank,
    /// King step 2: the two cards already peeked, awaiting the swap decision.
    pub a: Option<CardRef>,
    pub b: Option<CardRef>,
    pub awaiting_king_decision: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RoundResult {
    pub round: u32,
    /// Per-player totals as revealed at the end of the round.
    pub totals: BTreeMap<PlayerId, i32>,
    pub winner_id: PlayerId,
    /// Players who called (or counter-called) KABOO and did not win.
    pub penalty_ids: Vec<PlayerId>,
    /// Points added to each player's cumulative score this round.
    pub awarded: BTreeMap<PlayerId, i32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Standing {
    pub player_id: PlayerId,
    pub score: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GameResult {
    pub winner_id: PlayerId,
    pub standings: Vec<Standing>,
    pub reason: EndCondition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum LogKind {
    RoundStart,
    Peeked,
    Ready,
    Drew,
    TookDiscard,
    Swapped,
    Discarded,
    Effect,
    TapClaimed,
    TapSuccess,
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
    PlayerLeft,
    PlayerJoined,
}

/// Structured log entry. The UI renders the text — the engine never bakes in copy.
///
/// Deliberately has NO card field. A face logged here would stay in the
/// projection forever, so a card discarded in the open and later taken into a
/// hand would be readable by everyone — a leak no per-slot projection can undo.
/// Faces ship only while they are face-up on the table. See AGENTS.md §7.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LogEntry {
    pub round: u32,
    pub kind: LogKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub actor: Option<PlayerId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub target: Option<PlayerId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub text: Option<String>,
}

impl LogEntry {
    pub fn new(round: u32, kind: LogKind) -> Self {
        Self { round, kind, actor: None, target: None, text: None }
    }
    pub fn actor(mut self, id: &str) -> Self {
        self.actor = Some(id.to_string());
        self
    }
    pub fn target(mut self, id: &str) -> Self {
        self.target = Some(id.to_string());
        self
    }
    pub fn text(mut self, text: &str) -> Self {
        self.text = Some(text.to_string());
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum TieBreak {
    SeatOrder,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum EndCondition {
    FixedRounds,
    ScoreThreshold,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum KabooPenaltyMode {
    Added,
    Flat,
}

/// Every rule flag. Defaults and their reasons live in `rules.rs` and AGENTS.md §6.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RuleConfig {
    /// Include the two value-0 jokers. Default OFF.
    pub jokers: bool,
    /// "Matt's pairs": a matching pair in a hand counts 0 (red kings excepted). Default OFF.
    pub matts_pairs: bool,
    /// Allow counter-KABOO after a call. Default ON.
    pub counter_kaboo: bool,
    /// Do red kings match other kings when tapping? Default ON (match on rank).
    pub red_kings_match_rank: bool,
    /// A claimed tap goes stale once the discard top changes. Default ON.
    pub tap_closes_on_discard_change: bool,
    /// Does abandoning a claimed tap draw a penalty card? Default ON.
    pub tap_timeout_is_wrong_tap: bool,
    pub tie_break: TieBreak,
    pub end_condition: EndCondition,
    pub rounds: u32,
    pub score_limit: i32,
    pub kaboo_penalty_mode: KabooPenaltyMode,
    pub kaboo_penalty_points: i32,
    /// Hard guard so a round can never run forever. See AGENTS.md §6.
    pub turn_limit: u32,
    /// How long a player may hold up the table before the engine plays a safe
    /// default for them. Without this a disconnected player freezes the room.
    #[ts(type = "number")]
    pub turn_deadline_ms: Millis,
    #[ts(type = "number")]
    pub peek_deadline_ms: Millis,
    #[ts(type = "number")]
    pub tap_deadline_ms: Millis,
    #[ts(type = "number")]
    pub final_swap_deadline_ms: Millis,
    pub hand_size: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seat {
    pub id: PlayerId,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DrawSource {
    Deck,
    Discard,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Drawn {
    pub card_id: CardId,
    pub from: DrawSource,
    pub by: PlayerId,
}

/// Time and randomness, injected. Never read a clock or a random source inside
/// the engine: tests seed `rng`, the Worker backs it with `crypto.getRandomValues`.
pub struct Ctx<'a> {
    /// Epoch ms.
    pub now: Millis,
    /// Uniform [0, 1).
    pub rng: &'a mut dyn FnMut() -> f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameState {
    pub room_code: String,
    /// Bumped by every applied action.
    pub version: u32,
    pub phase: Phase,
    pub round: u32,
    pub rules: RuleConfig,
    pub players: Vec<PlayerState>,
    /// Player ids in seat order — the turn ring.
    pub seat_order: Vec<PlayerId>,
    pub dealer_seat: usize,
    pub turn_seat: usize,
    pub turn_count: u32,
    /// SERVER ONLY. Index 0 is the top of the draw pile.
    pub deck: Vec<CardId>,
    /// Index 0 is the bottom; the last element is the top.
    pub discard: Vec<CardId>,
    /// Bumped whenever the top of the discard pile changes. Stale taps are rejected against it.
    pub discard_seq: u32,
    /// The declined last card, parked so every card id is accounted for exactly once.
    pub set_aside: Option<CardId>,
    pub drawn: Option<Drawn>,
    /// The last card of the draw pile, held while the drawer decides. SERVER ONLY.
    pub held_card_id: Option<CardId>,
    pub tap: Option<TapState>,
    pub effect: Option<EffectState>,
    pub kaboo_caller_id: Option<PlayerId>,
    /// Seats still owed exactly one final turn after a KABOO.
    pub final_turns_remaining: Vec<PlayerId>,
    pub peek_deadline: Millis,
    pub final_swap_deadline: Millis,
    /// When the current player's turn is auto-played for them.
    pub turn_deadline: Millis,
    pub round_result: Option<RoundResult>,
    pub game_result: Option<GameResult>,
    pub log: Vec<LogEntry>,
    pub started_at: Millis,
    pub last_activity_at: Millis,
}

/// Every action names its actor in `player_id`. Actions that need a second party
/// use a distinctly named field (`target_player_id`, `target`, `a`/`b`) so an
/// actor can never be confused with a target.
///
/// The wire form is `{ "type": "CALL_KABOO", "playerId": "p1", ... }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum Action {
    // --- peek phase ---------------------------------------------------------
    Peek { player_id: PlayerId, slot: usize },
    Ready { player_id: PlayerId },
    // --- between rounds -----------------------------------------------------
    Continue { player_id: PlayerId },
    // --- turn spine ---------------------------------------------------------
    CallKaboo { player_id: PlayerId },
    CounterKaboo { player_id: PlayerId },
    Draw { player_id: PlayerId, from: DrawSource },
    Swap { player_id: PlayerId, slot: usize },
    Discard { player_id: PlayerId },
    FinalSwap { player_id: PlayerId, slot: usize },
    FinalDecline { player_id: PlayerId },
    // --- effects ------------------------------------------------------------
    EffectSkip { player_id: PlayerId },
    EffectPeekOwn { player_id: PlayerId, slot: usize },
    EffectPeekOther { player_id: PlayerId, target_player_id: PlayerId, slot: usize },
    EffectBlindSwap { player_id: PlayerId, a: CardRef, b: CardRef },
    EffectQueenSwap { player_id: PlayerId, slot: usize, target: CardRef },
    EffectKingPeek { player_id: PlayerId, a: CardRef, b: CardRef },
    EffectKingDecide { player_id: PlayerId, swap: bool },
    // --- out-of-turn tap race ------------------------------------------------
    Tap { player_id: PlayerId, seen_seq: u32 },
    TapChoose { player_id: PlayerId, target: CardRef },
    TapPlace { player_id: PlayerId, slot: usize },
    TapDeclinePlace { player_id: PlayerId },
    // --- housekeeping --------------------------------------------------------
    Leave { player_id: PlayerId },
}

/// The action's name alone, for `legal_actions`. Same SCREAMING_SNAKE strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[ts(export)]
pub enum ActionType {
    Peek,
    Ready,
    Continue,
    CallKaboo,
    CounterKaboo,
    Draw,
    Swap,
    Discard,
    FinalSwap,
    FinalDecline,
    EffectSkip,
    EffectPeekOwn,
    EffectPeekOther,
    EffectBlindSwap,
    EffectQueenSwap,
    EffectKingPeek,
    EffectKingDecide,
    Tap,
    TapChoose,
    TapPlace,
    TapDeclinePlace,
    Leave,
}

impl Action {
    pub fn player_id(&self) -> &str {
        match self {
            Action::Peek { player_id, .. }
            | Action::Ready { player_id }
            | Action::Continue { player_id }
            | Action::CallKaboo { player_id }
            | Action::CounterKaboo { player_id }
            | Action::Draw { player_id, .. }
            | Action::Swap { player_id, .. }
            | Action::Discard { player_id }
            | Action::FinalSwap { player_id, .. }
            | Action::FinalDecline { player_id }
            | Action::EffectSkip { player_id }
            | Action::EffectPeekOwn { player_id, .. }
            | Action::EffectPeekOther { player_id, .. }
            | Action::EffectBlindSwap { player_id, .. }
            | Action::EffectQueenSwap { player_id, .. }
            | Action::EffectKingPeek { player_id, .. }
            | Action::EffectKingDecide { player_id, .. }
            | Action::Tap { player_id, .. }
            | Action::TapChoose { player_id, .. }
            | Action::TapPlace { player_id, .. }
            | Action::TapDeclinePlace { player_id }
            | Action::Leave { player_id } => player_id,
        }
    }

    /// The same action, acting as `id`. The room layer uses this to discard
    /// whatever `playerId` arrived in the request body: the session decides who
    /// you are, never the body.
    pub fn with_player(mut self, id: &str) -> Self {
        match &mut self {
            Action::Peek { player_id, .. }
            | Action::Ready { player_id }
            | Action::Continue { player_id }
            | Action::CallKaboo { player_id }
            | Action::CounterKaboo { player_id }
            | Action::Draw { player_id, .. }
            | Action::Swap { player_id, .. }
            | Action::Discard { player_id }
            | Action::FinalSwap { player_id, .. }
            | Action::FinalDecline { player_id }
            | Action::EffectSkip { player_id }
            | Action::EffectPeekOwn { player_id, .. }
            | Action::EffectPeekOther { player_id, .. }
            | Action::EffectBlindSwap { player_id, .. }
            | Action::EffectQueenSwap { player_id, .. }
            | Action::EffectKingPeek { player_id, .. }
            | Action::EffectKingDecide { player_id, .. }
            | Action::Tap { player_id, .. }
            | Action::TapChoose { player_id, .. }
            | Action::TapPlace { player_id, .. }
            | Action::TapDeclinePlace { player_id }
            | Action::Leave { player_id } => *player_id = id.to_string(),
        }
        self
    }

    pub fn kind(&self) -> ActionType {
        match self {
            Action::Peek { .. } => ActionType::Peek,
            Action::Ready { .. } => ActionType::Ready,
            Action::Continue { .. } => ActionType::Continue,
            Action::CallKaboo { .. } => ActionType::CallKaboo,
            Action::CounterKaboo { .. } => ActionType::CounterKaboo,
            Action::Draw { .. } => ActionType::Draw,
            Action::Swap { .. } => ActionType::Swap,
            Action::Discard { .. } => ActionType::Discard,
            Action::FinalSwap { .. } => ActionType::FinalSwap,
            Action::FinalDecline { .. } => ActionType::FinalDecline,
            Action::EffectSkip { .. } => ActionType::EffectSkip,
            Action::EffectPeekOwn { .. } => ActionType::EffectPeekOwn,
            Action::EffectPeekOther { .. } => ActionType::EffectPeekOther,
            Action::EffectBlindSwap { .. } => ActionType::EffectBlindSwap,
            Action::EffectQueenSwap { .. } => ActionType::EffectQueenSwap,
            Action::EffectKingPeek { .. } => ActionType::EffectKingPeek,
            Action::EffectKingDecide { .. } => ActionType::EffectKingDecide,
            Action::Tap { .. } => ActionType::Tap,
            Action::TapChoose { .. } => ActionType::TapChoose,
            Action::TapPlace { .. } => ActionType::TapPlace,
            Action::TapDeclinePlace { .. } => ActionType::TapDeclinePlace,
            Action::Leave { .. } => ActionType::Leave,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum EngineErrorCode {
    #[serde(rename = "E_ILLEGAL")]
    #[ts(rename = "E_ILLEGAL")]
    Illegal,
    #[serde(rename = "E_NOT_YOUR_TURN")]
    #[ts(rename = "E_NOT_YOUR_TURN")]
    NotYourTurn,
    #[serde(rename = "E_STALE")]
    #[ts(rename = "E_STALE")]
    Stale,
    #[serde(rename = "E_TOO_LATE")]
    #[ts(rename = "E_TOO_LATE")]
    TooLate,
    #[serde(rename = "E_UNKNOWN_PLAYER")]
    #[ts(rename = "E_UNKNOWN_PLAYER")]
    UnknownPlayer,
    #[serde(rename = "E_ROOM_FULL")]
    #[ts(rename = "E_ROOM_FULL")]
    RoomFull,
    #[serde(rename = "E_GAME_OVER")]
    #[ts(rename = "E_GAME_OVER")]
    GameOver,
}

/// Deliberately generic — must never explain a rejection when the reason is hidden info.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EngineError {
    #[ts(type = "false")]
    pub ok: bool,
    pub code: EngineErrorCode,
    pub message: String,
}
