# AGENTS.md — Kaboo

> **Single source of truth for this project.** Read this file first, before touching anything.
> Trust it, but verify against the code. If code and this file disagree, fix whichever is wrong
> and say so in the commit message. Never leave them diverged.
> Other agent files (`CLAUDE.md`, `GEMINI.md`, …) are one-line pointers to this file.

---

## 1. Project summary

**Kaboo** is a browser-based, real-time multiplayer implementation of the card game *Kaboo*
(a Cabo-like "memory + slap" game). Two to **8** players play in the same room via a short
**room code**. **No accounts, no install, no app store.**

- **Cross-device, always.** Each player opens the site on **their own** phone or desktop, picks a
  name, and enters the shared code. There is no "pass the phone" mode and there will not be one.
- **Audience:** friends on phones, in the same room or on different continents. **Mobile-first.**
- **Stack (since the 2026-10-16 rewrite):** a **Rust** game engine and room layer, running on
  **Cloudflare Workers** (free plan) with **one Durable Object per room**; an **axum** router in
  front; a **Vue 3 + TypeScript** single-page app served by the same Worker as static assets.
  The previous Next.js/Vercel/Upstash/Ably build is tagged **`legacy-next`** (`git show legacy-next:<path>`).
- **Non-goals (do not build unless asked):** accounts, bots, chat, matchmaking, spectators,
  peer-to-peer/WebRTC, monetization.

---

## 2. Current status

| Milestone | Scope | Status |
|---|---|---|
| **R0** | Spike: workers-rs Durable Object + hibernating WebSocket + alarm + axum | ✅ **done** |
| **R1** | Clear the Next app, scaffold the Cargo workspace, `web/` and `wrangler.toml` | ✅ **done** |
| **R2** | Engine port to Rust + every engine suite | ✅ **done** — 151 tests, mutation-checked |
| **R3** | Protocol + room + sessions + the HTTP edge | ✅ **done** — 66 tests |
| **R4** | Worker + Durable Object + WebSocket + alarm | ✅ **done** — `bun run smoke` 29/29 against `wrangler dev`; alarm verified firing with every client idle |
| **R5** | Vue frontend | ✅ **done** — 34 web tests; every effect sub-flow driven in two real browsers (`scripts/ui-drive.js`) |
| **R6** | Deploy to `*.workers.dev`, real phones | 🔴 **blocked on the owner** — needs `wrangler login` (§10) |
| **M5** | Up to 8 players: the 3–8 seat layout around the table | ⚪ **not started** — engine and transport support 8 today |

`bun run verify` is green: `cargo fmt --check`, clippy with `-D warnings` (native **and** wasm32),
every Rust test, the generated-types check, and the web typecheck, lint and tests.

**Server-authoritative from end to end.** The browser holds no `GameState`; every fact on screen
arrived as a `RoomView` projected for one seat, so a card that player may not see was never
transmitted. Moves are HTTP POSTs, the room lives in its Durable Object, and each player's socket
receives only that player's projection.

**Not yet proven:** anything on Cloudflare's real network. Everything above ran under `wrangler dev`
(workerd locally, SQLite-backed Durable Objects, real WebSockets, real alarms) — the same runtime, but
not the same edge. No real phone has touched the Vue build.

---

## 3. How to run, test, lint, build

```bash
bun install              # JS deps (root: wrangler; web/: Vue, Vite, Vitest, ESLint)
bun run dev              # writes .dev.vars (ROOM_SECRET) if missing, builds web/, wrangler dev → http://127.0.0.1:8787
bun run dev:web          # Vite with hot reload on :5173, proxying /api (HTTP + WS) to wrangler dev
bun run smoke            # the wire contract over real HTTP + WebSocket — needs `bun run dev` running
bun run test             # cargo test (engine, protocol, room) + vitest (web)
bun run verify           # fmt + clippy (native & wasm32) + tests + generated-types diff + web checks ← before every commit
bun run build            # web/dist + the Worker's wasm (crates/worker/build)
bun run deploy           # build + wrangler deploy
bun run cards            # re-render web/public/cards/ from the pinned upstream deck (§8)
```

- **Package manager: `bun`** (`bun.lock`). Root `package.json` holds wrangler and the scripts; `web/`
  is a bun workspace with its own `package.json`. Use `cd web && bun run …`; `bun --cwd web run` is
  not accepted by this bun.
- **Rust:** stable, edition 2024, with the `wasm32-unknown-unknown` target and `worker-build`
  (`cargo install worker-build --locked`). `cargo test --workspace --exclude kaboo-worker` — the
  worker crate only builds for wasm32.
