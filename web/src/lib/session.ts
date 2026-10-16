import { readonly, ref } from 'vue';

/**
 * The seat, as remembered by this device.
 *
 * The session token is the ONLY thing that proves which player this browser
 * is, so it is what makes a refresh, a reconnect or a suspended tab survivable.
 * There are no accounts: losing this blob means losing the seat.
 *
 * One reactive ref, read once at start-up and kept in step with the `storage`
 * event, so two tabs on one machine agree: a player who leaves in one tab must
 * not stay seated in the other, because the seat belongs to the device. (The
 * React version needed `useSyncExternalStore` to avoid a server-rendered flash
 * of the join form; a client-only SPA has no server render to disagree with.)
 */

const KEY = 'kaboo.session';

export interface StoredSession {
  code: string;
  playerId: string;
  name: string;
  token: string;
}

function parse(raw: string | null): StoredSession | null {
  if (!raw) return null;
  try {
    const s = JSON.parse(raw) as Record<string, unknown>;
    if (
      typeof s !== 'object' ||
      s === null ||
      typeof s.code !== 'string' ||
      typeof s.playerId !== 'string' ||
      typeof s.name !== 'string' ||
      typeof s.token !== 'string'
    ) {
      return null;
    }
    return { code: s.code, playerId: s.playerId, name: s.name, token: s.token };
  } catch {
    return null;
  }
}

/**
 * `localStorage` throws in a private window with site data blocked, and some
 * webviews lack it. That degrades to "this device remembers nothing", which is
 * exactly a first-time visitor — never to a game that cannot start.
 */
function load(): StoredSession | null {
  try {
    return parse(window.localStorage.getItem(KEY));
  } catch {
    return null;
  }
}

const current = ref<StoredSession | null>(load());

window.addEventListener('storage', (event) => {
  if (event.key === KEY || event.key === null) current.value = load();
});

/** The seat this device holds. Read-only: change it with `saveSession`/`clearSession`. */
export const session = readonly(current);

export function saveSession(next: StoredSession): void {
  current.value = next;
  try {
    window.localStorage.setItem(KEY, JSON.stringify(next));
  } catch {
    // The game still works; this device just will not recognise the seat after a refresh.
  }
}

export function clearSession(): void {
  current.value = null;
  try {
    window.localStorage.removeItem(KEY);
  } catch {
    // As above.
  }
}
