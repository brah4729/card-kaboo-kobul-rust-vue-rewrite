//! A whole server, in one process, with no network: a map of room code → slot,
//! a virtual clock and a seeded shuffle. Every request goes through
//! `kaboo_room::api::handle`, the exact function the Durable Object calls, so
//! these suites exercise the real edge — routing, status codes and JSON.
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use kaboo_engine::testkit::{T0, hidden_card_ids, mulberry32, visible_card_ids};
use kaboo_engine::{Action, GameState, PlayerId, Projection};
use kaboo_protocol::{ActionResult, RoomError, Seated, SyncResponse, make_room_code, normalize_room_code};
use kaboo_room::api::{ApiRequest, ApiResponse, Route, handle};
use kaboo_room::{Alarm, Deps, Push, RoomRecord, alarm};

pub const KEY: &[u8] = b"test-secret-not-a-real-one";

pub struct Server {
    pub rooms: BTreeMap<String, Option<RoomRecord>>,
    pub now: i64,
    pub pushes: Vec<Push>,
    rng: Box<dyn FnMut() -> f64>,
}

#[derive(Debug, Clone)]
pub struct Seat {
    pub id: PlayerId,
    pub token: String,
}

impl From<Seated> for Seat {
    fn from(s: Seated) -> Self {
        Seat { id: s.player_id, token: s.token }
    }
}

impl Server {
    pub fn new(seed: u32) -> Self {
        Server { rooms: BTreeMap::new(), now: T0, pushes: Vec::new(), rng: Box::new(mulberry32(seed)) }
    }

    pub fn advance(&mut self, ms: i64) {
        self.now += ms;
    }

    /// One HTTP request, routed the way the Worker routes it.
    pub fn request(
        &mut self,
        route: Route,
        code: &str,
        session: Option<&str>,
        since: Option<&str>,
        body: &str,
    ) -> ApiResponse {
        let code = normalize_room_code(code);
        let slot = self.rooms.entry(code.clone()).or_insert(None);
        let mut deps = Deps { now: self.now, rng: &mut *self.rng, key: KEY };
        let response =
            handle(slot, &ApiRequest { route, code: &code, session, since, body: body.as_bytes() }, &mut deps);
        self.pushes.extend(response.pushes.iter().cloned());
        response
    }

    /// `POST /api/rooms`: mint a code, retrying on a collision like the Worker.
    pub fn create_raw(&mut self, body: &str) -> ApiResponse {
        loop {
            let code = make_room_code(&mut *self.rng);
            let response = self.request(Route::Create, &code, None, None, body);
            if !response.taken {
                return response;
            }
        }
    }

    pub fn alarm(&mut self, code: &str) -> Alarm {
        let slot = self.rooms.entry(code.to_string()).or_insert(None);
        let mut deps = Deps { now: self.now, rng: &mut *self.rng, key: KEY };
        let result = alarm(slot, &mut deps);
        if let Alarm::Ticked(pushes) = &result {
            self.pushes.extend(pushes.iter().cloned());
        }
        result
    }

    pub fn record(&self, code: &str) -> Option<&RoomRecord> {
        self.rooms.get(code).and_then(Option::as_ref)
    }

    pub fn state(&self, code: &str) -> &GameState {
        self.record(code).and_then(|r| r.state.as_ref()).expect("a dealt room")
    }

    // --- typed helpers -------------------------------------------------------

    #[track_caller]
    pub fn create(&mut self, name: &str) -> (String, Seat) {
        let response = self.create_raw(&json!({ "name": name }).to_string());
        assert_eq!(response.status, 200, "{}", response.body);
        let seated: Seated = serde_json::from_str(&response.body).unwrap();
        (seated.code.clone(), seated.into())
    }

    pub fn join_raw(&mut self, code: &str, name: &str) -> ApiResponse {
        self.request(Route::Join, code, None, None, &json!({ "name": name }).to_string())
    }

    #[track_caller]
    pub fn join(&mut self, code: &str, name: &str) -> Seat {
        let response = self.join_raw(code, name);
        assert_eq!(response.status, 200, "{}", response.body);
        serde_json::from_str::<Seated>(&response.body).unwrap().into()
    }

    pub fn start_raw(&mut self, code: &str, token: Option<&str>) -> ApiResponse {
        self.request(Route::Start, code, token, None, "")
    }

    #[track_caller]
    pub fn start(&mut self, code: &str, token: &str) -> SyncResponse {
        ok(self.start_raw(code, Some(token)))
    }

    pub fn act_raw(&mut self, code: &str, token: Option<&str>, action: &Value) -> ApiResponse {
        self.request(Route::Actions, code, token, None, &json!({ "action": action }).to_string())
    }

    /// A move, or the room error that kept it from reaching the engine.
    pub fn attempt(&mut self, code: &str, token: &str, action: &Action) -> Result<ActionResult, RoomError> {
        let response = self.act_raw(code, Some(token), &serde_json::to_value(action).unwrap());
        parse(response)
    }

    #[track_caller]
    pub fn act(&mut self, code: &str, token: &str, action: &Action) -> ActionResult {
        self.attempt(code, token, action).unwrap_or_else(|e| panic!("unexpected room error {:?}", e.code))
    }

    pub fn read_raw(&mut self, code: &str, token: Option<&str>, since: Option<&str>) -> ApiResponse {
        self.request(Route::State, code, token, since, "")
    }

    pub fn try_read(&mut self, code: &str, token: Option<&str>, since: u32) -> Result<SyncResponse, RoomError> {
        let since = since.to_string();
        parse(self.read_raw(code, token, Some(&since)))
    }

    #[track_caller]
    pub fn read(&mut self, code: &str, token: &str, since: u32) -> SyncResponse {
        self.try_read(code, Some(token), since).unwrap_or_else(|e| panic!("unexpected room error {:?}", e.code))
    }

    pub fn leave_raw(&mut self, code: &str, token: Option<&str>) -> ApiResponse {
        self.request(Route::Leave, code, token, None, "")
    }

    /// Create, seat everyone, deal. The usual starting point.
    pub fn deal(&mut self, names: &[&str]) -> (String, Vec<Seat>) {
        let (code, host) = self.create(names[0]);
        let mut seats = vec![host];
        for name in &names[1..] {
            seats.push(self.join(&code, name));
        }
        let host_token = seats[0].token.clone();
        self.start(&code, &host_token);
        (code, seats)
    }
}

pub fn parse<T: serde::de::DeserializeOwned>(response: ApiResponse) -> Result<T, RoomError> {
    if response.status == 200 {
        Ok(serde_json::from_str(&response.body).expect("a 200 body parses"))
    } else {
        Err(serde_json::from_str(&response.body).expect("an error body parses"))
    }
}

#[track_caller]
pub fn ok<T: serde::de::DeserializeOwned>(response: ApiResponse) -> T {
    assert_eq!(response.status, 200, "{}", response.body);
    serde_json::from_str(&response.body).unwrap()
}

pub fn projection(sync: &SyncResponse) -> &Projection {
    sync.room.projection().expect("expected a game, got a lobby")
}

/// Card ids a JSON payload shows that `viewer` may not see in `state`.
pub fn leaked(payload: &impl serde::Serialize, state: &GameState, viewer: &str) -> Vec<u64> {
    let mut visible = BTreeSet::new();
    visible_card_ids(&serde_json::to_value(payload).unwrap(), &mut visible);
    let hidden = hidden_card_ids(state, viewer);
    visible.intersection(&hidden).copied().collect()
}
