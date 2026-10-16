//! The room's Durable Object: storage, sockets and the alarm around
//! `kaboo_room`. It runs one event at a time and the runtime's input gate holds
//! other events while storage is awaited, so a read-decide-write here is never
//! interleaved with another — which is what settles the tap race.

use std::cell::RefCell;

use serde::{Deserialize, Serialize};
use worker::js_sys::{self, Reflect, Uint32Array};
use worker::wasm_bindgen::{self, JsCast, JsValue};
use worker::{
    Date, DurableObject, Env, Request, Response, ScheduledTime, State, WebSocket, WebSocketIncomingMessage,
    WebSocketPair, console_error, console_warn, durable_object,
};

use kaboo_protocol::{ClientMessage, RoomErrorCode, room_error};
use kaboo_room::api::{ApiRequest, Route, handle};
use kaboo_room::{Alarm, Deps, Push, RoomRecord, alarm, next_alarm, read};

use crate::{CODE_HEADER, TAKEN_HEADER};

const STORAGE_KEY: &str = "room";
/// Close code for a socket whose `auth` was refused. 4000–4999 is ours to use.
const CLOSE_REFUSED: u16 = 4001;

/// Who a socket belongs to. Stored WITH the socket, so it survives hibernation.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Attachment {
    code: String,
    /// `None` until the socket's `auth` message is accepted.
    player_id: Option<String>,
}

/// A uniform [0, 1) generator backed by `crypto.getRandomValues`, for the
/// engine's shuffle. Never `Math.random`: this shuffle decides who wins.
pub(crate) fn crypto_rng() -> impl FnMut() -> f64 {
    let crypto = Reflect::get(&js_sys::global(), &JsValue::from_str("crypto")).expect("crypto is global in Workers");
    let get_random_values: js_sys::Function = Reflect::get(&crypto, &JsValue::from_str("getRandomValues"))
        .expect("crypto.getRandomValues exists")
        .unchecked_into();
    let buf = Uint32Array::new_with_length(1);
    move || {
        get_random_values.call1(&crypto, &buf).expect("getRandomValues does not fail for 4 bytes");
        f64::from(buf.get_index(0)) / 4_294_967_296.0
    }
}

fn now() -> i64 {
    Date::now().as_millis() as i64
}

#[durable_object]
pub struct Room {
    state: State,
    env: Env,
    /// `None` = not loaded yet; `Some(None)` = loaded, and there is no room.
    cache: RefCell<Option<Option<RoomRecord>>>,
}

impl Room {
    /// The session signing key. Required: without it every instance would mint
    /// its own and sessions would break across restarts — the bug that once
    /// kicked players out seconds into a game.
    fn key(&self) -> Option<Vec<u8>> {
        let key = self
            .env
            .secret("ROOM_SECRET")
            .map(|s| s.to_string())
            .or_else(|_| self.env.var("ROOM_SECRET").map(|v| v.to_string()))
            .ok()
            .filter(|k| !k.is_empty());
        if key.is_none() {
            console_error!(
                "kaboo: ROOM_SECRET is not set. Run `wrangler secret put ROOM_SECRET` (or add it to .dev.vars)."
            );
        }
        key.map(String::into_bytes)
    }

    async fn load(&self) -> worker::Result<Option<RoomRecord>> {
        if let Some(cached) = self.cache.borrow().as_ref() {
            return Ok(cached.clone());
        }
        let stored: Option<String> = self.state.storage().get(STORAGE_KEY).await?;
        let record = match stored.map(|s| serde_json::from_str::<RoomRecord>(&s)) {
            Some(Ok(r)) => Some(r),
            Some(Err(e)) => {
                console_error!("kaboo: discarding an unreadable room record: {e}");
                None
            }
            None => None,
        };
        *self.cache.borrow_mut() = Some(record.clone());
        Ok(record)
    }

    /// Store the record (or delete the room) and point the alarm at the next deadline.
    async fn save(&self, record: &Option<RoomRecord>) -> worker::Result<()> {
        *self.cache.borrow_mut() = Some(record.clone());
        let storage = self.state.storage();
        match record {
            Some(r) => {
                let json = serde_json::to_string(r).map_err(|e| worker::Error::RustError(e.to_string()))?;
                storage.put(STORAGE_KEY, json).await?;
                let at = js_sys::Date::new(&JsValue::from_f64(next_alarm(r) as f64));
                storage.set_alarm(ScheduledTime::new(at)).await?;
            }
            None => storage.delete_all().await?,
        }
        Ok(())
    }

    /// Send each push to every socket authenticated as its player. A failed
    /// send is ignored: the move is already stored, and the next read or
    /// reconnect recovers anything a dropped push carried.
    fn deliver(&self, pushes: &[Push]) {
        if pushes.is_empty() {
            return;
        }
        for ws in self.state.get_websockets() {
            let Ok(Some(Attachment { player_id: Some(player), .. })) = ws.deserialize_attachment::<Attachment>() else {
                continue;
            };
            for push in pushes.iter().filter(|p| p.player_id == player) {
                if let Ok(json) = serde_json::to_string(&push.message) {
                    let _ = ws.send_with_str(json);
                }
            }
        }
    }

