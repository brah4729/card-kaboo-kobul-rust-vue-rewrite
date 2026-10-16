//! THE ROOM — every use case, framework-free.
//!
//! One room lives in one Durable Object, which runs one request at a time. So
//! there is no compare-and-set here and no retry loop: each use case takes the
//! room's slot (`&mut Option<RoomRecord>`), decides, and either writes the
//! whole new record or leaves it alone. The tap race is settled by arrival
//! order — the second tap meets the first one's claim and the engine says
//! `E_TOO_LATE`, for the right reason.
//!
//! Two rules run through all of it:
//!
//! * **The session decides who you are.** The `playerId` in an incoming
//!   action is discarded and replaced with the one the signed token proves.
//! * **Every change is pushed.** A use case that changed the room returns the
//!   fresh `SyncResponse` for every other seat, and the Durable Object sends
//!   each to that seat's sockets. The caller gets theirs in the HTTP response.

pub mod api;
pub mod session;

use std::collections::BTreeMap;

use kaboo_engine::{
    Action, Ctx, EngineEvent, MAX_PLAYERS, MIN_PLAYERS, Phase, PlayerId, Seat, apply_action, next_deadline, project,
    start_game, tick,
};
use kaboo_protocol::{
    ActionResult, DeliveredEvent, Millis, RoomError, RoomErrorCode, RoomView, RosterEntry, Seated, SyncResponse,
    room_error,
};
use serde::{Deserialize, Serialize};

pub use session::{SessionPayload, sign_session, verify_session};

/// How many undelivered events we keep per player before dropping the oldest.
pub const PENDING_LIMIT: usize = 50;
/// A room with no writes for this long is deleted by its alarm.
pub const ROOM_TTL_MS: Millis = 12 * 60 * 60 * 1000;
const NAME_MAX: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoomStatus {
    Lobby,
    Playing,
}

/// A private event plus the sequence number it was assigned.
///
/// The client keeps the highest seq it has seen and passes it back as `since`,
/// so an event a dropped push never delivered is still greater than the cursor
/// and comes back on the next read or reconnect. Delivery is never tracked
/// server-side, which is what lets a push be best-effort.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueuedEvent {
    pub seq: u32,
    pub event: EngineEvent,
}

/// The room at rest. SERVER ONLY: it holds every hand and the deck order, so it
/// is never serialised toward a client — `project()` is the only path out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomRecord {
    pub code: String,
    pub status: RoomStatus,
    /// Join order is seat order: the engine assigns `seat` by array index.
    pub roster: Vec<RosterEntry>,
    pub host_id: PlayerId,
    /// The number the next seat id is minted from. Never decremented, so an id
    /// is never reused: a player who left the lobby keeps a token naming their
    /// old id, and handing that id to the next arrival would hand them the seat.
    pub next_seat: u32,
    /// `None` until the host deals.
    pub state: Option<kaboo_engine::GameState>,
    pub pending: BTreeMap<PlayerId, Vec<QueuedEvent>>,
    /// Monotonic; clients pass the last value they saw as `since`.
    pub event_seq: u32,
    pub created_at: Millis,
    pub updated_at: Millis,
}

/// Time, randomness and the signing key, injected.
pub struct Deps<'a> {
    pub now: Millis,
    pub rng: &'a mut dyn FnMut() -> f64,
    pub key: &'a [u8],
}

impl Deps<'_> {
    fn ctx(&mut self) -> Ctx<'_> {
        Ctx { now: self.now, rng: &mut *self.rng }
    }
}

/// A message for one seat's sockets.
#[derive(Debug, Clone, PartialEq)]
pub struct Push {
    pub player_id: PlayerId,
    pub message: SyncResponse,
}

/// What a use case produced: its answer, whether the room changed (and must
/// be stored), and what to push to the other seats.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome<T> {
    pub value: T,
    pub changed: bool,
    pub pushes: Vec<Push>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CreateError {
    /// That code is already a room: mint another and try again.
    Taken,
    Room(RoomError),
}

/// Trim, cap and reject empty. The UI escapes whatever is left.
pub fn clean_name(raw: Option<&str>) -> Option<String> {
    let trimmed: String = raw?.trim().chars().take(NAME_MAX).collect();
    (!trimmed.is_empty()).then_some(trimmed)
}

