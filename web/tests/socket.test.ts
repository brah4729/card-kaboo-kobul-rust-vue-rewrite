import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { DeliveredEvent } from '@/generated/DeliveredEvent';
import { connect, socketUrl } from '@/lib/socket';
import { labelFor } from '@/lib/useGame';

/** A WebSocket the test drives by hand. */
class FakeSocket {
  static all: FakeSocket[] = [];
  readyState = 0;
  sent: string[] = [];
  onopen: (() => void) | null = null;
  onmessage: ((e: { data: string }) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  constructor(public url: string) {
    FakeSocket.all.push(this);
  }
  send(data: string) {
    this.sent.push(data);
  }
  close() {
    this.readyState = 3;
  }
  open() {
    this.readyState = 1;
    this.onopen?.();
  }
  receive(payload: unknown) {
    this.onmessage?.({ data: JSON.stringify(payload) });
  }
  drop() {
    this.readyState = 3;
    this.onclose?.();
  }
}

const session = { code: 'ABCD', playerId: 'p1', name: 'Ana', token: 'tok.en' };
const lobby = { room: { status: 'lobby', code: 'ABCD', you: 'p1', host: true, hostId: 'p1', roster: [], seats: 8 }, events: [], eventSeq: 0 };

describe('the room socket', () => {
  beforeEach(() => {
    FakeSocket.all = [];
    vi.useFakeTimers();
    vi.stubGlobal('fetch', vi.fn(() => Promise.reject(new Error('offline'))));
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  const start = (overrides = {}) => {
    const synced: unknown[] = [];
    const lost: string[] = [];
    const stop = connect({
      session,
      since: () => 7,
      onSync: (s) => synced.push(s),
      onLost: (c) => lost.push(c),
      createSocket: (url) => new FakeSocket(url) as unknown as WebSocket,
      ...overrides,
    });
    return { synced, lost, stop };
  };

  it('never puts the token in the URL', () => {
    start();
    expect(FakeSocket.all[0]!.url).toMatch(/\/api\/rooms\/ABCD\/ws$/);
    expect(FakeSocket.all[0]!.url).not.toContain('tok.en');
  });

  it('authenticates with its first message, from the current cursor', () => {
    start();
    FakeSocket.all[0]!.open();
    expect(JSON.parse(FakeSocket.all[0]!.sent[0]!)).toEqual({ type: 'auth', token: 'tok.en', since: 7 });
  });

  it('hands every sync to the caller', () => {
    const { synced } = start();
    FakeSocket.all[0]!.open();
    FakeSocket.all[0]!.receive(lobby);
    expect(synced).toHaveLength(1);
  });

  it('stops for good on a lost seat, and says why', () => {
    const { lost } = start();
    FakeSocket.all[0]!.open();
    FakeSocket.all[0]!.receive({ ok: false, code: 'E_BAD_SESSION', message: 'That seat is not yours.' });
    expect(lost).toEqual(['E_BAD_SESSION']);
    FakeSocket.all[0]!.drop();
    vi.advanceTimersByTime(60_000);
    expect(FakeSocket.all).toHaveLength(1);
  });

  it('reconnects after a drop, and re-authenticates', () => {
    start();
    FakeSocket.all[0]!.open();
    FakeSocket.all[0]!.drop();
    vi.advanceTimersByTime(600);
    expect(FakeSocket.all).toHaveLength(2);
    FakeSocket.all[1]!.open();
    expect(JSON.parse(FakeSocket.all[1]!.sent[0]!).type).toBe('auth');
  });

  it('opens nothing more once stopped', () => {
    const { stop } = start();
    stop();
    FakeSocket.all[0]!.drop();
    vi.advanceTimersByTime(60_000);
    expect(FakeSocket.all).toHaveLength(1);
  });

  it('builds a wss URL on https', () => {
    expect(socketUrl('AB CD', { protocol: 'https:', host: 'kaboo.example' } as Location)).toBe('wss://kaboo.example/api/rooms/AB%20CD/ws');
  });
});

describe('labelFor — a notice is labelled from its event alone', () => {
  const ev = (e: Partial<DeliveredEvent>) => ({ seq: 1, type: 'peeked', ...e }) as DeliveredEvent;

  it('names your own card by its 1-based slot, briefly', () => {
    expect(labelFor(ev({ playerId: 'p1', slot: 2 }), 'p1')).toBe('slot 3');
  });

  it("names someone else's card by name and slot", () => {
    expect(labelFor(ev({ playerId: 'p2', slot: 0 }), 'p1', { p2: 'Ben' })).toBe("Ben's card · slot 1");
  });

  it("never prints the Queen's source slot: the card is in your hand by then", () => {
    expect(labelFor(ev({ type: 'revealed', playerId: 'p2', slot: 3 }), 'p1')).toBe('the card you were given');
  });

  it('labels a draw and the last card', () => {
    expect(labelFor(ev({ type: 'drew' }), 'p1')).toBe('the card you drew');
    expect(labelFor(ev({ type: 'deckOut' }), 'p1')).toMatch(/last card/);
  });
});
