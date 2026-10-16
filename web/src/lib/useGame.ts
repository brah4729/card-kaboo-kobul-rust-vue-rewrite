import { computed, onMounted, onUnmounted, ref, shallowRef, watch, type Ref, type ShallowRef } from 'vue';
import type { Action } from '@/generated/Action';
import type { DeliveredEvent } from '@/generated/DeliveredEvent';
import type { PublicCard } from '@/generated/PublicCard';
import type { RoomError } from '@/generated/RoomError';
import type { RoomView } from '@/generated/RoomView';
import type { SyncResponse } from '@/generated/SyncResponse';
import { leaveRoom, sendAction, startRoom } from './api';
import { FATAL, isRoomError } from './protocol';
import type { StoredSession } from './session';
import { connect } from './socket';

/**
 * THE GAME, AS THIS DEVICE SEES IT.
 *
 * The engine is not in the browser at all. Every fact on screen arrived from
 * the server as a `RoomView` already projected for this one player, so the UI
 * cannot show a card the seat is not entitled to — it was never sent.
 *
 * What is left for the client to own:
 * * **The cursor.** A monotonic `eventSeq` the client advances itself, so a
 *   push and a response that carry the same peek count it once.
 * * **Peeks.** Private events become transient notices and are then forgotten.
 * * **A display clock**, for counting deadlines down. The server's alarm decides
 *   when they actually fire; this only animates the number.
 */

/** A card the viewer was told about, and must now remember. */
export interface Peek {
  /** The server's `seq`, so a push and a replay agree it is one peek. */
  key: number;
  card: PublicCard;
  /** What the look was about: "slot 3", "Bo's card · slot 1". */
  label: string;
  /**
   * The round the notice arrived in. A notice is kept only for its round,
   * which stops the tray growing without limit — at the reveal every card is
   * face-up anyway, so a reminder has stopped being information.
   */
  round: number;
}

export interface Game {
  room: ShallowRef<RoomView | null>;
  /** Browser time, for animating a countdown. Never used to decide anything. */
  now: Ref<number>;
  peeks: Ref<Peek[]>;
  /** A move was refused. Deliberately a bare flag: the reason may be hidden info. */
  refused: Ref<boolean>;
  /** The socket is open; `false` means updates arrive more slowly over HTTP. */
  live: Ref<boolean>;
  busy: Ref<boolean>;
  send(action: Action): void;
  deal(): void;
  dismissPeeks(): void;
  leave(): void;
}

/**
 * A human label for a private notice, built from the event alone — never from
 * the view or the state, or it becomes somewhere extra information can creep in.
 */
export function labelFor(event: DeliveredEvent, viewerId: string, names: Record<string, string> = {}): string {
  const who = (id: string) => (id === viewerId ? 'your' : `${names[id] ?? id}'s`);
  switch (event.type) {
    case 'deckOut':
      return 'the last card — only you saw it';
    case 'drew':
      return 'the card you drew';
    case 'revealed':
      // A Queen's `revealed` carries the slot the card came FROM; it is in the
      // viewer's own hand by now, so printing that number would point at the
      // wrong card.
      return 'the card you were given';
    case 'peeked': {
      if (event.playerId === undefined) return 'a card';
      // Named by slot, 1-based, the way the table counts. Bare "slot 3" for
      // your own card: the caption sets the tray's width, and a longer one
      // wraps the tray onto a second row on a 360px phone (AGENTS.md §9).
      if (typeof event.slot !== 'number') return `${who(event.playerId)} card`;
      return event.playerId === viewerId
        ? `slot ${event.slot + 1}`
        : `${who(event.playerId)} card · slot ${event.slot + 1}`;
    }
    default:
      return 'a card';
  }
}

/** The names on the table, for labels. Empty in the lobby. */
function namesOf(room: RoomView): Record<string, string> {
  if (room.status !== 'playing') return {};
  return Object.fromEntries(room.view.players.map((p) => [p.id, p.name]));
}

/**
 * The live connection for one seat. Mounted ONCE, above both the lobby and
 * the table, so the deal does not tear down the socket or reset the cursor.
 */