/// What one player is entitled to see, right now. Never the raw state.
pub fn view_for(record: &RoomRecord, player_id: &str) -> RoomView {
    let host = record.host_id == player_id;
    match (&record.state, record.status) {
        (Some(state), RoomStatus::Playing) => RoomView::Playing {
            code: record.code.clone(),
            you: player_id.into(),
            host,
            view: Box::new(project(state, player_id)),
        },
        _ => RoomView::Lobby {
            code: record.code.clone(),
            you: player_id.into(),
            host,
            host_id: record.host_id.clone(),
            roster: record.roster.clone(),
            seats: MAX_PLAYERS,
        },
    }
}

/// The sync payload for one player: their view plus every queued private
/// event above `since`. A push sends `since = 0`, because the server does not
/// know where the client is; the per-event seq lets the client dedupe.
pub fn sync_for(record: &RoomRecord, player_id: &str, since: u32) -> SyncResponse {
    SyncResponse {
        room: view_for(record, player_id),
        events: record
            .pending
            .get(player_id)
            .into_iter()
            .flatten()
            .filter(|e| e.seq > since)
            .map(|e| DeliveredEvent { event: e.event.clone(), seq: e.seq })
            .collect(),
        event_seq: record.event_seq,
    }
}

/// The fresh truth for every seat except `except`, who gets it in their response.
fn fan_out(record: &RoomRecord, except: Option<&str>) -> Vec<Push> {
    record
        .roster
        .iter()
        .filter(|r| Some(r.id.as_str()) != except)
        .map(|r| Push { player_id: r.id.clone(), message: sync_for(record, &r.id, 0) })
        .collect()
}

fn changed<T>(record: &RoomRecord, except: Option<&str>, value: T) -> Outcome<T> {
    Outcome { value, changed: true, pushes: fan_out(record, except) }
}

fn unchanged<T>(value: T) -> Outcome<T> {
    Outcome { value, changed: false, pushes: Vec::new() }
}

/// Queue every PRIVATE event for its recipient. Public events carry no face and
/// already reach clients through `view.log`, so they are not queued.
fn queue_private(record: &mut RoomRecord, events: &[EngineEvent]) -> bool {
    let before = record.event_seq;
    for event in events {
        let Some(to) = &event.to else { continue };
        record.event_seq += 1;
        let queue = record.pending.entry(to.clone()).or_default();
        queue.push(QueuedEvent { seq: record.event_seq, event: event.clone() });
        if queue.len() > PENDING_LIMIT {
            let excess = queue.len() - PENDING_LIMIT;
            queue.drain(..excess);
        }
    }
    record.event_seq != before
}

fn live(slot: &Option<RoomRecord>, now: Millis) -> Result<&RoomRecord, RoomError> {
    match slot {
        Some(r) if now - r.updated_at <= ROOM_TTL_MS => Ok(r),
        _ => Err(room_error(RoomErrorCode::NotFound)),
    }
}

/// The verified session for `code`, checked BEFORE the room is looked up: a
/// token minted for one room says `E_BAD_SESSION` against any other code,
/// whether or not that code is a room. The code is part of the signed payload,
/// so a token cannot be replayed against another room.
pub fn session_for(code: &str, token: Option<&str>, now: Millis, key: &[u8]) -> Result<SessionPayload, RoomError> {
    verify_session(token, now, key).filter(|s| s.code == code).ok_or_else(|| room_error(RoomErrorCode::BadSession))
}

/// The session, the live room, and proof the session still holds a seat in it.
fn seated<'a>(
    slot: &'a Option<RoomRecord>,
    code: &str,
    token: Option<&str>,
    deps: &Deps,
) -> Result<(SessionPayload, &'a RoomRecord), RoomError> {
    let session = session_for(code, token, deps.now, deps.key)?;
    let record = live(slot, deps.now)?;
    // A seat that left the lobby is not a seat.
    if !record.roster.iter().any(|r| r.id == session.player_id) {
        return Err(room_error(RoomErrorCode::BadSession));
    }
    Ok((session, record))
}

fn seat_token(code: &str, player_id: &str, name: &str, deps: &Deps) -> String {
    sign_session(
        &SessionPayload { code: code.into(), player_id: player_id.into(), name: name.into(), iat: deps.now },
        deps.key,
    )
}

