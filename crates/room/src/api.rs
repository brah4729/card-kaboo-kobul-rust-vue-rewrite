//! THE HTTP EDGE — parsing in, status codes out.
//!
//! Framework-free, so the Durable Object is a few lines and every route is
//! tested natively. The one rule worth stating: **a rejection never explains
//! itself.** Every failure leaves with the generic sentence from
//! `room_error_message`, because a specific reason is itself information.
//!
//! A success and a refused MOVE are both 200: a refused move still returns the
//! fresh projection, which is what tells the loser of a tap race the table moved.

use serde::Serialize;
use serde_json::{Map, Value};

use kaboo_engine::Action;
use kaboo_protocol::{RoomError, RoomErrorCode, room_error, status_of};

use crate::{CreateError, Deps, Push, RoomRecord, act, create_room, join_room, leave_room, read, start_room};

/// The room-scoped routes. Creating a room is separate: the Worker picks the
/// code first, so it is the one route that can come back `Taken`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Create,
    Join,
    Start,
    Actions,
    State,
    Leave,
}

impl Route {
    /// `(method, last path segment)` → route. `POST /api/rooms` is `Create`.
    pub fn parse(method: &str, op: Option<&str>) -> Option<Route> {
        Some(match (method, op) {
            ("POST", None) => Route::Create,
            ("POST", Some("join")) => Route::Join,
            ("POST", Some("start")) => Route::Start,
            ("POST", Some("actions")) => Route::Actions,
            ("GET", Some("state")) => Route::State,
            ("POST", Some("leave")) => Route::Leave,
            _ => return None,
        })
    }
}

pub struct ApiRequest<'a> {
    pub route: Route,
    /// Already normalized (`normalize_room_code`).
    pub code: &'a str,
    /// The `x-kaboo-session` header.
    pub session: Option<&'a str>,
    /// The raw `since` query parameter.
    pub since: Option<&'a str>,
    pub body: &'a [u8],
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApiResponse {
    pub status: u16,
    pub body: String,
    /// The record changed and must be stored.
    pub changed: bool,
    pub pushes: Vec<Push>,
    /// `Create` only: the code was already a room. Mint another.
    pub taken: bool,
}

fn json<T: Serialize>(status: u16, value: &T) -> ApiResponse {
    ApiResponse {
        status,
        body: serde_json::to_string(value).expect("wire types always serialize"),
        changed: false,
        pushes: Vec::new(),
        taken: false,
    }
}

pub fn fail(error: &RoomError) -> ApiResponse {
    json(status_of(error.code), error)
}

fn bad_request() -> ApiResponse {
    fail(&room_error(RoomErrorCode::BadRequest))
}

/// A body that is absent, empty or malformed is an EMPTY body, not a 500. The
/// route's own validation then rejects it with the right code — one error path.
fn body_of(raw: &[u8]) -> Map<String, Value> {
    match serde_json::from_slice::<Value>(raw) {
        Ok(Value::Object(map)) => map,
        _ => Map::new(),
    }
}

/// `?since=` — a cursor the client owns. Anything unusable means "from the
/// start", never "you missed it".
pub fn since_of(raw: Option<&str>) -> u32 {
    raw.and_then(|s| s.parse::<u32>().ok()).unwrap_or(0)
}

/// Handle one request against this room's slot.
pub fn handle(slot: &mut Option<RoomRecord>, req: &ApiRequest, deps: &mut Deps) -> ApiResponse {
    let body = body_of(req.body);
    let name = body.get("name").and_then(Value::as_str);

    let result = match req.route {
        Route::Create => {
            return match create_room(slot, req.code, name, deps) {
                Ok(seated) => ApiResponse { changed: true, ..json(200, &seated) },
                Err(CreateError::Taken) => ApiResponse { taken: true, ..fail(&room_error(RoomErrorCode::Busy)) },
                Err(CreateError::Room(e)) => fail(&e),
            };
        }
        Route::Join => join_room(slot, req.code, name, deps).map(|o| (json(200, &o.value), o.changed, o.pushes)),
        Route::Start => {
            start_room(slot, req.code, req.session, deps).map(|o| (json(200, &o.value), o.changed, o.pushes))
        }
        Route::Actions => {
            // An action that does not parse as an action is a bad request, not a
            // refusal: no engine state could have produced it.
            //
            // The browser does not say who it is in the body (the session does),
            // so `playerId` is filled with a placeholder before parsing and then
            // replaced from the session. Whatever the body claimed is discarded.
            let Some(Value::Object(mut action)) = body.get("action").cloned() else { return bad_request() };
            action.insert("playerId".into(), Value::String(String::new()));
            let Ok(action) = serde_json::from_value::<Action>(Value::Object(action)) else { return bad_request() };
            act(slot, req.code, req.session, action, deps).map(|o| (json(200, &o.value), o.changed, o.pushes))
        }
        Route::State => read(slot, req.code, req.session, since_of(req.since), deps)
            .map(|o| (json(200, &o.value), o.changed, o.pushes)),
        Route::Leave => leave_room(slot, req.code, req.session, deps)
            .map(|o| (json(200, &serde_json::json!({ "ok": true })), o.changed, o.pushes)),
    };

    match result {
        Ok((response, changed, pushes)) => ApiResponse { changed, pushes, ..response },
        Err(e) => fail(&e),
    }
}
