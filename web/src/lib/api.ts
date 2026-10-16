import type { Action } from '@/generated/Action';
import type { ActionResult } from '@/generated/ActionResult';
import type { RoomError } from '@/generated/RoomError';
import type { Seated } from '@/generated/Seated';
import type { SyncResponse } from '@/generated/SyncResponse';
import { SESSION_HEADER, isRoomError, offline } from './protocol';
import type { StoredSession } from './session';

/**
 * THE API CLIENT — the only place this app calls `fetch`.
 *
 * Every call resolves to a value or a `RoomError`; none of them throw. A lost
 * tap race is an expected outcome and a dropped connection must not look
 * different from a busy table, so no caller needs a try/catch to avoid a blank
 * screen.
 */

const BASE = '/api/rooms';

async function call(path: string, init: { method: string; body?: unknown; token?: string }): Promise<unknown> {
  const headers: Record<string, string> = { 'content-type': 'application/json' };
  if (init.token) headers[SESSION_HEADER] = init.token;

  let response: Response;
  try {
    response = await fetch(path, {
      method: init.method,
      headers,
      body: init.body === undefined ? undefined : JSON.stringify(init.body),
      // Never from a cache: a stale projection is a player looking at a table
      // that has already moved on.
      cache: 'no-store',
    });
  } catch {
    return offline();
  }

  let payload: unknown;
  try {
    payload = await response.json();
  } catch {
    return offline();
  }
  if (isRoomError(payload)) return payload;
  return response.ok ? payload : offline();
}

function isSeated(value: unknown): value is Seated {
  if (typeof value !== 'object' || value === null) return false;
  const v = value as Record<string, unknown>;
  return typeof v.code === 'string' && typeof v.playerId === 'string' && typeof v.token === 'string';
}

/** A response that is not a sync is a failure, not something to coerce. */
function asSync<T extends SyncResponse>(value: unknown): T | RoomError {
  if (isRoomError(value)) return value;
  const sync = value as Partial<SyncResponse> | null;
  if (typeof sync !== 'object' || sync === null || typeof sync.room !== 'object' || sync.room === null) {
    return offline();
  }
  return { ...(sync as T), events: sync.events ?? [], eventSeq: sync.eventSeq ?? 0 };
}

const room = (s: StoredSession) => `${BASE}/${encodeURIComponent(s.code)}`;

async function seated(result: unknown, name: string): Promise<StoredSession | RoomError> {
  if (isRoomError(result)) return result;
  if (!isSeated(result)) return offline();
  return { code: result.code, playerId: result.playerId, name, token: result.token };
}

/** Mint a room and take seat 1. */
export async function createRoom(name: string): Promise<StoredSession | RoomError> {
  return seated(await call(BASE, { method: 'POST', body: { name } }), name);
}

/** Take a seat in somebody else's room, given only the four characters. */
export async function joinRoom(code: string, name: string): Promise<StoredSession | RoomError> {
  return seated(await call(`${BASE}/${encodeURIComponent(code)}/join`, { method: 'POST', body: { name } }), name);
}

/** The host deals. Everybody else is refused, and the refusal says no more than that. */
export async function startRoom(session: StoredSession): Promise<SyncResponse | RoomError> {
  return asSync(await call(`${room(session)}/start`, { method: 'POST', token: session.token }));
}

/**
 * One move. `action.playerId` is ignored by the server — it is replaced from
 * the session, because a request body is not allowed to say who you are.
 */
export async function sendAction(session: StoredSession, action: Action): Promise<ActionResult | RoomError> {
  return asSync<ActionResult>(
    await call(`${room(session)}/actions`, { method: 'POST', token: session.token, body: { action } }),
  );
}

/** This player's truth from `since` onward: the fallback when the socket cannot connect. */
export async function readState(session: StoredSession, since: number): Promise<SyncResponse | RoomError> {
  return asSync(await call(`${room(session)}/state?since=${since}`, { method: 'GET', token: session.token }));
}

/** Give up the seat. Frees it before the deal; only marks it gone afterwards. */
export async function leaveRoom(session: StoredSession): Promise<void> {
  await call(`${room(session)}/leave`, { method: 'POST', token: session.token });
}
