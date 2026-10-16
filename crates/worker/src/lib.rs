//! The Cloudflare Worker. Two halves, both thin:
//!
//! * **The front** (`fetch`): axum routes `/api/*` to the room's Durable
//!   Object by its code. `POST /api/rooms` mints the code first and retries on
//!   a collision. Everything else is a static asset, served by the platform
//!   before this code ever runs (`run_worker_first = ["/api/*"]`).
//! * **The room** (`Room`): one Durable Object per code. It loads the record,
//!   hands the request to `kaboo_room::api::handle`, stores what changed, pushes
//!   to sockets, and sets its alarm for the next deadline.
//!
//! WebSocket upgrades skip axum and go straight to the room: the upgrade
//! response must reach the runtime untouched.

mod room;

use axum::extract::{Path, Request as AxumRequest};
use axum::http::{self, StatusCode};
use axum::routing::{any, post};
use axum::{Extension, Router};
use tower_service::Service;
use worker::{Env, Headers, HttpRequest, Method, Request, RequestInit, Stub, console_error, event, send};

use kaboo_protocol::{RoomErrorCode, make_room_code, normalize_room_code, room_error, status_of};

pub use room::Room;

/// The internal header that tells a room its own code. A Durable Object does
/// not reliably know the name it was reached by, so the front always says.
pub(crate) const CODE_HEADER: &str = "x-kaboo-code";
/// The room's answer to `create` when that code is already a live room.
pub(crate) const TAKEN_HEADER: &str = "x-kaboo-taken";
/// How many codes to try before giving up with `E_BUSY`. Four characters from
/// a 31-letter alphabet is ~920k codes, so a collision at all is rare.
const CODE_ATTEMPTS: usize = 6;
/// The largest request body any route accepts.
const BODY_LIMIT: usize = 16 * 1024;

type Body = axum::body::Body;
type Reply = http::Response<Body>;

#[event(fetch)]
async fn fetch(req: HttpRequest, env: Env, _ctx: worker::Context) -> worker::Result<Reply> {
    let path = req.uri().path().to_string();
    if let Some(code) = ws_code(&path) {
        let stub = room_stub(&env, &code)?;
        // An incoming request's headers are immutable, so the upgrade is
        // re-issued with a copy (keeping `Upgrade` and friends) plus the code.
        let incoming = Request::try_from(req)?;
        let headers = Headers::new();
        for (name, value) in incoming.headers().entries() {
            headers.set(&name, &value)?;
        }
        headers.set(CODE_HEADER, &code)?;
        let mut init = RequestInit::new();
        init.with_method(Method::Get).with_headers(headers);
        let upgrade = Request::new_with_init("https://room/ws", &init)?;
        return Ok(Reply::from(stub.fetch_with_request(upgrade).await?));
    }

    let mut router = Router::new()
        .route("/api/rooms", post(create))
        .route("/api/rooms/{code}/{op}", any(forward))
        .fallback(|| async { (StatusCode::NOT_FOUND, "Not found") })
        .layer(Extension(send::SendWrapper::new(env)));
    Ok(router.call(req).await?)
}

/// `/api/rooms/{code}/ws` → the normalized code.
fn ws_code(path: &str) -> Option<String> {
    let rest = path.strip_prefix("/api/rooms/")?;
    let (code, op) = rest.split_once('/')?;
    (op == "ws").then(|| normalize_room_code(code))
}

fn room_stub(env: &Env, code: &str) -> worker::Result<Stub> {
    env.durable_object("ROOM")?.id_from_name(code)?.get_stub()
}

fn error_reply(code: RoomErrorCode) -> Reply {
    let body = serde_json::to_string(&room_error(code)).expect("an error serializes");
    http::Response::builder()
        .status(status_of(code))
        .header("content-type", "application/json")
        .body(Body::from(body))
        .expect("a static response builds")
}

/// Re-issue an axum request to a room as `https://room/{op}?{query}`.
async fn to_room(env: &Env, code: &str, op: &str, req: AxumRequest) -> worker::Result<Reply> {
    let (parts, body) = req.into_parts();
    let bytes = axum::body::to_bytes(body, BODY_LIMIT).await.map_err(|e| worker::Error::RustError(e.to_string()))?;

    let headers = Headers::new();
    headers.set(CODE_HEADER, code)?;
    if let Some(session) = parts.headers.get(kaboo_protocol::SESSION_HEADER).and_then(|v| v.to_str().ok()) {
        headers.set(kaboo_protocol::SESSION_HEADER, session)?;
    }
    let mut init = RequestInit::new();
    init.with_method(Method::from(parts.method.to_string())).with_headers(headers);
    if !bytes.is_empty() {
        init.with_body(Some(worker::js_sys::Uint8Array::from(bytes.as_ref()).into()));
    }
    let query = parts.uri.query().map(|q| format!("?{q}")).unwrap_or_default();
    let inner = Request::new_with_init(&format!("https://room/{op}{query}"), &init)?;
    Ok(Reply::from(room_stub(env, code)?.fetch_with_request(inner).await?))
}

#[send]
async fn forward(
    Extension(env): Extension<send::SendWrapper<Env>>,
    Path((code, op)): Path<(String, String)>,
    req: AxumRequest,
) -> Reply {
    let code = normalize_room_code(&code);
    // An empty code can never be a room; do not wake a Durable Object for it.
    if code.is_empty() {
        return error_reply(RoomErrorCode::NotFound);
    }
    to_room(&env, &code, &op, req).await.unwrap_or_else(|e| {
        console_error!("kaboo: forwarding to room {code} failed: {e}");
        error_reply(RoomErrorCode::Busy)
    })
}

/// Mint a code, and ask that room to be created. On a collision, mint another.
#[send]
async fn create(Extension(env): Extension<send::SendWrapper<Env>>, req: AxumRequest) -> Reply {
    let body = match axum::body::to_bytes(req.into_body(), BODY_LIMIT).await {
        Ok(b) => b,
        Err(_) => return error_reply(RoomErrorCode::BadRequest),
    };
    let mut rng = room::crypto_rng();
    for _ in 0..CODE_ATTEMPTS {
        let code = make_room_code(&mut rng);
        let attempt = AxumRequest::builder().method("POST").body(Body::from(body.clone())).expect("request builds");
        match to_room(&env, &code, "create", attempt).await {
            Ok(reply) if reply.headers().contains_key(TAKEN_HEADER) => continue,
            Ok(reply) => return reply,
            Err(e) => {
                console_error!("kaboo: creating room {code} failed: {e}");
                return error_reply(RoomErrorCode::Busy);
            }
        }
    }
    error_reply(RoomErrorCode::Busy)
}
