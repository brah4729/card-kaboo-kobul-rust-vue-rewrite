//! The session token is the security boundary: the room overwrites the
//! `playerId` inside every incoming action with the one verified here, so if
//! verification can be talked into accepting a forged token, a player can act
//! as another player and read their cards.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use kaboo_room::{SessionPayload, sign_session, verify_session};

const KEY: &[u8] = b"test-secret-not-a-real-one";
const NOW: i64 = 1_700_000_000_000;

fn payload() -> SessionPayload {
    SessionPayload { code: "TEST".into(), player_id: "p2".into(), name: "Two".into(), iat: NOW }
}

fn verify(token: &str, now: i64) -> Option<SessionPayload> {
    verify_session(Some(token), now, KEY)
}

#[test]
fn round_trips_the_payload_it_was_given() {
    assert_eq!(verify(&sign_session(&payload(), KEY), NOW), Some(payload()));
}

#[test]
fn gives_different_tokens_to_different_players() {
    let a = sign_session(&SessionPayload { player_id: "p1".into(), ..payload() }, KEY);
    let b = sign_session(&payload(), KEY);
    assert_ne!(a, b);
    assert_eq!(verify(&a, NOW).unwrap().player_id, "p1");
    assert_eq!(verify(&b, NOW).unwrap().player_id, "p2");
}

#[test]
fn rejects_a_payload_edited_after_signing() {
    let token = sign_session(&payload(), KEY);
    let (_, signature) = token.split_once('.').unwrap();
    // Re-encode the body with a different player and keep the old signature —
    // the shape a player would produce by hand-editing localStorage.
    let forged = SessionPayload { player_id: "p1".into(), name: "One".into(), ..payload() };
    let body = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&forged).unwrap());
    assert_eq!(verify(&format!("{body}.{signature}"), NOW), None);
}

#[test]
fn rejects_an_edited_signature() {
    let token = sign_session(&payload(), KEY);
    let (body, signature) = token.split_once('.').unwrap();
    let mut chars: Vec<char> = signature.chars().collect();
    let last = chars.len() - 2; // not the final char: its low bits are padding
    chars[last] = if chars[last] == 'A' { 'B' } else { 'A' };
    let flipped: String = chars.into_iter().collect();
    assert_eq!(verify(&format!("{body}.{flipped}"), NOW), None);
}

#[test]
fn rejects_a_token_signed_with_another_key() {
    let token = sign_session(&payload(), b"some-other-deployment");
    assert_eq!(verify(&token, NOW), None);
}

#[test]
fn rejects_an_expired_token() {
    let token = sign_session(&payload(), KEY);
    assert!(verify(&token, NOW + 60_000).is_some());
    assert_eq!(verify(&token, NOW + 25 * 60 * 60 * 1000), None);
}

#[test]
fn rejects_a_token_dated_in_the_future() {
    let token = sign_session(&SessionPayload { iat: NOW + 10 * 60_000, ..payload() }, KEY);
    assert_eq!(verify(&token, NOW), None);
}

#[test]
fn rejects_anything_that_is_not_one_of_our_tokens() {
    assert_eq!(verify_session(None, NOW, KEY), None);
    let unsigned = format!("{}.{}", URL_SAFE_NO_PAD.encode("{}"), URL_SAFE_NO_PAD.encode("nope"));
    for bad in ["", "nonsense", ".", ".signature-only", "body-only.", "a.b.c", unsigned.as_str()] {
        assert_eq!(verify(bad, NOW), None, "{bad:?} must not verify");
    }
}

#[test]
fn rejects_a_correctly_signed_body_that_is_not_a_session() {
    // Signed by us, so the signature passes — the shape check has to catch it,
    // or a hand-built payload could smuggle in a missing playerId.
    for junk in [r#"{"code":"TEST"}"#, r#"{"code":"","playerId":"p1","name":"x","iat":0}"#] {
        let body = URL_SAFE_NO_PAD.encode(junk);
        let signature = {
            use hmac::{Hmac, KeyInit, Mac};
            let mut m = <Hmac<sha2::Sha256> as KeyInit>::new_from_slice(KEY).unwrap();
            m.update(body.as_bytes());
            URL_SAFE_NO_PAD.encode(m.finalize().into_bytes())
        };
        assert_eq!(verify(&format!("{body}.{signature}"), NOW), None, "{junk}");
    }
}