    fn upgrade(&self, code: &str) -> worker::Result<Response> {
        let pair = WebSocketPair::new()?;
        self.state.accept_web_socket(&pair.server);
        pair.server.serialize_attachment(Attachment { code: code.to_string(), player_id: None })?;
        Response::from_websocket(pair.client)
    }

    async fn on_auth(&self, ws: &WebSocket, code: &str, token: &str, since: u32) -> worker::Result<()> {
        let Some(key) = self.key() else {
            let _ = ws.close(Some(1011), Some("server misconfigured"));
            return Ok(());
        };
        let mut slot = self.load().await?;
        let mut rng = crypto_rng();
        let mut deps = Deps { now: now(), rng: &mut rng, key: &key };
        match read(&mut slot, code, Some(token), since, &mut deps) {
            Ok(outcome) => {
                ws.serialize_attachment(Attachment {
                    code: code.to_string(),
                    player_id: Some(outcome.value.room.you().to_string()),
                })?;
                ws.send_with_str(serde_json::to_string(&outcome.value)?)?;
                if outcome.changed {
                    self.save(&slot).await?;
                }
                self.deliver(&outcome.pushes);
            }
            Err(error) => {
                // A refused seat is final for this socket: the client decides
                // what to do from the code (E_NOT_FOUND / E_BAD_SESSION = lost seat).
                ws.send_with_str(serde_json::to_string(&error)?)?;
                let _ = ws.close(Some(CLOSE_REFUSED), Some("refused"));
            }
        }
        Ok(())
    }
}

impl DurableObject for Room {
    fn new(state: State, env: Env) -> Self {
        Self { state, env, cache: RefCell::new(None) }
    }

    async fn fetch(&self, req: Request) -> worker::Result<Response> {
        let code = req.headers().get(CODE_HEADER)?.unwrap_or_default();
        let url = req.url()?;
        let op = url.path().trim_start_matches('/').to_string();
        if op == "ws" {
            return self.upgrade(&code);
        }

        let method = req.method().to_string();
        let route = if op == "create" { Some(Route::Create) } else { Route::parse(&method, Some(&op)) };
        let Some(route) = route else { return Response::error("Not found", 404) };

        let Some(key) = self.key() else {
            return json_response(500, &serde_json::to_string(&room_error(RoomErrorCode::Busy))?);
        };
        let session = req.headers().get(kaboo_protocol::SESSION_HEADER)?;
        let since = url.query_pairs().find(|(k, _)| k == "since").map(|(_, v)| v.into_owned());
        let mut req = req;
        let body = req.bytes().await.unwrap_or_default();

        let mut slot = self.load().await?;
        let mut rng = crypto_rng();
        let mut deps = Deps { now: now(), rng: &mut rng, key: &key };
        let reply = handle(
            &mut slot,
            &ApiRequest { route, code: &code, session: session.as_deref(), since: since.as_deref(), body: &body },
            &mut deps,
        );

        if reply.changed {
            self.save(&slot).await?;
        }
        self.deliver(&reply.pushes);

        let mut response = json_response(reply.status, &reply.body)?;
        if reply.taken {
            response.headers_mut().set(TAKEN_HEADER, "1")?;
        }
        Ok(response)
    }

    async fn alarm(&self) -> worker::Result<Response> {
        let mut slot = self.load().await?;
        let mut rng = crypto_rng();
        // The alarm signs nothing, so it needs no key.
        let mut deps = Deps { now: now(), rng: &mut rng, key: &[] };
        match alarm(&mut slot, &mut deps) {
            Alarm::Ticked(pushes) => {
                self.save(&slot).await?;
                self.deliver(&pushes);
            }
            Alarm::Expired => {
                self.save(&None).await?;
                for ws in self.state.get_websockets() {
                    let _ = ws.close(Some(1000), Some("room expired"));
                }
            }
            Alarm::Idle => {
                // Woken early (or for a deadline something else already
                // resolved): just aim at the next one.
                if slot.is_some() {
                    self.save(&slot).await?;
                }
            }
        }
        Response::ok("")
    }

    async fn websocket_message(&self, ws: WebSocket, message: WebSocketIncomingMessage) -> worker::Result<()> {
        let WebSocketIncomingMessage::String(text) = message else { return Ok(()) };
        let Ok(Some(attachment)) = ws.deserialize_attachment::<Attachment>() else { return Ok(()) };
        match serde_json::from_str::<ClientMessage>(&text) {
            Ok(ClientMessage::Auth { token, since }) => self.on_auth(&ws, &attachment.code, &token, since).await,
            Err(_) => {
                console_warn!("kaboo: ignoring a socket message that is not a ClientMessage");
                Ok(())
            }
        }
    }

    async fn websocket_close(&self, _ws: WebSocket, _code: usize, _reason: String, _clean: bool) -> worker::Result<()> {
        Ok(())
    }

    async fn websocket_error(&self, _ws: WebSocket, _error: worker::Error) -> worker::Result<()> {
        Ok(())
    }
}

fn json_response(status: u16, body: &str) -> worker::Result<Response> {
    let mut response = Response::ok(body)?.with_status(status);
    response.headers_mut().set("content-type", "application/json")?;
    response.headers_mut().set("cache-control", "no-store")?;
    Ok(response)
}
