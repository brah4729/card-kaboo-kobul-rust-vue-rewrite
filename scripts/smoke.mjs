/**
 * The wire contract, over real HTTP and a real WebSocket, against a running Worker.
 *
 * `cargo test` drives `kaboo_room::api::handle` as a function, which catches
 * almost everything — but it never exercises the Worker's routing, the
 * Durable Object's storage, the socket or the JSON on the wire. This does.
 *
 *     bun run dev            # in one terminal (wrangler dev on :8787)
 *     bun run smoke          # in another
 *
 * Point it somewhere else with KABOO_BASE, e.g. the deployed workers.dev URL.
 */

const ORIGIN = process.env.KABOO_BASE ?? 'http://localhost:8787';
const BASE = `${ORIGIN}/api/rooms`;

const headers = (token) => ({
  'content-type': 'application/json',
  ...(token ? { 'x-kaboo-session': token } : {}),
});

async function call(path, { method = 'GET', body, token } = {}) {
  const response = await fetch(BASE + path, {
    method,
    headers: headers(token),
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  return { status: response.status, json: await response.json() };
}

let failed = 0;
const ok = (cond, label) => {
  if (!cond) failed += 1;
  console.log(`${cond ? 'PASS' : 'FAIL'}  ${label}`);
};
const view = (state) => state.json.room?.view;

/** A socket that has sent `auth`, with every message it has received so far. */
async function openSocket(code, token, since = 0) {
  const ws = new WebSocket(`${ORIGIN.replace(/^http/, 'ws')}/api/rooms/${code}/ws`);
  const messages = [];
  ws.onmessage = (event) => messages.push(JSON.parse(String(event.data)));
  await new Promise((resolve, reject) => {
    ws.onopen = resolve;
    ws.onerror = () => reject(new Error('socket failed to open'));
  });
  ws.send(JSON.stringify({ type: 'auth', token, since }));
  return { ws, messages };
}

/** Wait until `predicate(messages)` holds, or give up after `ms`. */
async function until(socket, predicate, ms = 3000) {
  const deadline = Date.now() + ms;
  while (Date.now() < deadline) {
    if (predicate(socket.messages)) return true;
    await new Promise((r) => setTimeout(r, 25));
  }
  return false;
}

try {
  // --- the page -------------------------------------------------------------
  const page = await fetch(`${ORIGIN}/some/deep/link`, { headers: { accept: 'text/html' } });
  ok(page.status === 200 && (await page.text()).includes('<'), 'a deep link serves the app (SPA fallback)');
  const unknownApi = await fetch(`${ORIGIN}/api/nothing-here`);
  ok(unknownApi.status === 404, `an unknown API path is a 404, not the app -> ${unknownApi.status}`);

  // --- create and join -----------------------------------------------------
  const created = await call('', { method: 'POST', body: { name: 'Ana' } });
  const { code, token: ana } = created.json;
  ok(created.status === 200 && !!ana && /^[A-Z2-9]{4}$/.test(code), `create -> ${created.status} ${code}`);

  const lobby = await call(`/${code}/state`, { token: ana });
  ok(
    lobby.json.room.status === 'lobby' && lobby.json.room.you === 'p1' && lobby.json.room.host,
    'the creator is seat p1 and the host',
  );
  ok(lobby.json.room.hostId === 'p1', 'the lobby names the host instead of leaving it to be inferred');

  // Ana's phone listens from here on.
  const anaSocket = await openSocket(code, ana);
  ok(await until(anaSocket, (m) => m.some((x) => x.room?.status === 'lobby')), 'the socket answers auth with the lobby');

  // Typed the way a human types it: lowercase, with padding.
  const joined = await call(`/ ${code.toLowerCase()} /join`, { method: 'POST', body: { name: 'Ben' } });
  const ben = joined.json.token;
  ok(joined.json.playerId === 'p2', `a lowercase, padded code joins the same room -> ${joined.json.playerId}`);
  ok(
    await until(anaSocket, (m) => m.some((x) => x.room?.roster?.length === 2)),
    'the join is PUSHED to the host, with no request from her',
  );

  // --- deal -----------------------------------------------------------------
  const dealt = await call(`/${code}/start`, { method: 'POST', token: ana });
  ok(view(dealt).phase === 'peek', 'the deal lands in peek');
  ok(JSON.stringify(view(dealt).peekableSlots) === '[2,3]', `peekable slots ${JSON.stringify(view(dealt).peekableSlots)}`);

  const benSocket = await openSocket(code, ben);
  ok(await until(benSocket, (m) => m.some((x) => x.room?.view?.phase === 'peek')), "Ben's socket opens on the dealt table");

  // --- the hidden-information boundary -------------------------------------
  const peekA = await call(`/${code}/actions`, { method: 'POST', token: ana, body: { action: { type: 'PEEK', slot: 3 } } });
  ok(peekA.json.events.length === 1 && peekA.json.events[0].to === 'p1', 'the peek is addressed to the seat that asked');
  ok(typeof peekA.json.events[0].seq === 'number', `the event carries a seq (${peekA.json.events[0].seq})`);
  const cardA = JSON.stringify(peekA.json.events[0].card);

  // A forged playerId in the body, over real HTTP.
  const peekB = await call(`/${code}/actions`, {
    method: 'POST',
    token: ben,
    body: { action: { type: 'PEEK', playerId: 'p1', slot: 3 } },
  });
  ok(peekB.json.events[0].to === 'p2', "Ben's forged playerId is discarded: the event goes to the session, not the body");

  const stateB = await call(`/${code}/state?since=0`, { token: ben });
  ok(!JSON.stringify(stateB.json).includes(cardA), "nothing in p2's payload is p1's card");
  ok(!benSocket.messages.some((m) => JSON.stringify(m).includes(cardA)), "nothing pushed to p2's socket is p1's card either");
  ok(
    view(stateB).players.find((p) => p.id === 'p1').hand.every((s) => s.face === null && !('id' in s)),
    "p1's hand is faceless and id-less to p2",
  );
  ok(stateB.json.events.length === 1 && stateB.json.events[0].to === 'p2', 'p2 receives exactly his own one event');

  // --- the cursor -----------------------------------------------------------
  const caught = await call(`/${code}/state?since=${stateB.json.eventSeq}`, { token: ben });
  ok(caught.json.events.length === 0, 'a caught-up cursor gets nothing');

  // --- ready, and into the turn --------------------------------------------
  await call(`/${code}/actions`, { method: 'POST', token: ana, body: { action: { type: 'READY' } } });
  const ready = await call(`/${code}/actions`, { method: 'POST', token: ben, body: { action: { type: 'READY' } } });
  ok(view(ready).phase === 'turnStart', `both ready -> ${view(ready).phase}`);
  ok(view(ready).turnPlayerId === 'p2', `round 1 opens on the player after the dealer: ${view(ready).turnPlayerId}`);
  ok(
    await until(anaSocket, (m) => m.some((x) => x.room?.view?.phase === 'turnStart')),
    "Ben's Ready is pushed to Ana's socket",
  );

  const offTurn = await call(`/${code}/actions`, { method: 'POST', token: ana, body: { action: { type: 'DRAW', from: 'deck' } } });
  ok(offTurn.status === 200 && !!offTurn.json.refusal, `drawing out of turn is refused -> ${offTurn.json.refusal?.code}`);

  const drew = await call(`/${code}/actions`, { method: 'POST', token: ben, body: { action: { type: 'DRAW', from: 'deck' } } });
  ok(view(drew).phase === 'drawn' && view(drew).drawn?.isYours === true, 'the drawer sees the card he drew');

  const drewA = await call(`/${code}/state`, { token: ana });
  ok(view(drewA).drawn?.isYours === false && view(drewA).drawn?.card === null, 'and the other seat sees only that a card was drawn');

  // --- refusals -------------------------------------------------------------
  const late = await call(`/${code}/join`, { method: 'POST', body: { name: 'Cal' } });
  ok(late.status === 409 && late.json.code === 'E_ALREADY_STARTED', `late join -> ${late.status} ${late.json.code}`);

  const noRoom = await call('/ZZZZ/join', { method: 'POST', body: { name: 'Cal' } });
  ok(noRoom.status === 404 && noRoom.json.code === 'E_NOT_FOUND', `join a code that is not a room -> ${noRoom.status}`);

  const forged = await openSocket(code, `${ana.slice(0, -2)}xx`);
  ok(
    await until(forged, (m) => m.some((x) => x.ok === false && x.code === 'E_BAD_SESSION')),
    'a socket with a forged token is told E_BAD_SESSION',
  );

  // --- reconnect ------------------------------------------------------------
  const before = (await call(`/${code}/state`, { token: ana })).json.eventSeq;
  const again = await openSocket(code, ana, before);
  ok(
    await until(again, (m) => m.some((x) => x.room?.you === 'p1' && x.events?.length === 0)),
    'a reconnecting socket with a current cursor is handed nothing twice',
  );

  for (const socket of [anaSocket, benSocket, forged, again]) socket.ws.close();
  console.log(`\n${failed === 0 ? 'all good' : `${failed} FAILED`} — room ${code}`);
} catch (error) {
  console.error(`\nCould not reach ${ORIGIN}. Is \`bun run dev\` running?\n`, error.message);
  process.exitCode = 1;
}

if (failed > 0) process.exitCode = 1;