- **Windows (the owner's machine):** the GNU host toolchain needs MinGW's `dlltool.exe` on `PATH`
  (WinLibs, via winget) or `windows-sys`/`getrandom` fail to build. And `wrangler dev` reports an
  opaque `internal error` from a Durable Object whose SQLite file path passes Windows' MAX_PATH —
  keep the repo at a short path (or pass `--persist-to` somewhere short).
- **`wrangler dev` does not re-index `web/dist`.** After `bun run web:build`, restart wrangler, or
  the page asks for a hashed bundle the asset manifest does not know and gets `index.html` back
  (a MIME-type error in the console). `bun run dev` does both in order.
- **`ts-rs` writes `web/src/generated/`** whenever `cargo test` runs (`.cargo/config.toml` sets
  `TS_RS_EXPORT_DIR`). Commit the result; `verify` fails if the Rust types moved and the generated
  TypeScript was not committed with them. Never edit those files by hand.
- **Driving the UI:** `scripts/ui-drive.js` is a Playwright function (run it through Playwright's
  `browser_run_code` with the file) that opens two isolated phone-sized contexts and plays create →
  join → deal → peek → every effect → skip → an off-turn tap, asserting as it goes.

**Secrets.** One, required: **`ROOM_SECRET`**, the HMAC key that signs session tokens.
Production: `wrangler secret put ROOM_SECRET`. Local: `.dev.vars` (gitignored), written by
`scripts/dev-secret.mjs`. Without it a room answers `500` and logs why — there is deliberately no
fallback key (§7).

---

## 4. Architecture and file map

```
Cargo.toml            workspace: engine · protocol · room · worker
crates/
  engine/             ← PURE Rust. No IO, no clock, no randomness of its own (all via `Ctx`).
    src/types.rs        GameState · Action (serde-tagged, SCREAMING_SNAKE) · RuleConfig · LogEntry …
    src/reduce.rs       apply_action · tick · next_deadline · start_game      ← the public API
    src/project.rs      project · legal_actions · peekable_slots              ← the hidden-info boundary
    src/state.rs        create_game · deal · PEEK_SLOTS · MIN/MAX_PLAYERS · slot mutation (bumps gen)
    src/turn.rs         advance_turn · KABOO final turns · settle_loose_cards · reveal_round
    src/effects.rs      7/8 · 9/10 · J · Q · K
    src/tap.rs          claim · choose · place · decline · timeout penalty
    src/cards.rs · rules.rs · scoring.rs · errors.rs · events.rs
    src/testkit.rs      (feature `testkit`) mulberry32 · Table · arrange · invariants · fuzzer · choose_move
    tests/              cards · engine · kaboo · tap · projection · properties (proptest) · client_contract
  protocol/           ← THE WIRE CONTRACT: RoomView · SyncResponse · DeliveredEvent · RoomError · ClientMessage
                        · status_of · room codes. ts-rs exports every type to web/src/generated/.
  room/               ← every use case, framework-free, natively tested
    src/lib.rs          RoomRecord · create/join/start/act/read/leave · alarm · next_alarm · sync_for · fan-out
    src/session.rs      HMAC-SHA256 session tokens (constant-time compare)
    src/api.rs          the HTTP edge: Route · handle(slot, request, deps) → status + JSON + pushes
    tests/              session · room · http · transport_game (whole games at 2–8 players ← load-bearing)
  worker/             ← the Cloudflare Worker (wasm32 only)
    src/lib.rs          #[event(fetch)]: axum routes /api/*; WebSocket upgrades go straight to the room
    src/room.rs         the `Room` Durable Object: storage, sockets (hibernating), alarm, crypto RNG
web/                  ← Vue 3 + TypeScript SPA (Vite)
  src/App.vue           Join → Room (Lobby | Game). One question: is there a stored seat?
  src/components/       RoomScreen (mounts useGame once) · LobbyScreen · JoinScreen · GameScreen ·
                        ActionBar · Card · Hand · CardFlights · PeekTray · StatusPill · Scoreboard · GameLog
  src/lib/              api.ts (the only fetch) · socket.ts (the room WebSocket + HTTP fallback) ·
                        useGame.ts (the composable) · session.ts · intent.ts · flights.ts (pure) · cards.ts · protocol.ts
  src/generated/        ts-rs output — never edit
  src/styles/           tokens.css ← the ONLY place raw values are defined. game.css is layout only.
  public/cards/         54 PNGs (52 + 2 jokers), public domain — see public/cards/README.md
  tests/                flights · intent (incl. the tap-bug regression) · socket + notice labels
scripts/  smoke.mjs (HTTP + WS against a running Worker) · ui-drive.js (two browsers) · dev-secret.mjs · cards.mjs
wrangler.toml         Worker + assets (SPA fallback, /api/* to the Worker first) + ROOM binding + SQLite migration
```

**Request flow.** `POST /api/rooms` → the Worker mints a code and asks that room's Durable Object to
`create` (a collision answers `x-kaboo-taken` and the Worker mints another). Every other
`/api/rooms/:code/:op` is forwarded to the Durable Object named by the normalized code, which loads
its `RoomRecord`, calls `kaboo_room::api::handle`, stores what changed, pushes to the other seats'
sockets, and points its alarm at `next_alarm(record)`. `GET /api/rooms/:code/ws` is a WebSocket
upgrade, handed to the same Durable Object untouched.

**The socket protocol.** Client → server: only `{"type":"auth","token","since"}` (the first message;
again on wake to resync). Server → client: a `SyncResponse` (the same shape as every HTTP answer) or
a `RoomError` (`E_NOT_FOUND` / `E_BAD_SESSION` = the seat is gone; the socket is then closed).
Moves never go over the socket — a move has one path, and it is the one that returns `refusal`.

**Engine public API — keep it stable.** `apply_action(&state, &action, &mut ctx)` and
`tick(&state, &mut ctx)` return `ApplyResult { state, events, error, version_changed }` (`tick`
returns `None` when nothing was due). `start_game(code, seats, ctx, rules)` → `GameState`;
`project(&state, viewer)` → `Projection`; `next_deadline(&state)` → the instant `tick` would next act.
`Ctx { now, rng }`. **Rules live on the state.** `apply_action` is **pure and total**: it clones,
never mutates its input, never panics on a bad action. A refused action that *did* advance the clock
still returns `version_changed: true` with the moved state, so a timeout is never silently dropped.

**There is no `JOIN` action and no `lobby` phase in the engine.** `create_game` takes 2–8 seats and
deals immediately. Joining is a **room-layer** concern: `RoomRecord.roster` accumulates and the host's
`start_room` hands the engine the finished list.

**Phases** (explicit, no scattered booleans):
`peek → turnStart → drawn → effect → finalSwap → reveal → gameOver`. The round result rides on
`state.round_result` while the phase is `reveal`. Tapping (`state.tap`) is an **orthogonal pending
claim**, not a phase: claimable during `turnStart`, `drawn` or `effect`, forbidden during `finalSwap`.
Round 1 is opened by the player **after** the dealer (seat 0 deals, so `p2` moves first), and the deal
rotates each round.

---

## 5. Game rules (source of truth)

**Deck.** 52 cards. Optional Jokers (value 0) behind `rules.jokers`, **default off**.
Values: Ace=1, 2–10 face value, J/Q/K=10, **RED kings = −1** (both red kings).

**Deal & peek.** 4 face-down cards per player in a 2×2 square. Slots are fixed and **never
compacted** — players remember positions. Slots `0,1` = far row, `2,3` = **near/bottom row** (the app draws the four in one row, so these read as cards 3 and 4)
(`PEEK_SLOTS`). Each player may look at their bottom two **once** each, then must remember them —
`PEEK` does not end the phase; each player's `READY` does.

**A turn.** Draw the top of the draw pile **or** take the top discard, then SWAP or DISCARD.
- **Swap:** replace one of your own cards. The replaced card goes face-up onto the discard; you may
  **not** look at it first.
- **Discard:** put the drawn card on the discard; it may trigger an effect.
- *Logged assumption:* a card taken from the discard must be **swapped**, not discarded back.

**Effects** (triggered by the discarded card):

| Card | Effect |
|---|---|
| 7 / 8 | Look at one of **your own** cards |
| 9 / 10 | Look at one **opponent's** card |
| J | **Blind-swap** any two cards on the table |
| Q | Swap one of your cards with any other; you **may look at the card you receive** |
| K | Look at any two cards, then **decide whether to swap them**, and announce the decision |

**Tapping (out-of-turn race).** Whenever a table card matches the top discard, **any** player may
tap. First valid tap wins. The tapper discards at least one matching card, from **any** player's
hand; if it was another player's, they may place one of their own cards in its spot. A **wrong
tap**: the card goes back and the tapper draws a blind card into their hand. A wrong tap is a
**successful action, not an error** — refusing it would reveal whether the card matched.

**KABOO.** At the start of your turn you may call it (intended when you believe your total is ≤ 3).
The 3-point condition is **NOT enforced server-side** — rejecting it would leak hidden information;
the +20 penalty punishes bad calls. Afterwards every other player gets exactly **one** final turn.
Losing all your cards **auto-KABOOOs**. **Counter-KABOO:** after a KABOO, a player may skip their
turn and counter-KABOO to try to score lower than the original caller.

**HOUSE RULE — DECK OUT.** The moment a player draws the **last card of the draw pile**, the round
ends. That player may swap it with one of their own, or decline. **No discard, no effect, no
tapping.** Then everyone reveals and the lowest total wins. A swapped-in last card counts; if
declined it is set aside and **not scored**. A wrong-tap blind draw taking the last card goes into
the hand (penalty cards always do) and the round ends with **no swap offered**. **NO reshuffle.**

**Scoring.** The round winner scores **0**. A KABOO caller who did **not** win gets **+20**
(`kabooPenaltyMode: 'added'` — added to their hand total). All others score the sum of their card
values. Points tally across rounds. Optional **"Matt's pairs"**: a matching pair counts 0 (except
red kings); behind `rules.mattsPairs`, **default off**.

---

## 6. Open decisions

Each has a **flag** and a **default currently implemented**. Every flag here exists in
`src/engine/rules.ts`; if you add one, add it here too.

| # | Question | Flag | Default now | Notes |
|---|---|---|---|---|
| 1 | Tie-break for round winner | `rules.tieBreak` | `'seatOrder'` — ties broken by seat order walking left from the dealer | Deterministic, leaks nothing |
| 2 | How a multi-round game ends | `rules.endCondition` / `rounds` / `scoreLimit` | `'fixedRounds'`, `rounds: 5` | Alternative implemented: `'scoreThreshold'` (first to `scoreLimit`=100) |
| 3 | Exact tap-window closing time | `rules.tapClosesOnDiscardChange` | `true` — a tap is stale once the discard top changes | Taps also rejected during `finalSwap` |
| 4 | Does a tap timeout count as a wrong tap? | `rules.tapTimeoutIsWrongTap` | `true` — abandoning a claimed tap draws a penalty card | Stops "claim a tap to block others, then let it expire" |
| 5 | Counter-KABOO semantics ("the gist does not define this fully") | `rules.counterKaboo` | **minimal reading**: a counter-caller skips their turn; scored exactly like a KABOO caller (0 if they win, +20 if not); does not extend or restart the final-turn count | One counter per round per player |
| 6 | Do red kings match other kings for tapping? | `rules.redKingsMatchRank` | `true` — tapping matches on **rank**, so K matches K regardless of colour | "Matt's pairs" still excludes red kings |
| 7 | Nobody readies during `peek` | `rules.peekDeadlineMs` | `45_000`, auto-readies everyone when it passes | Fired by the room's alarm (§7, 2026-10-16) |
| 8 | Hard guard against a round that never ends (players ping-pong the discard forever without consuming the deck) | `rules.turnLimit` | `500` turns, then the round force-ends into `reveal` | Guarantees "a round always terminates" |
| 9 | How much of a failed call is the penalty? | `rules.kabooPenaltyMode` / `kabooPenaltyPoints` | `'added'` + `20` — the caller keeps their hand score and adds 20 | Alternative implemented: a flat-penalty mode |
| 10 | Hand size | `rules.handSize` | `4` | Fixed by the brief; a flag so tests can vary it |

Also implemented, **defaulted off**: `rules.jokers`, `rules.mattsPairs`.

**Need the user's answer** (a default is implemented, so nothing is blocked): #2 and #5.

Room-layer decisions with defaults, same treatment: **room code length 4**, **name capped at 12
characters**, **turn/peek deadlines** (`rules.turnDeadlineMs`, resolved lazily), and **rooms expire
after ~12h of no writes** (Redis `EXPIRE`, refreshed on every write).

---

## 7. Decision log

| Date | Decision | Why | Alternatives rejected |
|---|---|---|---|
| 2026-10-15 | Tokens extracted from the reference site via **real computed styles** (headless Chromium + `getComputedStyle` over the whole DOM), not a screenshot | The brief requires real values; guesses drift | Reading the Tailwind source only (misses which utilities land); eyeballing a screenshot |
| 2026-10-15 | Engine is **dependency-free and IO-free**; `now` and `rng` arrive via `Ctx`, and `applyAction` **deep-clones** its input and returns a new state | Pure, unit- and property-testable, movable to a plain Node server later; state is small, so cloning removes a class of aliasing bugs for no real cost | `Date.now()`/`Math.random()` inside the engine; persistent/immutable structural sharing |
| 2026-10-15 | **Hidden cards project as `{index, occupied, gen}` — never with a card id.** `gen` bumps whenever that slot's contents change | The brief asks to animate "by card ID", but a card id for a face-down card **leaks its face**: discard reveals publish `{id, rank}`, so `id` ⇒ value. `gen` still gives a stable animation key and still signals *that* the card changed (which you'd see in real life) | Sending ids for own hand (leaks); sending nothing (can't animate); server-side "your peeked cards" (brief says peeks are transient) |
| 2026-10-15 | The **drawn card is private to the drawer** from the deck; public from the discard | Matches the physical game — you look at a drawn card privately | Treating all draws as public |
| 2026-10-15 | Tap is **two-step**: `TAP` (claim) then `TAP_CHOOSE` (name the card) | A mistap costs a penalty card, so claim-then-aim is fairer on touch; the race is still settled atomically by the CAS | Single-step atomic `TAP {card}` (loses the race on a fat finger) |
| 2026-10-15 | Effects with a free look (Q's received card) **auto-reveal** to the actor instead of prompting | Saves a round trip for an action that is never disadvantageous | Separate `EFFECT_CONFIRM` step |
| 2026-10-15 | **`LogEntry` has no `card` field at all**, so the type cannot carry a face. Found by the property fuzz: a card discarded in the open was logged with its face, so once it was taken into a hand everyone could still read it | No per-slot projection can undo a face published earlier and stored forever. Removing the field makes the leak structurally impossible rather than a rule to remember | "Don't log faces" as a convention — it already was, and the bug happened anyway |
| 2026-10-15 | **`revealRound` settles loose cards** before scoring: a card the turn player still held goes into their hand; a deck-out card still under consideration is treated as declined | Fuzz bug (seed 1050406591): a wrong-tap penalty drawing the **last** card ends the round while the turn player is mid-draw, and clearing the field deleted that card from the game | Clearing the field (loses a card); refusing the penalty (contradicts the house rule) |
| 2026-10-15 | `Projection.peekableSlots` exposes exactly which slots are legal to look at, and during a King effect `EFFECT_KING_DECIDE` is advertised **instead of** `EFFECT_SKIP` once the two cards are known | The "`legalActions` is honest" test caught two bugs: `PEEK` advertised for any unready player though the handler accepts only slots 2–3, once each (the client cannot derive this from `phase`); and `EFFECT_SKIP` advertised where the resolver refuses it | Letting the UI re-derive "the bottom two" (it got it wrong); accepting a peek at any slot (breaks the rule); advertising both and letting one fail (a button that does nothing) |
| 2026-10-15 | fast-check's **generator seed is pinned** (`FC_SEED = 20261015`), and every counterexample is promoted to a named regression case | A fresh seed per run made failures unreproducible: the counterexample appeared once and vanished | Unpinned seeds (red builds you cannot replay) |
| 2026-10-15 | `Projection.discardSeq` is public, and the client-side contract has its own test file that plays whole games **from `project()` output alone** | Found by writing the client: a `TAP` must carry the `seenSeq` it was reacting to and **no other field could supply it**, so tapping was unimplementable. A safe projection that omits what a client needs to act is still broken — invisible until a human clicks a button that cannot work. Leaks nothing: anyone can count the pile | Using `version` as the sequence (it moves for unrelated reasons, so every tap would read as stale); omitting `seenSeq` (gives up the anti-stale guarantee) |
| 2026-10-15 | **A hot-seat build was written and then deleted.** One device, players passing the phone behind a "pass the phone" gate | It was a misreading of "M2 = local 2-player UI": **local** modifies the *store* (in memory instead of Redis), not the players. A room code is meaningless when everyone shares a screen, and the whole `GameState` sat in browser React state — so the hidden-information boundary the engine is built around was bypassed and nothing was actually transported. Recorded so it is not rebuilt: **two players on one machine means two browser windows**, which is the same code path as two countries | Keeping it behind a flag (two code paths, one of them untested and wrong); a single-device demo mode (not the product) |
| 2026-10-15 | The room layer owns the lobby and **the engine was not modified to add joining**. `startRoom` hands the engine a complete roster | `createGame` fixes the roster at construction and a fresh game is already in `peek`, so the engine's `lobby` phase was unreachable scaffolding, and a `JOIN` action would have meant a mutable roster — the most bug-prone thing in a card game, for a fact the room already knows | An engine `JOIN` action (roster mutation inside the state machine); re-creating the game per join (throws away the deal) |
| 2026-10-15 | **[Superseded 2026-10-16: the Durable Object serializes the room and an alarm fires deadlines.]** **Compare-and-set at the store, `rev` on the record, `version` on the state**, and the tap race is settled by whoever commits first; deadlines are **timestamps resolved lazily** on the next read *and* the next action | Serverless holds no shared memory and no durable timer: last-write-wins corrupts hands, and `setTimeout` does not survive a function returning. `rev` is separate from `version` because a join commits before any state exists | Naive read-modify-write; durable timers/cron; a client-supplied version check (the engine's own `seenSeq`/phase/turn guards are stricter and cannot leak) |
| 2026-10-15 | **Every private event carries its own `seq`** (`DeliveredEvent = EngineEvent & { seq }`), and the server sends the **whole** queue on both push and poll, because it does not know what the client has seen | Push and poll are two independent paths to the same fact, so without a per-event seq the client cannot tell a replay from a replay of a replay — a player would be shown the same peek twice, or a poll would re-announce a card the push had already delivered. With it, delivery is at-least-once and idempotent against one client-held cursor, which is what lets push stay best-effort. **Found only by writing the client** — the protocol looked complete before | Tracking delivery server-side (a second source of truth, and a dropped push then loses the peek); push-only delivery (a missed message is a lost card) |
| 2026-10-15 | **[Superseded 2026-10-16: the alarm keeps the table moving; the room socket pushes; HTTP is only the fallback while the socket is down.]** **Polling always, push on top.** `POLL_MS` 2s, `HEARTBEAT_MS` 8s when push is healthy. `realtimeToken` answers `{ token: null }` with a **200**, not a 400, when no push service is configured | A serverless function holds no timer, so **deadlines only resolve when somebody touches the room** — a purely push-driven table would sit in one phase forever the moment nobody acts. And running without Ably is a supported configuration, so "there is no push channel" is an answer, not a rejection; a 400 would blame the client for the server's choice | Push-only (deadlocks); a durable timer per deadline (does not survive); no polling when push is healthy (a dropped push would freeze the table) |
| 2026-10-15 | The **session rides in an `x-kaboo-session` header**, never a query string, and `action.playerId` from the body is **discarded and replaced from the session** | A URL ends up in server logs, browser history and `Referer` headers, and this token **is** the seat. Signed rather than random-plus-lookup so a session survives a Redis blip and yields the Ably capability directly. Overwriting the body's `playerId` is what makes "act as another player" impossible rather than merely discouraged — tested over real HTTP in both directions | Session in the URL (logged, shared, replayable); trusting the body's `playerId` (anyone can act as anyone) |
| 2026-10-15 | **[Superseded 2026-10-16: one hibernating WebSocket per device, tagged with its seat — the same "a seat only ever receives its own projection" property, without Ably.]** One **private channel per player** (`kaboo:room:{CODE}:p:{playerId}`) with a subscribe-only scoped capability, and `leave` as a **seventh** route | A shared table channel cannot carry a private peek. `tokenFor` mints `capability: { [thatOneChannel]: ['subscribe'] }`, so a player cannot listen to another seat even knowing its name. `leave` was added because `room.ts` already implemented and tested `leaveRoom` and the UI needs it — a route the plan had missed | One room-wide channel (cannot deliver a peek privately); filtering a shared channel client-side (the card would already be on the wire) |
| 2026-10-15 | **[React-era: a client-only Vue SPA has no server render to disagree with, so the seat is a plain reactive ref kept in step with the `storage` event.]** The **seat is read with `useSyncExternalStore`**, with `getServerSnapshot` returning `null` and a second constant-snapshot store for "has the browser taken over yet" | `localStorage` is external state: copying it into `useState` via an effect is a second source of truth and renders once with the wrong answer — a seated player would see the join form and then be yanked off it. The server cannot read `localStorage`, so it renders the neutral gate and the client takes over. Also the only shape React 19's `set-state-in-effect` and `refs` lint rules accept | `useState(() => loadSession())` (hydration mismatch); `useEffect(() => setSession(…))` (lint error, cascading render); a `/api/me` round trip (a network hop to answer a question the device can answer) |
| 2026-10-15 | **[Still the shape in Vue: `RoomScreen` mounts `useGame` once, above lobby and table, keyed on the token.]** `useGame` takes a **non-null** session and is mounted once, in `page.tsx`, above both the lobby and the table; the ref holding `onLost` is written **in an effect**, not during render | The lobby and the table are two views of one live connection, so dealing must not tear down the stream and rebuild the Ably connection — and the peek cursor must survive it. A null branch would have needed `setState` inside an effect to reset state, which is both a lint error and a cascading render | One `useGame` per screen (reconnect on deal, cursor reset); resetting state in an effect (lint, extra render); `useEffectEvent` (not stable in this Next version) |
| 2026-10-15 | **The deck is real artwork, vendored as PNG — a deliberate, measured deviation from the brief's "render cards as SVG or CSS"** | The brief's rule exists to avoid copyrighted art, so the fix was a licence, not a drawing: Byron Knoll's vector deck, released into the **public domain** (readme read before anything was vendored), via `notpeter/Vector-Playing-Cards` at a pinned commit. Rasterised at 264px wide — 3× the largest a card is ever drawn — and quantised to 128 colours: **668 KB for all 54**, at **exactly the size they are displayed**, so there is no scaling work for the browser to do. Hand-drawn CSS pips would have been a worse game to play *and* a new source of contrast bugs | Shipping the SVGs (8 MB raw, ~3 MB after SVGO for the 12 court cards alone — and `floatPrecision: 0`, the setting that would have made them small, collapsed a queen into stray diagonals, verified by rendering both and looking); WebP (no gain on flat art this size, and one more thing to get wrong); `next/image` (the files are already pre-sized, so the optimiser buys a round trip and nothing else) |
| 2026-10-15 | **Every action lives in one `.kb-dock` pinned to the bottom centre**, full-bleed, `max-width: 48rem` inside | The owner's report: "the buttons are hard to see and reach". As a bar hanging off the bottom of the left-hand desktop column, the shortest mouse travel to *Swap* ran diagonally across the whole table; on a phone the bottom is where the thumb already is. The peek tray and the refusal message moved in with it, so nothing floats over the pile the next player is about to tap. Disabled buttons stopped being `opacity: 0.45` (≈2:1 — a button you cannot read is not a button) and became flat, muted and legible at 6.5:1 | A floating action button (hides the one control that matters); a top bar (furthest point from a thumb); keeping the bar in the column and only making it bigger |
| 2026-10-15 | **The table is exactly `100dvh` and does not scroll; the board between the fixed rows is the only scroller.** The dock owns the height with `flex: 0 0 auto` instead of `position: sticky` | Sticky only ever pins a bar that is *already* at the bottom of its scroll: at the top of a too-tall page the bar sits mid-document and the last row of the player's hand hides behind it — the buttons were reachable and the cards they act on were not. Below the desktop breakpoint each column of the board scrolls on its own, because scrolling the board as a whole dragged the scoreboard's heading off the top of the screen the moment a hand needed to come into view | Trimming the layout until the worst case fits (shrinking the cards every player looks at all game to serve a hand that took a penalty card); `position: fixed` (content passes underneath); letting the page scroll (a dock that scrolls away is not a dock) |
| 2026-10-15 | When it becomes your move, the board scrolls your own cards into view — `scrollIntoView({ block: 'nearest' })`, instantly | `'nearest'` scrolls only when the element is not already fully visible, so it is a **no-op on every screen that already fits** — which is most of them. Instant rather than smooth because a control you cannot see is worse than a jump you did not ask for, and `prefers-reduced-motion` is not a question worth asking when the alternative is an invisible card | Shrinking cards to fit the worst case; a manual "scroll down" affordance (the player does not yet know there is anything to scroll to); smooth scrolling (a 40px correction does not want 300ms of animation) |
| 2026-10-15 | **`--card-h: calc(var(--card-w) * 1.4545)` was replaced by `--card-ratio` + `aspect-ratio` on `.kb-card`** — a real, pre-existing bug, not a refactor | A CSS custom property is substituted **where it is declared**: a `calc()` on `:root` resolves against the root's `--card-w` and is then inherited *frozen*, so it does not follow a descendant's override. Every place that shrank `--card-w` (compact hands, the peek tray) got a card the right width and the wrong height — measured at **43×85px, a ratio of 1.98 instead of 1.45**. Invisible against plain black rectangles; glaring the moment real art arrived | A second `.kb-card` rule per context (five places to forget); `height: calc(100% * …)` (needs a definite height, which is the problem); JS measurement (breaks SSR and resize) |
| 2026-10-15 | `.kb-label { margin: 0 }` — the label class dresses `h3`, `p` and `label` | An `h3` at 12px still carries a 1em margin, so every heading cost 24px of nothing on top of the parent's flex gap: **48px across the opponent row and the player's own row**, which on an 844px phone was most of the second row of the hand | Setting the margins at each call site (the same bug, four places to forget) |
| 2026-10-15 | **A peek notice is kept only for the round it arrived in** (`Peek.round`, pruned on insert) | The tray only ever grew: `setPeeks((prev) => [...prev, …])` with no bound meant a player who never tapped *Remember it* accumulated every peek and every draw of the whole game — measured at three cards after one turn, and a dock that eats the screen by round two. Nothing is lost by dropping them at a round boundary: at the reveal every card is face-up, so the reminder has stopped being information | A hard cap with a "+N earlier" note (silently withholds cards the player was shown); clearing on turn change (wipes the opening peeks, the only cards anyone legitimately saw); an effect on the round number (a second place the tray changes shape, and a lint error) |
| 2026-10-15 | **[Obsolete 2026-10-16: there is no Redis.]** **Redis credentials are found by suffix and paired by prefix, in `src/store/env.ts`** | Vercel's Upstash integration names the variables `<PREFIX>_KV_REST_API_URL`/`_TOKEN` (the prefix is picked at install: `STORAGEKOBUL` here), not `UPSTASH_REDIS_REST_*`. Recognising only the latter would make a correct deploy silently fall back to the in-process `Map`, which looks fine for one player and splits rooms across serverless instances. Matching on the suffix means nothing has to be renamed in the Vercel dashboard. The URL and token must share a prefix (never mix two integrations), only `KV_REST_API_TOKEN` matches (the `…_READ_ONLY_TOKEN` ends differently, and the store writes), and keys are sorted so the choice is deterministic. Explicit `UPSTASH_REDIS_REST_*` still wins | Renaming the variables by hand in the dashboard (a manual step to forget on every re-install); hard-coding `STORAGEKOBUL_` (breaks on re-install); taking the first `*_TOKEN` found (could be the read-only one) |
| 2026-10-15 | **[Superseded 2026-10-16: `ROOM_SECRET` is required and there is no fallback.]** **The session key is `ROOM_SECRET`, else derived from the Redis token (`src/server/sessionKey.ts`); the old "throw in production if Redis is set but ROOM_SECRET is not" guard was removed** | Symptom: players were kicked from the room seconds into a game. Cause: with no shared key each serverless instance minted its own random one, so a session signed by one instance failed verification on the next, which the client treats as a lost seat. The old guard could not catch it, because it only looked for the `UPSTASH_REDIS_REST_*` names and Vercel's integration uses `<PREFIX>_KV_REST_API_*`. Every instance reads the same Redis token, so every instance derives the same key (HMAC with a fixed label, so the key does not reveal the token) and nothing needs setting in the Vercel dashboard. Trade-off: one secret serves two purposes; a leaked Redis token already exposes every room, so nothing new is exposed | Requiring `ROOM_SECRET` in the dashboard (the owner asked not to touch Vercel); keeping the throw and widening its env check (turns the misconfiguration into a loud 500 but still needs the manual step); a random key per instance (the bug) |
| 2026-10-15 | **The hand is one row that scrolls sideways, not a 2×2 square** | Owner's report: the table felt static and confusing, and the square made it hard to see where a drawn card went. A row reads left to right, gives the player's own cards more width (drawn larger than an opponent's: `19vw`, capped at 88px, which fits four cards on a 320px phone) and saves about 48px of height on a phone. It still never compacts or reorders slots, so position is still the memory game; a penalty card extends the row. The first four fit without scrolling | Keeping the grid and animating it (the owner prefers the row); wrapping a long hand onto a second row (a shape nobody has learned) |
| 2026-10-15 | **Card movement is shown by `src/lib/flights.ts` (pure: compares two snapshots) + `CardFlights.tsx` (draws ghost cards)** | The state is already correct when a snapshot arrives; the ghost only shows where a card came from. The planner reads only what the viewer may already see (counts, the public discard top, `occupied`, `gen`), so it adds no information, and it recognises a pattern or draws **nothing**: a poll gap can swallow several moves, and a wrong animation tells the player a card went somewhere it did not. A layout effect hides the destination before paint (otherwise the card flashes into place and only then flies). Anchors are static (`data-flight` on the three piles and every slot), so both rectangles are read from the new DOM. A ring (`.kb-card--landed`) stays on the arrived card for about 1.2s and is kept under `prefers-reduced-motion`, where the flight itself is skipped | Animating the real elements (a slot's card re-mounts whenever `gen` changes, so there is no stable element to move); an animation library (a dependency for one tween); animating only the destination (cannot show where a card came from) |
| 2026-10-15 | **No turn action until the card preview is closed** (`blocked` in `GameScreen`, `closeFirst` in `ActionBar`) | Owner's request. "Remember it" is the only record of what a player saw, so moving on before closing it is moving on before the card was taken in. While a notice is open the turn buttons are disabled with a stated reason and no table card is pickable. **Exempt:** the opening peek (notices pile up by design, and Ready has its own gate), taps (a race against a deadline), and a King's swap decision (closing the notice would remove the two cards the decision is about). The turn deadline still runs, so an unclosed notice cannot strand a table | Gating the tap (loses the race to a dialog); gating the King decision (decide blind); hiding the buttons instead of disabling them (no reason shown) |
| 2026-10-15 | **Ready is disabled, with the reason printed, while a peek notice for the opening pair is still open** — a *client-side* gate, deliberately, and separate from `blocked` above (which exempts the peek) | Owner's request: "make sure people can't click ready before they close the 2 cards they're seeing at the start." Looking at your own two cards *is* the peek phase, so Ready is the step that throws the look away. A client-side gate is the only honest one available: notices are transient reveals the server keeps no record of (§9.2), so there is no dismissal for the server to enforce; and the 45s peek deadline still readies everyone, so it cannot strand a table. **Still open:** a player who never peeks at all is ungated — closing the notices presupposes there are some. Forcing the peek would make it mandatory, which is a rule change and is raised in §6, not made here | Server-side enforcement (needs stored dismissal state, which §9.2 forbids); a toast that only warns (the owner asked for "can't", not "is discouraged"); hiding the button (a disabled control that says why beats a missing one) |
| 2026-10-15 | **The live-card cue is a ring, not a lift or a pulse of the whole card** (`.kb-card--live`, a 3px→7px `--c-highlight` ring) | The cue means "this is what the current decision is about", and it has to survive being one of four cards the player is memorising. A lift would break the row's baseline and make the hand look ragged for as long as the decision lasts; a scale would change the card's size, which is the one thing that carries position; only a ring leaves the frame exactly where it was. `--c-highlight` because KABOO already owns that colour as "the loudest thing on screen". Set on `Card` only, and chosen in `GameScreen` — see the two traps in §9 | An outline (`outline` is the focus ring's job — two rings on a focused card, and the focus one must win); a border (changes the layout box); a fully lit background (reads as face-up) |
| 2026-10-15 | **`.kb-card--selected` lifts with `translate`, and hover/press add to it with `translate` too — never `transform`** | `translate` and `transform` are *separate properties that compose*, so `--selected`'s `translate: 0 -2px` and the hover rule's `translate: 0 -4px` (and `:active`'s `translate: 0 0`) resolve against each other in the ordinary cascade instead of one silently erasing the other. With a single `transform` the last rule to win would decide, and the press state would lose its selected lift. `transform` is left free for a future rotate/scale. Same reason the reduced-motion block resets `translate` **and** `transform` | One `transform` property (states overwrite rather than compose); `top`/`margin` (animates layout every frame) |
| 2026-10-15 | **A claimed tap's intent is derived from `view.tap`, not from button state** — `GameScreen`'s `activeIntent` is `tapPlace`/`tapChoose` while `view.tap.playerId === youId`, and wins over the `none` a button would have set | **A real, pre-existing bug that made the tap unplayable.** A tap is the one decision a player does not start with a button: the claim puts them in it and `legalActions` then reads `TAP_CHOOSE`. Nothing anywhere set `intent` to `'tapChoose'`, so after claiming, every card on the table stayed inert while the bar said "pick the card that matches the top of the pile" — measured in the browser as `ownTappable: 0, allTappable: 0` against a server advertising `['TAP_CHOOSE']`. The tap could then only end on the 8s deadline, which counts as a *wrong tap*: a penalty card, for a button that could not work. After: `ownTappable: 4, allTappable: 8` | A `onIntent('tapChoose')` set in an effect when the tap arrives (an effect that exists only to mirror state the view already carries, and one render late); putting the tap's two steps in `ActionBar`'s `Intent` state (the tap can arrive from another player's claim on your behalf) |
| 2026-10-16 | **Rewrite: Rust engine + room on Cloudflare Workers with one Durable Object per room, Vue 3 + TypeScript in front, all on the free plan.** The Next tree is tagged `legacy-next` and was replaced in place | Owner's request (Vue + Rust/axum, free hosting). A Durable Object is the shape this game always wanted: one single-threaded actor per room with its own storage, its own WebSockets and its own **alarm** — which removes the three workarounds serverless forced (compare-and-set retries, deadlines resolved lazily by whoever touched the room next, and clients polling to be that "whoever"). axum is the HTTP router inside the Worker; tokio/hyper do not exist on wasm | Render free tier with an in-memory Axum server (sleeps after 15 idle minutes, loses rooms on sleep); an Oracle free VM (ops burden); keeping Vercel + Upstash + Ably (the most moving parts, and Vercel does not host Axum) |
| 2026-10-16 | **No compare-and-set: the Durable Object serializes the room.** Each use case reads the slot, decides, and writes the whole record or nothing; the tap race is settled by arrival order and the loser gets `E_TOO_LATE` from the engine, for the right reason | A Durable Object runs one event at a time and its input gate holds other events while storage is awaited, so read-decide-write cannot interleave. `rev`, `StateStore`, `E_BUSY`-on-contention and the retry loop all went with it | Keeping a CAS on top (guards a race that cannot happen) |
| 2026-10-16 | **Deadlines fire from the room's alarm; client polling is gone.** After every write the room sets its alarm to `next_alarm(record)` = the earliest engine deadline (`next_deadline`), or the room's 12h expiry. `alarm()` ticks, stores and pushes to everyone; at expiry it deletes the room and closes its sockets. Reads still tick lazily too, harmlessly. Verified under `wrangler dev`: a dealt table went peek → turnStart at exactly 45s with nobody connected doing anything | Serverless had no timer, so the old design needed every client polling to keep the table alive (§7, "Polling always"). A test (`next_deadline_is_never_late`) holds the alarm time to "the first instant `tick` would act", so the alarm can never sleep past a deadline | Polling kept as a heartbeat (pointless with an alarm); an alarm per deadline type (one alarm per object is all the platform offers) |
| 2026-10-16 | **One WebSocket per device to its room; auth is the first message, never the URL; moves stay on HTTP.** The socket is accepted as *hibernatable* with an attachment `{code, playerId}` that survives eviction; pushes go to every socket whose attachment names the seat. The client re-sends `auth` with its cursor on reconnect, tab wake and `online`, and the answer is a full `SyncResponse` — so connecting *is* the initial load and reconnecting *is* the resync. While the socket is down the client reads over HTTP every 4s | A browser cannot set headers on a WebSocket, and a token in a URL ends up in logs and history (the reason the session rides in a header, §7). Moves on HTTP keep one path per move, and that path is the one that answers with `refusal`. Hibernation keeps idle sockets from billing duration on the free plan | Ably (a second service, and a key to manage); `Sec-WebSocket-Protocol` for the token (still logged by some proxies); moves over the socket (two paths, and no natural place for `refusal`) |
| 2026-10-16 | **A WebSocket upgrade bypasses axum** and is re-issued to the Durable Object with a copied header set plus `x-kaboo-code` | An incoming request's headers are immutable in workers-rs (`Cannot get a mutable reference to an immutable headers object`), and the upgrade response must reach the runtime untouched; axum adds nothing to a pass-through | Routing the upgrade through axum (the spike showed a 500) |
| 2026-10-16 | **`ROOM_SECRET` is required; there is no fallback key** | The Redis-token derivation (§7, 2026-10-15) existed because Vercel's instances shared nothing else; Workers have secrets. A random per-isolate fallback would reproduce the "kicked out seconds into a game" bug the moment an isolate restarted | Deriving a key from something else; a dev-only random key (would make local behaviour diverge from production) |
| 2026-10-16 | **Lobby seat ids are never reused** (`RoomRecord.next_seat`). A real, pre-existing security bug | The TS room minted `p{roster.length+1}`: when `p2` left the lobby and someone joined, the newcomer was `p2` again — and the departed player's token, still validly signed and still naming `p2`, now acted as the newcomer and could read their cards. The TS test asserted the reuse; the Rust test asserts the opposite and that the old token is refused | Revoking tokens server-side (needs a revocation list); keeping the reuse |
| 2026-10-16 | **Private events produced by a tick are queued** (they were dropped) | The TS `read` ticked deadlines but never queued the resulting private events — e.g. the drawn card of a timed-out auto-play. Now `read` and `alarm` queue them like `act` does | — |
| 2026-10-16 | **`EngineEvent` is one flat struct with optional fields**, not an enum per event | Keeps the wire shape the browser already reads (`{type, to, card, playerId, slot, …}`) and generates a usable TS type. Public events never leave the server (they are not queued), so the two TS fields whose meaning collided (`from`, `result`) became `source` / `round_result` / `game_result` with no client impact | A tagged enum per event (a TS union of 25 shapes for 4 that ever reach a client); `serde_json::Value` payloads (untyped on both sides) |
| 2026-10-16 | **The edge fills `action.playerId` with a placeholder before parsing, then the room replaces it from the session** | The client never says who it is in a body (it is discarded anyway), and the typed `Action` requires the field. An action that still does not parse is `400 E_BAD_REQUEST` — no engine state could have produced it | `#[serde(default)]` on 22 fields; a separate wire type for incoming actions |
| 2026-10-16 | **Wire types are generated from Rust by ts-rs** into `web/src/generated/`, and `verify` fails if they are stale. `i64` timestamps are annotated `#[ts(type = "number")]` | One definition, no drift: adding a field to `RoomView` is a Rust change and a regenerated file, never a hand-edit in two languages | Hand-written TS mirrors (they drifted in exactly the way §7's `discardSeq` row describes); `bigint` timestamps (JSON numbers, not bigints, arrive) |
| 2026-10-16 | **The tests moved with the code, and the old seeds still mean the same deal.** `mulberry32` and Fisher–Yates are bit-for-bit ports (a test pins values computed with the TS generator); property tests use proptest with `RngSeed::Fixed(20261015)`; every named regression became a named Rust test | Seed 1050406591 still ends its fuzz round at step 118, exactly as recorded in TS. Mutation checks (5 engine paths, 1 room path) all turn the suite red | Re-deriving expectations from the new code (would test the port against itself) |
| 2026-10-16 | **The Vue UI fixes two legacy UI lies:** the Queen's target excludes your own cards (the engine refuses them), and the deck-out buttons are shown only to the drawer | Both were buttons that could only produce `That did not work.` — the §9 rule "if the UI can do something, the engine must accept it" applied to the client too | — |
| 2026-10-17 | **Found by driving the UI, not by a test: in Vue, read a computed before mutating what it derives from.** `GameScreen.onSlot` reset `chosen` and then switched on `intent` — which is computed from `chosen`, so every pick was a pick with no intent and nothing was sent | The React version captured the intent at render time; Vue's computed is live. It looked like "Swap does nothing" in the browser and was invisible to every unit test. Now captured into a local first, with a comment | — |

---

## 8. Design tokens and style notes

**Source:** `https://hackathon.ackteams.com/` and `/student`, read as **computed styles** in headless
Chromium. The reference is **neo-brutalist**. We take colour, type, border weight, radii, shadows,
spacing and motion — **not** its layout or content.

**The signature (do not dilute it):** zero border-radius — literally `0`, not "small"; hard black
borders (`2px`, `4px` for section dividers); hard offset shadows with **zero blur**, pure black
(`2px/4px/8px`, `0 0` spread); uppercase `font-weight: 900` with tight tracking on headings, mono
bold 12px for codes and labels. **Press = move into the shadow:** on `:active`, `box-shadow: none`
**and** translate by the offset the shadow had (`4px 4px` shadow ⇒ `translate(4px, 4px)`) — that is
the motion signature.

**Tokens live in `src/styles/tokens.css`** — written, and the only place raw values are defined.

| Group | Tokens | Values | Provenance |
|---|---|---|---|
| Surfaces | `--c-bg`, `--c-surface`, `--c-surface-2` | `#ffffff`, `#f9fafb`, `#f3f4f6` | `body` bg, gray-50, gray-100 |
| Ink | `--c-ink`, `--c-ink-soft`, `--c-ink-muted` | `#000000`, `#364153`, `#4a5565` | text + all borders, gray-700, gray-600 |
| Primary | `--c-primary`, `-press`, `-soft`, `-line` | `#0055ff`, `#193cb8`, `#dbeafe`, `#90c5ff` | `--color-primary`, blue-800/100/300 |
| Highlight | `--c-highlight` | `#fef08a` | `--color-highlight` (KABOO) |
| Status | `--c-danger`/`-soft`, `--c-success`/`-soft` | `#fb2c36`/`#ffe2e2`, `#008138`/`#dcfce7` | red-500/100, green-700/100 |
| Borders & radius | `--bw-1/-2/-4`, `--radius` | `1px`/`2px`/`4px`, **`0`** | computed widths; **every** element computed `0` |
| Shadows | `--shadow-sm/-/-lg` | `2px 2px 0 0 #000`, `4px 4px 0 0 #000`, `8px 8px 0 0 #000` | computed `box-shadow`, blur `0` |
| Type | `--text-xs…--text-4xl`; `--fw-normal/-medium/-bold/-black`; `--tracking-tight/-tighter/-wide`; `--leading-*` | `12/14/16/18/20/24/36/48px`; `400/500/700/900`; `-0.025/-0.05/0.1em` | computed sizes, weights, `letter-spacing` |
| Fonts | `--font-sans`, `--font-mono` | Geist Sans, Geist Mono (+ system fallbacks) | `@fontsource/geist-sans`, `@fontsource/geist-mono` (self-hosted, bundled by Vite) |
| Spacing | `--space-1…--space-12` | `4px` base | `--spacing` |
| Motion | `--dur`, `--ease`, `--dur-flip` | `150ms`, `cubic-bezier(.4,0,.2,1)`, `240ms` | computed transitions; the flip is our one deliberate exception |
| Layout | `--gutter`, `--tap-target`, `--card-w`, `--card-ratio` | `16px`, `44px`, `clamp(52px, 15vw, 88px)`, `1.4545` | ours, not the reference's |
| Focus | `--focus-width` | `4px` | ours — the one control that must never be missed |

**Component classes** (defined in `tokens.css`; components apply them, never raw values):
`kb-surface`, `kb-header`, `kb-btn` (`--primary` / `--danger` / `--kaboo`), `kb-chip`
(`--dark` / `--soft`), `kb-title`, `kb-label`, `kb-card` (`--down` / `--selected` / `--live` /
`--landed` / `--dimmed` / `--empty`), `kb-table`. `game.css` is layout only and defines no raw values.

Three rules the class bodies encode, so read them there before overriding: **KABOO** is highlight
yellow with black text and an `8px` shadow (never white on yellow — it is 1.3:1); contrast is
`#000` on `#fff`, `#193cb8` on `#dbeafe` (7.4:1), or black on `--c-highlight` (17:1);
`prefers-reduced-motion` collapses `--dur` to `0ms` rather than removing transitions, because card
movement is the game's information channel.

**Card interaction states**, all in the `game.css` card block, and all four are needed at once:
*rest* (`--shadow-sm`) → *hover* (lift `4px`, `--shadow`) → *press* (into the shadow: `box-shadow:
none`, `translate: 0 0`) → *live* (the yellow ring, §7). Hover is gated on
`@media (hover: hover) and (pointer: fine)`; press and live are not, because a touch screen has both.
Under `prefers-reduced-motion` the **state survives and the movement goes**: `--dur` is `0ms`, the
hover lift and the press translation are set to `none`, and `--live` keeps its static `3px` ring with
`animation: none` — a player who asked for no motion still has to be told which card is live.

**Card art.** `web/public/cards/{rank}{suit}.png` (`AS`…`KD`, plus `Joker1`/`Joker2`), **public domain**
— Byron Knoll's vector deck, redistributed by `notpeter/Vector-Playing-Cards`; the render script
pins the upstream commit. `web/public/cards/README.md` records this where the files are. `cardFile()` (`web/src/lib/cards.ts`)
maps a `PublicCard` to its file, and the two jokers are told apart by **id** (52/53) because their
`rank` and `suit` are identical. The **back is CSS**, not art: nothing to load for the ten of them on
the table, and a shut card must not read as "a card" from across the table. The face image overshoots
its frame by 4% (`.kb-card__face { inset: -2% }`) so the deck's own rounded border falls outside the
clip and a card ends up with exactly one edge — the `2px` rule on `.kb-card`. At `108%` the corner
index clips; at `104%` it does not.

---

## 9. Conventions and gotchas

**Hidden information — the rules that matter most:**
1. **The server is authoritative.** Full state never leaves the Durable Object; clients receive
   `project(state, viewer)` and nothing else. There is no `GameState` type in the browser at all.
2. **Peeks are transient reveal events, never stored state.** Events carry `to`, the room queues them
   only for that player, and the client holds them in the peek tray and forgets them when dismissed.
3. **Never compute or reveal a player's own total during play** — not in the projection, not in the
   UI, not in an event. Totals appear only at `reveal`. (A projection test asserts the word "total"
   never appears in a live projection.)
4. **Errors are generic.** Neither `EngineError` nor `RoomError` explains *why*; the client renders
   one flat "That did not work." A refusal is distinguishable from "nothing happened" **only** by the
   `refusal` field on an otherwise successful response.
5. **Never send a card id for a card the viewer may not see** (`gen` instead, §7), and **never put a
   face in a log entry** — `LogEntry` has no card field. Faces belong in *events* or on the public discard.
6. Rule checks that would *leak* are deliberately **not** enforced (the KABOO 3-point condition).
7. `POST /join` is a **room-existence oracle**, and `E_NOT_FOUND`/`E_BAD_SESSION` are
   distinguishable — they must be, since joining by code is the product. Neither reveals a card.
8. **The session decides who you are.** The room replaces the body's `playerId` with the session's,
   and checks the session's room code *before* looking the room up (a token for room A against any
   other code is `E_BAD_SESSION`, room or not).

**Things that look wrong but are intentional:**
- A hand can be **longer than 4** and can contain **empty slots**; `0` cards is legal and auto-KABOOs.
  Slots are never compacted — render gaps as empty, not as a shifted row.
- `state.set_aside` holds the **declined last card** so every card id is accounted for exactly once.
- `legal_actions` must be **honest**: everything it advertises is accepted by `apply_action` in that
  state. `projection.rs` holds it to this. Add a handler branch → update the advertisement in the same commit.
- `version_changed: false` means "do not write" — the input state is returned unchanged.
- A player's stored session **is** their seat. The room never frees a seat once the deal has
  happened; the engine autoplays a disconnected turn.
- The engine's `gen` field is `generation` in Rust (`gen` is reserved in edition 2024) and `gen` on the wire.

**The client contract — the rule that keeps the UI alive:**
- The UI reads **`Projection` only**. A card the viewer may not see has a `null` face and no
  component may overrule that. Faces arrive as private events and live only in the peek tray.
- The UI never re-derives a rule: buttons come from `legalActions`, tappable slots from
  `peekableSlots`/`canTap`. If a rule seems missing from the projection, **add it there**.
- **If the UI can do something, a case belongs in `crates/engine/tests/client_contract.rs`** — those
  games are played from projection output alone via `testkit::choose_move`, which may not read
  `GameState`; the room's `transport_game.rs` plays whole games through the HTTP edge with the same function.
- **Every entry in `legalActions` that needs a slot chosen must have an `Intent` reachable from a
  button or from the view itself** (`web/src/lib/intent.ts`). A claimed tap has no button: its intent
  is derived from `view.tap` (§7, the tap bug; `tests/intent.test.ts` pins it).
- Every button is a real `<button>`; every target is ≥ 44px. A mistap costs a penalty card.

**Transport:** `crates/protocol` is the only thing both sides know, and the browser's copy is
generated. Every response and every push is a `SyncResponse` built by the same `sync_for`, so a fact
cannot arrive two different ways. Delivery is at-least-once: a push carries the seat's whole private
queue and the client dedupes on each event's `seq` against one cursor (`useGame`). A refused move that
moved nothing pushes nothing; a moved clock (alarm or lazy read) pushes to everyone.

**Traps already hit:**
- Never shuffle with a comparator or a non-injected RNG. `shuffle(items, rng)` (Fisher–Yates); the
  Worker backs `rng` with `crypto.getRandomValues`, tests with `mulberry32`.
- A property failure is only useful if it replays: keep the proptest seed fixed, put the offending
  action in the message, promote each counterexample to a named regression test.
- **Mutation-test anything load-bearing before trusting a green suite.** Break the routing, the
  projection or `legal_actions` and confirm the suite goes red. (When doing it with a script, restore
  from a backup in a `finally` — an interrupted run once left a mutation in `room/src/lib.rs`, caught
  only because the next test run went red.)
- **Vue: read a computed before you mutate what it derives from** (§7, 2026-10-17).
- **In browser tests, read text with `textContent`, not `innerText`:** `innerText` applies CSS, and
  every label here is `text-transform: uppercase` ("YOUR TURN", "SLOT 3").
- Tailwind is **not** used. Plain CSS + tokens; utility classes would fight the token layer.
- `touch-action: manipulation` and `overscroll-behavior: none` go on `.kb-table`; layout uses `dvh`
  (never `vh`) with `env(safe-area-inset-*)` padding; `index.html` sets `viewport-fit=cover` so the
  insets are real.
- **A CSS custom property is substituted where it is DECLARED, not where it is used.** Derive sizes
  with `aspect-ratio` (or a `calc()` inside the rule that uses it) — §7, `--card-ratio`.
- **Layout is measured, not assumed.** `getBoundingClientRect()` in a real browser found the
  stretched card and an 844px screen overflowing by 167px; at 390×844 the Vue build has no page scroll.
- **The hand is ONE row, side by side, scrolled sideways (owner's call, 2026-10-15).** Do not bring
  the 2×2 grid back without asking the owner.
- `:hover` is gated behind `@media (hover: hover) and (pointer: fine)` — on touch, an ungated hover
  sticks to whatever was tapped last.
- **The peek tray's caption sets the tray's width, which sets its height.** Keep notice labels short
  (`slot 3`), and measure the tray, not the card, when changing one.
- **The board shrinks when the peek tray opens.** The "scroll your own cards into view" watcher depends
  on the tray being open as well as on whose turn it is.
- **wrangler `watch_dir` must not include build output** (`crates/worker/build`), or every build
  triggers the next one forever. It lists the four `src/` directories.

---

## 10. Next steps

1. **Deploy (R6). Blocked on the owner, not on code.** Run `! npx wrangler login` in this session,
   then `npx wrangler secret put ROOM_SECRET` (a value from `openssl rand -hex 32`) and `bun run deploy`.
   Then `KABOO_BASE=https://kaboo.<account>.workers.dev bun run smoke`, and the UI drive with its
   `BASE` pointed at the same URL.
2. **Real hardware.** Only ever measured in headless Chromium. Check on a real phone: the dock clears
   the home indicator, the hand row stays one row and scrolls sideways, *Remember it* is reachable
   one-handed, hover does nothing on touch, and the socket survives the phone sleeping (it should
   reconnect and resync on wake).
3. **M5 — 3 to 8 seats.** Opponents render in a scrolling row; the brief wants 3–8 seated around the
   table on desktop while keeping the mobile column. The phone layout is full at three opponents.
4. **A topbar that fits.** It wraps to two rows below about 380px.
5. **Watch the free-plan quota** once real games run: 100k requests/day, and WebSocket messages and
   alarms count toward it.

**Open questions for the user:** §6 #2 (how a multi-round game ends) and #5 (counter-KABOO); and
**should the opening peek be mandatory?** Ready is gated on *closing* the two notices, so a player who
never looks is ungated. All have working defaults, so nothing is blocked.
