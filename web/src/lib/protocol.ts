/**
 * The few runtime pieces of the wire contract. The TYPES are generated from
 * the Rust crates into `src/generated/` (ts-rs) and must not be written by hand;
 * these constants mirror `crates/protocol/src/lib.rs` and are checked against
 * it by the smoke script, which talks to the real server.
 */
import type { RoomError } from '@/generated/RoomError';
import type { RoomErrorCode } from '@/generated/RoomErrorCode';

/** The session rides in a header, never a query string. */
export const SESSION_HEADER = 'x-kaboo-session';

export const ROOM_ERROR_MESSAGES: Record<RoomErrorCode, string> = {
  E_NOT_FOUND: 'That room is not open.',
  E_ROOM_FULL: 'That room is full.',
  E_ALREADY_STARTED: 'That game has already started.',
  E_BAD_SESSION: 'That seat is not yours.',
  E_BAD_REQUEST: 'That did not work.',
  E_BUSY: 'The table is busy. Try again.',
};

/** Success types carry no `ok` field, so this is the only discriminator needed. */
export function isRoomError(value: unknown): value is RoomError {
  return typeof value === 'object' && value !== null && (value as { ok?: unknown }).ok === false;
}

/** A transport failure or a malformed body, dressed as a room error. */
export function offline(): RoomError {
  return { ok: false, code: 'E_BUSY', message: ROOM_ERROR_MESSAGES.E_BUSY };
}

/**
 * Reasons to drop a seat: the room expired, or this seat is not ours any more.
 * Retrying on either would be a silent, permanent failure — a frozen table.
 */
export const FATAL: ReadonlySet<RoomErrorCode> = new Set<RoomErrorCode>(['E_NOT_FOUND', 'E_BAD_SESSION']);

/** Mirrors `ROOM_CODE_ALPHABET`: no 0/O, 1/I/L, so a code survives being read aloud. */
export const ROOM_CODE_ALPHABET = 'ABCDEFGHJKMNPQRSTUVWXYZ23456789';

/** Mirrors `MIN_PLAYERS` / `MAX_PLAYERS` in the engine. */
export const MIN_PLAYERS = 2;
export const MAX_PLAYERS = 8;

/**
 * Accept a room code the way a human types it — lowercase, hyphenated, padded —
 * and reduce it to canonical form. Look-alikes are NOT mapped onto each other:
 * that would silently send a player to a different room.
 */
export function normalizeRoomCode(raw: string): string {
  let out = '';
  for (const ch of raw.toUpperCase()) if (ROOM_CODE_ALPHABET.includes(ch)) out += ch;
  return out;
}
