//! THE HTTP EDGE — `api::handle`, called the way the Durable Object calls it.
//!
//! Two things exist only at this layer:
//! * **`x-kaboo-session` is the identity.** This is where the header is read and
//!   the body's `playerId` discarded. A mistake here is a player who can act as
//!   another player.
//! * **Every failure leaves as a generic sentence.** A status code is fine to
//!   expose; a reason is not.

mod common;

use common::{Seat, Server, leaked, ok};
use kaboo_protocol::{ROOM_CODE_ALPHABET, RoomErrorCode, Seated, room_error_message};
use kaboo_room::api::Route;
use serde_json::{Value, json};

fn body(response: &kaboo_room::api::ApiResponse) -> Value {
    serde_json::from_str(&response.body).unwrap()
}

fn lobby(server: &mut Server, names: &[&str]) -> (String, Vec<Seat>) {
    let (code, host) = server.create(names[0]);
    let mut seats = vec![host];
    for name in &names[1..] {
        seats.push(server.join(&code, name));
    }
    (code, seats)
}

fn playing(server: &mut Server, names: &[&str]) -> (String, Vec<Seat>) {
    let (code, seats) = lobby(server, names);
    let token = seats[0].token.clone();
    assert_eq!(server.start_raw(&code, Some(&token)).status, 200);
    (code, seats)
}

fn act_json(server: &mut Server, code: &str, token: &str, action: Value) -> kaboo_room::api::ApiResponse {
    server.act_raw(code, Some(token), &action)
}

// --- creating and joining ----------------------------------------------------------

