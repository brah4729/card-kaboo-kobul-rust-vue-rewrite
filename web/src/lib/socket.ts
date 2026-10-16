import type { RoomErrorCode } from '@/generated/RoomErrorCode';
import type { SyncResponse } from '@/generated/SyncResponse';
import { readState } from './api';
import { FATAL, isRoomError } from './protocol';
import type { StoredSession } from './session';

/**
 * THE SOCKET — the room's own WebSocket, with HTTP only as a fallback.
 *
 * The room is a Durable Object with its own clock (an alarm), so deadlines fire
 * on the server whether or not anyone is connected. That is what let polling
 * go: the socket only has to carry news, never to keep the table alive.
 *
 * * **Auth is the first message**, never the URL: a browser cannot set headers
 *   on a WebSocket, and a URL ends up in logs and history. The server answers
 *   with a full `SyncResponse`, so connecting IS the initial load and a
 *   reconnect IS the resync.
 * * **Delivery is at-least-once.** A push carries the whole private queue; the
 *   caller dedupes on each event's `seq` against one cursor, so a replay after a
 *   reconnect is harmless.
 * * **Fallback:** while the socket is down, the state is read over HTTP every
 *   few seconds, so a network that blocks WebSockets still plays — slowly.
 */

/** First reconnect delay; doubles to `MAX_BACKOFF_MS`. */
const BACKOFF_MS = 500;
const MAX_BACKOFF_MS = 10_000;
/** How often to read over HTTP while the socket is down. */
const FALLBACK_MS = 4_000;

export interface SocketOptions {
  session: StoredSession;
  /** The cursor this client has already seen. Read at each (re)connect. */
  since: () => number;
  onSync: (sync: SyncResponse) => void;
  /** `true` while the socket is open and authenticated. */
  onLive?: (live: boolean) => void;
  /** The seat is gone. The socket has already stopped itself. */
  onLost?: (code: RoomErrorCode) => void;
  /** For tests. */
  createSocket?: (url: string) => WebSocket;
}

export function socketUrl(code: string, location: Location = window.location): string {
  const scheme = location.protocol === 'https:' ? 'wss' : 'ws';
  return `${scheme}://${location.host}/api/rooms/${encodeURIComponent(code)}/ws`;
}

/** Start watching the room. Returns the function that stops it. */
export function connect(options: SocketOptions): () => void {
  const { session, since, onSync, onLive, onLost } = options;
  const create = options.createSocket ?? ((url: string) => new WebSocket(url));

  let stopped = false;
  let ws: WebSocket | null = null;
  let backoff = BACKOFF_MS;
  let reconnectTimer: ReturnType<typeof setTimeout> | undefined;
  let fallbackTimer: ReturnType<typeof setTimeout> | undefined;
  let live = false;

  function setLive(next: boolean): void {
    if (live === next) return;
    live = next;
    onLive?.(next);
    if (next) clearTimeout(fallbackTimer);
    else scheduleFallback(0);
  }

  function stop(): void {
    stopped = true;
    clearTimeout(reconnectTimer);
    clearTimeout(fallbackTimer);
    document.removeEventListener('visibilitychange', onVisibility);
    window.removeEventListener('online', onOnline);
    ws?.close(1000);
    ws = null;
  }

  function lose(code: RoomErrorCode): void {
    stop();
    onLost?.(code);
  }

  function auth(): void {
    ws?.send(JSON.stringify({ type: 'auth', token: session.token, since: since() }));
  }

  function open(): void {
    if (stopped) return;
    const socket = create(socketUrl(session.code));
    ws = socket;
    socket.onopen = () => auth();
    socket.onmessage = (event) => {
      let payload: unknown;
      try {
        payload = JSON.parse(String(event.data));
      } catch {
        return;
      }
      if (isRoomError(payload)) {
        if (FATAL.has(payload.code)) lose(payload.code);
        return;
      }
      const sync = payload as Partial<SyncResponse> | null;
      // Written by our own server, but it crossed a network: checked, not trusted.
      if (typeof sync !== 'object' || sync === null || typeof sync.room !== 'object' || sync.room === null) return;
      backoff = BACKOFF_MS;
      setLive(true);
      onSync({ room: sync.room, events: sync.events ?? [], eventSeq: sync.eventSeq ?? 0 });
    };
    socket.onclose = () => {
      if (ws !== socket) return;
      ws = null;
      setLive(false);
      if (stopped) return;
      reconnectTimer = setTimeout(open, backoff);
      backoff = Math.min(backoff * 2, MAX_BACKOFF_MS);
    };
    // An error is always followed by a close, which does the work.
    socket.onerror = () => {};
  }

  function scheduleFallback(delay: number): void {
    clearTimeout(fallbackTimer);
    fallbackTimer = setTimeout(async () => {
      if (stopped || live) return;
      const result = await readState(session, since());
      if (stopped) return;
      if (isRoomError(result)) {
        if (FATAL.has(result.code)) {
          lose(result.code);
          return;
        }
      } else if (!live) {
        onSync(result);
      }
      scheduleFallback(FALLBACK_MS);
    }, delay);
  }

  /** A tab coming back, or a network coming back: catch up now. */
  function wake(): void {
    if (stopped || document.visibilityState === 'hidden') return;
    if (ws && ws.readyState === WebSocket.OPEN) {
      auth(); // re-auth answers with a fresh sync from our cursor
    } else if (!ws) {
      clearTimeout(reconnectTimer);
      backoff = BACKOFF_MS;
      open();
    }
  }
  const onVisibility = () => wake();
  const onOnline = () => wake();
  document.addEventListener('visibilitychange', onVisibility);
  window.addEventListener('online', onOnline);

  open();
  // Until the socket says otherwise, the HTTP read is what draws the first frame.
  scheduleFallback(1_500);
  return stop;
}