/// Mint a room and take seat 1. `Taken` when the code is already a live room.
pub fn create_room(
    slot: &mut Option<RoomRecord>,
    code: &str,
    raw_name: Option<&str>,
    deps: &mut Deps,
) -> Result<Seated, CreateError> {
    let name = clean_name(raw_name).ok_or_else(|| CreateError::Room(room_error(RoomErrorCode::BadRequest)))?;
    if live(slot, deps.now).is_ok() {
        return Err(CreateError::Taken);
    }
    let player_id = "p1".to_string();
    *slot = Some(RoomRecord {
        code: code.into(),
        status: RoomStatus::Lobby,
        roster: vec![RosterEntry { id: player_id.clone(), name: name.clone(), joined_at: deps.now }],
        host_id: player_id.clone(),
        next_seat: 2,
        state: None,
        pending: BTreeMap::new(),
        event_seq: 0,
        created_at: deps.now,
        updated_at: deps.now,
    });
    Ok(Seated { code: code.into(), token: seat_token(code, &player_id, &name, deps), player_id })
}

/// Take the next free seat. Seat order is join order, which is the turn ring.
pub fn join_room(
    slot: &mut Option<RoomRecord>,
    _code: &str,
    raw_name: Option<&str>,
    deps: &mut Deps,
) -> Result<Outcome<Seated>, RoomError> {
    let name = clean_name(raw_name).ok_or_else(|| room_error(RoomErrorCode::BadRequest))?;
    let record = live(slot, deps.now)?;
    if record.status != RoomStatus::Lobby {
        return Err(room_error(RoomErrorCode::AlreadyStarted));
    }
    if record.roster.len() >= MAX_PLAYERS {
        return Err(room_error(RoomErrorCode::RoomFull));
    }

    let mut next = record.clone();
    let player_id = format!("p{}", next.next_seat);
    next.next_seat += 1;
    next.roster.push(RosterEntry { id: player_id.clone(), name: name.clone(), joined_at: deps.now });
    next.updated_at = deps.now;

    let seated = Seated { code: next.code.clone(), token: seat_token(&next.code, &player_id, &name, deps), player_id };
    let me = seated.player_id.clone();
    let outcome = changed(&next, Some(&me), seated);
    *slot = Some(next);
    Ok(outcome)
}

/// Host only. This is the moment the roster freezes and the engine takes over.
pub fn start_room(
    slot: &mut Option<RoomRecord>,
    code: &str,
    token: Option<&str>,
    deps: &mut Deps,
) -> Result<Outcome<SyncResponse>, RoomError> {
    let (session, record) = seated(slot, code, token, deps)?;
    if record.status != RoomStatus::Lobby {
        return Err(room_error(RoomErrorCode::AlreadyStarted));
    }
    if record.host_id != session.player_id {
        return Err(room_error(RoomErrorCode::BadSession));
    }
    if record.roster.len() < MIN_PLAYERS {
        return Err(room_error(RoomErrorCode::BadRequest));
    }

    let mut next = record.clone();
    // The engine has no join action: the roster is handed over complete.
    let seats: Vec<Seat> = next.roster.iter().map(|r| Seat { id: r.id.clone(), name: r.name.clone() }).collect();
    next.state = Some(start_game(&next.code, &seats, &mut deps.ctx(), None));
    next.status = RoomStatus::Playing;
    next.updated_at = deps.now;

    let outcome = changed(&next, Some(&session.player_id), sync_for(&next, &session.player_id, 0));
    *slot = Some(next);
    Ok(outcome)
}

/// Apply one action. This is the tap race.
///
/// `apply_action` runs deadlines itself before dispatching, so a single call
/// both advances the clock and resolves the move. It can also return an error
/// AND a moved state — a deadline fired while the action was refused — and the
/// moved state must still be kept, or the timeout is silently lost.
pub fn act(
    slot: &mut Option<RoomRecord>,
    code: &str,
    token: Option<&str>,
    incoming: Action,
    deps: &mut Deps,
) -> Result<Outcome<ActionResult>, RoomError> {
    let (session, record) = seated(slot, code, token, deps)?;
    let Some(state) = record.state.as_ref().filter(|_| record.status == RoomStatus::Playing) else {
        return Err(room_error(RoomErrorCode::BadRequest));
    };

    // The body's playerId is discarded outright: the token says who you are.
    let action = incoming.with_player(&session.player_id);
    let result = apply_action(state, &action, &mut deps.ctx());

    let mut next = record.clone();
    let queued = queue_private(&mut next, &result.events);
    let moved = result.version_changed || queued;
    if moved {
        next.state = Some(result.state);
        next.updated_at = deps.now;
    }

    let value = ActionResult { sync: sync_for(&next, &session.player_id, 0), refusal: result.error };
    if !moved {
        // A refused action that moved nothing has nothing to tell anyone.
        return Ok(unchanged(value));
    }
    let outcome = changed(&next, Some(&session.player_id), value);
    *slot = Some(next);
    Ok(outcome)
}