#[test]
fn mints_a_room_a_seat_and_a_session() {
    let mut server = Server::new(7);
    let seated: Seated = ok(server.create_raw(r#"{"name":"Ana"}"#));
    assert_eq!(seated.code.len(), 4);
    assert!(seated.code.chars().all(|c| ROOM_CODE_ALPHABET.contains(c)));
    assert_eq!(seated.player_id, "p1");
    let (body, signature) = seated.token.split_once('.').unwrap();
    let url_safe = |s: &str| s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    assert!(url_safe(body) && url_safe(signature) && !body.is_empty() && !signature.is_empty());
}

#[test]
fn retries_a_code_that_is_already_a_room() {
    let mut server = Server::new(7);
    let (code, _) = server.create("Ana");
    let again = server.request(Route::Create, &code, None, None, r#"{"name":"Bo"}"#);
    assert!(again.taken, "the Worker must be told to mint another code");
    assert_eq!(server.record(&code).unwrap().roster[0].name, "Ana", "the existing room is untouched");
}

#[test]
fn seats_players_in_join_order_and_refuses_a_ninth() {
    let mut server = Server::new(7);
    let (code, seats) = lobby(&mut server, &["p1", "p2", "p3", "p4", "p5", "p6", "p7", "p8"]);
    let ids: Vec<_> = seats.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, vec!["p1", "p2", "p3", "p4", "p5", "p6", "p7", "p8"]);
    let ninth = server.join_raw(&code, "p9");
    assert_eq!(ninth.status, 409);
    assert_eq!(body(&ninth)["code"], "E_ROOM_FULL");
    assert_eq!(body(&ninth)["ok"], false);
}

#[test]
fn says_who_the_host_is_and_it_follows_the_seat_when_the_host_leaves() {
    let mut server = Server::new(7);
    let (code, seats) = lobby(&mut server, &["Ana", "Bo", "Cal"]);
    let first = body(&server.read_raw(&code, Some(&seats[1].token), None));
    assert_eq!(first["room"]["host"], false);
    assert_eq!(first["room"]["hostId"], "p1");

    assert_eq!(server.leave_raw(&code, Some(&seats[0].token)).status, 200);
    let after = body(&server.read_raw(&code, Some(&seats[1].token), None));
    assert_eq!(after["room"]["host"], true);
    assert_eq!(after["room"]["hostId"], "p2");
}

#[test]
fn accepts_a_code_the_way_a_human_typed_it() {
    let mut server = Server::new(7);
    let (code, seats) = lobby(&mut server, &["Ana", "Bo"]);
    let lower = code.to_lowercase();
    let messy = format!(" {}-{} ", &lower[..2], &lower[2..]);
    assert_eq!(server.read_raw(&messy, Some(&seats[0].token), None).status, 200);
}

#[test]
fn refuses_a_name_that_is_empty_missing_or_not_a_string() {
    let mut server = Server::new(7);
    for bad in [r#"{"name":"   "}"#, "{}", r#"{"name":42}"#, r#"{"name":null}"#] {
        let response = server.create_raw(bad);
        assert_eq!(response.status, 400, "{bad}");
        assert_eq!(body(&response)["code"], "E_BAD_REQUEST");
    }
}

#[test]
fn treats_an_unparseable_body_as_an_empty_one_not_a_crash() {
    let mut server = Server::new(7);
    let response = server.create_raw("{not json");
    assert_eq!(response.status, 400);
    assert_eq!(body(&response)["code"], "E_BAD_REQUEST");
}

// --- the session header is the identity ----------------------------------------------

#[test]
fn refuses_a_request_with_no_session_at_all() {
    let mut server = Server::new(7);
    let (code, _) = lobby(&mut server, &["Ana", "Bo"]);
    let response = server.read_raw(&code, None, None);
    assert_eq!(response.status, 401);
    assert_eq!(body(&response)["code"], "E_BAD_SESSION");
}

#[test]
fn refuses_a_session_minted_for_another_room() {
    let mut server = Server::new(7);
    let (_, a) = lobby(&mut server, &["Ana", "Bo"]);
    let (b, _) = lobby(&mut server, &["Cy", "Di"]);
    assert_eq!(server.read_raw(&b, Some(&a[0].token), None).status, 401);
}

#[test]
fn ignores_a_player_id_forged_into_the_action_body() {
    let mut server = Server::new(7);
    let (code, seats) = playing(&mut server, &["Ana", "Bo"]);
    // Finish the peek, so there is a real turn to steal. The browser sends no
    // playerId at all: the session says who it is.
    for seat in &seats {
        for slot in [2, 3] {
            act_json(&mut server, &code, &seat.token, json!({ "type": "PEEK", "slot": slot }));
        }
        act_json(&mut server, &code, &seat.token, json!({ "type": "READY" }));
    }
    let opening = body(&server.read_raw(&code, Some(&seats[0].token), None));
    assert_eq!(opening["room"]["view"]["turnPlayerId"], "p2");

    // Ana claims to be Bo, who is on turn. The header must win.
    let forged =
        act_json(&mut server, &code, &seats[0].token, json!({ "type": "DRAW", "playerId": "p2", "from": "deck" }));
    assert_eq!(forged.status, 200);
    let result = body(&forged);
    assert_eq!(result["room"]["you"], "p1");
    assert_eq!(result["refusal"]["code"], "E_NOT_YOUR_TURN");
    assert!(result["room"]["view"]["drawn"].is_null(), "a forged action moved the game");
    assert_eq!(result["room"]["view"]["phase"], "turnStart");

    // The real owner can still move, with a body that lies the other way.
    let real = body(&act_json(
        &mut server,
        &code,
        &seats[1].token,
        json!({ "type": "DRAW", "playerId": "p1", "from": "deck" }),
    ));
    assert!(real.get("refusal").is_none());
    assert_eq!(real["room"]["view"]["drawn"]["by"], "p2");
}

#[test]
fn refuses_the_deal_from_anyone_but_the_host() {
    let mut server = Server::new(7);
    let (code, seats) = lobby(&mut server, &["Ana", "Bo"]);
    let response = server.start_raw(&code, Some(&seats[1].token));
    assert_eq!(response.status, 401);
    assert_eq!(body(&response)["code"], "E_BAD_SESSION");
}

#[test]
fn refuses_to_deal_to_a_table_of_one() {
    let mut server = Server::new(7);
    let (code, host) = server.create("Alone");
    assert_eq!(server.start_raw(&code, Some(&host.token)).status, 400);
}

#[test]
fn refuses_a_second_deal() {
    let mut server = Server::new(7);
    let (code, seats) = playing(&mut server, &["Ana", "Bo"]);
    let again = server.start_raw(&code, Some(&seats[0].token));
    assert_eq!(again.status, 409);
    assert_eq!(body(&again)["code"], "E_ALREADY_STARTED");
}

// --- a rejection never explains itself --------------------------------------------------

#[test]
fn sends_the_canned_sentence_for_every_failure() {
    let mut server = Server::new(7);
    let (code, seats) = lobby(&mut server, &["Ana", "Bo"]);
    let (full, _) = lobby(&mut server, &["f1", "f2", "f3", "f4", "f5", "f6", "f7", "f8"]);
    let (started, _) = playing(&mut server, &["s1", "s2"]);

    let failures = [
        (server.read_raw(&code, None, None), RoomErrorCode::BadSession, "E_BAD_SESSION", 401),
        (server.read_raw("ZZZZ", Some(&seats[0].token), None), RoomErrorCode::BadSession, "E_BAD_SESSION", 401),
        (server.start_raw(&code, Some(&seats[1].token)), RoomErrorCode::BadSession, "E_BAD_SESSION", 401),
        (server.join_raw("ZZZZ", "x"), RoomErrorCode::NotFound, "E_NOT_FOUND", 404),
        (server.join_raw(&full, "x"), RoomErrorCode::RoomFull, "E_ROOM_FULL", 409),
        (server.join_raw(&started, "x"), RoomErrorCode::AlreadyStarted, "E_ALREADY_STARTED", 409),
        (
            act_json(&mut server, &code, &seats[0].token, json!({ "type": "DRAW" })),
            RoomErrorCode::BadRequest,
            "E_BAD_REQUEST",
            400,
        ),
    ];
    for (response, code, wire, status) in failures {
        let payload = body(&response);
        assert_eq!(payload["code"], wire);
        assert_eq!(response.status, status, "{wire}");
        assert_eq!(payload["ok"], false);
        // Byte-for-byte the canned sentence: nothing was appended to say why.
        assert_eq!(payload["message"], room_error_message(code));
    }
}

#[test]
fn answers_a_refusal_during_play_with_the_fresh_view_as_a_success() {
    let mut server = Server::new(7);
    let (code, seats) = playing(&mut server, &["Ana", "Bo"]);
    let response = act_json(&mut server, &code, &seats[0].token, json!({ "type": "DRAW", "from": "deck" }));
    assert_eq!(response.status, 200);
    let payload = body(&response);
    assert!(payload.get("refusal").is_some());
    assert!(payload.get("room").is_some());
}

#[test]
fn rejects_an_action_that_is_not_an_action() {
    let mut server = Server::new(7);
    let (code, seats) = playing(&mut server, &["Ana", "Bo"]);
    for bad in [
        r#"{"action":"DRAW"}"#,
        r#"{"action":7}"#,
        r#"{"action":null}"#,
        "{}",
        r#"{"action":{}}"#,
        r#"{"action":{"type":"FLY"}}"#,
    ] {
        let response = server.request(Route::Actions, &code, Some(&seats[0].token), None, bad);
        assert_eq!(response.status, 400, "{bad}");
        assert_eq!(body(&response)["code"], "E_BAD_REQUEST");
    }
}

// --- no response body carries another player's card ----------------------------------------

#[test]
fn keeps_every_seat_blind_through_a_deal_and_a_peek() {
    let mut server = Server::new(7);
    let (code, seats) = playing(&mut server, &["Ana", "Bo", "Cy"]);
    let peeked = body(&act_json(&mut server, &code, &seats[0].token, json!({ "type": "PEEK", "slot": 2 })));
    assert_eq!(peeked["events"].as_array().unwrap().len(), 1);
    assert_eq!(peeked["events"][0]["to"], "p1");
    assert!(peeked["events"][0].get("card").is_some(), "the peek must actually reveal something");

    let mut hidden_seen = 0;
    for seat in &seats {
        let state = server.state(&code).clone();
        hidden_seen += kaboo_engine::testkit::hidden_card_ids(&state, &seat.id).len();
        let payload = body(&server.read_raw(&code, Some(&seat.token), None));
        let bad = leaked(&payload["room"], &state, &seat.id);
        assert!(bad.is_empty(), "{} was sent cards {bad:?}", seat.id);
        for event in payload["events"].as_array().unwrap() {
            assert_eq!(event["to"], seat.id.as_str());
        }
    }
    assert!(hidden_seen > 0, "no seat had anything hidden to leak");
}

// --- reading state ------------------------------------------------------------------------

#[test]
fn treats_an_unusable_cursor_as_the_beginning_never_as_you_missed_it() {
    let mut server = Server::new(7);
    let (code, seats) = playing(&mut server, &["Ana", "Bo"]);
    let token = seats[0].token.clone();
    act_json(&mut server, &code, &token, json!({ "type": "PEEK", "slot": 2 }));
    // A cursor is the client's own bookkeeping, so a garbled one is "from the start".
    for since in [None, Some(""), Some("abc"), Some("-5"), Some("1e999"), Some("0")] {
        let response = server.read_raw(&code, Some(&token), since);
        assert_eq!(response.status, 200, "{since:?}");
        assert_eq!(body(&response)["events"].as_array().unwrap().len(), 1, "cursor {since:?} lost the peek");
    }
}
