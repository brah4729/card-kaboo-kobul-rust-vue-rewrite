//! SESSIONS — how a device proves which seat is its own.
//!
//! `base64url(json payload).base64url(hmac-sha256)`, signed with the room
//! secret. Signed rather than "a random string plus a lookup table" so a
//! session needs no storage and carries the (room, player) pair it is scoped to.
//!
//! This is the ONLY thing standing between a player and another player's
//! cards: the room overwrites the `playerId` inside every incoming action with
//! the one verified here, so a forged body cannot act as someone else. It is
//! the security boundary, and it is written to fail closed.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use subtle::ConstantTimeEq;

use kaboo_protocol::Millis;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPayload {
    pub code: String,
    pub player_id: String,
    pub name: String,
    /// Issued at, epoch ms.
    pub iat: Millis,
}

/// Long enough to resume after a phone sleeps overnight; the room expires first anyway.
pub const MAX_AGE_MS: Millis = 24 * 60 * 60 * 1000;
/// A token from further in the future than this is a clock problem or a forgery.
const CLOCK_SKEW_MS: Millis = 60_000;

fn mac(key: &[u8], body: &str) -> Vec<u8> {
    let mut m = <Hmac<Sha256> as KeyInit>::new_from_slice(key).expect("HMAC takes a key of any length");
    m.update(body.as_bytes());
    m.finalize().into_bytes().to_vec()
}

pub fn sign_session(payload: &SessionPayload, key: &[u8]) -> String {
    let body = URL_SAFE_NO_PAD.encode(serde_json::to_vec(payload).expect("a payload always serializes"));
    let signature = URL_SAFE_NO_PAD.encode(mac(key, &body));
    format!("{body}.{signature}")
}

/// Verify a token, returning its payload or `None`.
///
/// `None` covers every failure — tampered, garbled, expired, wrong shape — and
/// the caller must not distinguish between them in a response.
pub fn verify_session(token: Option<&str>, now: Millis, key: &[u8]) -> Option<SessionPayload> {
    let token = token?;
    let (body, provided) = token.split_once('.')?;
    if body.is_empty() || provided.is_empty() {
        return None;
    }
    let provided = URL_SAFE_NO_PAD.decode(provided).ok()?;
    let expected = mac(key, body);
    // `ct_eq` on slices of different length is simply false, and says so in
    // constant time — a length check that panicked would itself be an oracle.
    if !bool::from(provided.ct_eq(&expected)) {
        return None;
    }

    let payload: SessionPayload = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(body).ok()?).ok()?;
    if payload.code.is_empty() || payload.player_id.is_empty() {
        return None;
    }
    if now - payload.iat > MAX_AGE_MS || payload.iat > now + CLOCK_SKEW_MS {
        return None;
    }
    Some(payload)
}