/// Read this player's truth: the first load, a resync after a reconnect, and
/// the WebSocket's opening message.
///
/// Expired deadlines resolve here too (the alarm normally beats us to it), and
/// queued events at or below `since` are dropped: the client has confirmed them,
/// so that is garbage collection, never a lost event.
pub fn read(
    slot: &mut Option<RoomRecord>,
    code: &str,
    token: Option<&str>,
    since: u32,
    deps: &mut Deps,
) -> Result<Outcome<SyncResponse>, RoomError> {
    let (session, record) = seated(slot, code, token, deps)?;
    let me = session.player_id;

    let mut next = record.clone();
    let mut state_moved = false;
    if let Some(state) = &record.state
        && let Some(ticked) = tick(state, &mut deps.ctx())
    {
        queue_private(&mut next, &ticked.events);
        next.state = Some(ticked.state);
        state_moved = true;
    }

    let queue = next.pending.entry(me.clone()).or_default();
    let before = queue.len();
    queue.retain(|e| e.seq > since);
    let collected = queue.len() != before;
    if queue.is_empty() {
        next.pending.remove(&me);
    }

    let value = sync_for(&next, &me, since);
    if !state_moved && !collected {
        return Ok(unchanged(value));
    }
    if state_moved {
        next.updated_at = deps.now;
    }
    let outcome = Outcome {
        value,
        changed: true,
        // Garbage collection is nobody else's news; a moved clock is everybody's.
        pushes: if state_moved { fan_out(&next, Some(&me)) } else { Vec::new() },
    };
    *slot = Some(next);
    Ok(outcome)
}

/// Mark a player gone. Before the deal the seat goes back; after it, the seat is
/// never freed — the turn ring is fixed and the engine autoplays their turns.
pub fn leave_room(
    slot: &mut Option<RoomRecord>,
    code: &str,
    token: Option<&str>,
    deps: &mut Deps,
) -> Result<Outcome<()>, RoomError> {
    let (session, record) = seated(slot, code, token, deps)?;
    let me = session.player_id;

    let mut next = record.clone();
    next.updated_at = deps.now;
    if next.status == RoomStatus::Lobby {
        next.roster.retain(|r| r.id != me);
        if next.host_id == me
            && let Some(first) = next.roster.first()
        {
            next.host_id = first.id.clone();
        }
        // Last one out: the room is simply abandoned, and its alarm reaps it.
    } else if let Some(state) = &record.state {
        let result = apply_action(state, &Action::Leave { player_id: me.clone() }, &mut deps.ctx());
        queue_private(&mut next, &result.events);
        next.state = Some(result.state);
    }

    let outcome = changed(&next, Some(&me), ());
    *slot = Some(next);
    Ok(outcome)
}

/// What the room's alarm did.
#[derive(Debug, Clone, PartialEq)]
pub enum Alarm {
    /// Nothing was due.
    Idle,
    /// A deadline fired: store the room and push to everyone.
    Ticked(Vec<Push>),
    /// The room outlived its TTL: delete it.
    Expired,
}

/// Run the room's clock with nobody touching it. This is what replaced client
/// polling: a deadline now fires on time even if every phone is asleep.
pub fn alarm(slot: &mut Option<RoomRecord>, deps: &mut Deps) -> Alarm {
    let Some(record) = slot.as_ref() else { return Alarm::Idle };
    if deps.now - record.updated_at > ROOM_TTL_MS {
        *slot = None;
        return Alarm::Expired;
    }
    let Some(state) = &record.state else { return Alarm::Idle };
    let Some(ticked) = tick(state, &mut deps.ctx()) else { return Alarm::Idle };

    let mut next = record.clone();
    queue_private(&mut next, &ticked.events);
    next.state = Some(ticked.state);
    next.updated_at = deps.now;
    let pushes = fan_out(&next, None);
    *slot = Some(next);
    Alarm::Ticked(pushes)
}

/// When the room's alarm should next fire: the earliest game deadline, or the
/// moment the room expires if nothing sooner is due.
pub fn next_alarm(record: &RoomRecord) -> Millis {
    let expiry = record.updated_at + ROOM_TTL_MS + 1;
    let deadline = record.state.as_ref().filter(|s| s.phase != Phase::GameOver).and_then(next_deadline);
    deadline.map_or(expiry, |d| d.min(expiry))
}