export function useGame(session: StoredSession, onLost: () => void): Game {
  const room = shallowRef<RoomView | null>(null);
  const peeks = ref<Peek[]>([]);
  const refused = ref(false);
  const live = ref(false);
  const busy = ref(false);
  const now = ref(Date.now());

  /** Not reactive: read by the socket at each (re)connect. */
  let cursor = 0;

  function apply(sync: SyncResponse): void {
    const seen = cursor;
    // Only what this client has not already been told.
    const fresh = sync.events.filter((e) => e.seq > seen && e.card !== undefined);
    if (fresh.length > 0) {
      const round = sync.room.status === 'playing' ? sync.room.view.round : 0;
      const names = namesOf(sync.room);
      peeks.value = [
        // Prune-on-insert: notices from an earlier round go, here and only here.
        ...peeks.value.filter((p) => p.round === round),
        ...fresh.map((e) => ({ key: e.seq, card: e.card!, label: labelFor(e, sync.room.you, names), round })),
      ];
    }
    // `eventSeq` is room-wide, so advancing to it is correct: every event
    // addressed to this player carries a seq at or below it.
    if (sync.eventSeq > cursor) cursor = sync.eventSeq;
    room.value = sync.room;
  }

  const stop = connect({
    session,
    since: () => cursor,
    onSync: apply,
    onLive: (value) => (live.value = value),
    onLost: () => onLost(),
  });

  const clock = window.setInterval(() => (now.value = Date.now()), 1000);
  onUnmounted(() => {
    stop();
    window.clearInterval(clock);
  });

  useWakeLock(
    computed(() => {
      const r = room.value;
      return r?.status === 'playing' && r.view.turnPlayerId === r.you && r.view.phase !== 'gameOver';
    }),
  );

  function run(work: Promise<SyncResponse | RoomError>): void {
    busy.value = true;
    void work
      .then((result) => {
        if (isRoomError(result)) {
          refused.value = true;
          if (FATAL.has(result.code)) onLost();
          return;
        }
        // A refusal still carries the fresh view — that is what tells us our
        // own view was stale. Show it, and say nothing more about why.
        refused.value = (result as { refusal?: unknown }).refusal !== undefined;
        apply(result);
      })
      .finally(() => (busy.value = false));
  }

  return {
    room,
    now,
    peeks,
    refused,
    live,
    busy,
    send: (action) => run(sendAction(session, action)),
    deal: () => run(startRoom(session)),
    dismissPeeks: () => (peeks.value = []),
    // Leaving is a courtesy to the table, so it never blocks the button.
    leave: () => void leaveRoom(session),
  };
}

/**
 * Keep the screen awake while this player has something to do: a phone that
 * sleeps mid-turn looks like a hang to everybody else. Best-effort throughout,
 * and re-acquired when the tab comes back, because browsers drop it on hide.
 */
function useWakeLock(active: Ref<boolean>): void {
  type Sentinel = { release: () => Promise<void> };
  let sentinel: Sentinel | null = null;

  async function acquire(): Promise<void> {
    const wakeLock = (navigator as Navigator & { wakeLock?: { request(type: 'screen'): Promise<Sentinel> } })
      .wakeLock;
    if (!wakeLock || !active.value || document.visibilityState !== 'visible' || sentinel) return;
    try {
      const lock = await wakeLock.request('screen');
      if (active.value) sentinel = lock;
      else void lock.release();
    } catch {
      // Denied or unsupported. Nothing to fix.
    }
  }

  function release(): void {
    void sentinel?.release().catch(() => {});
    sentinel = null;
  }

  const onVisible = () => {
    if (document.visibilityState === 'visible') {
      sentinel = null;
      void acquire();
    }
  };
  onMounted(() => document.addEventListener('visibilitychange', onVisible));
  onUnmounted(() => {
    document.removeEventListener('visibilitychange', onVisible);
    release();
  });
  watch(active, (on) => (on ? void acquire() : release()), { immediate: true });
}
