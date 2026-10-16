import { describe, expect, it } from 'vitest';
import type { Projection } from '@/generated/Projection';
import { activeIntent, selectableFor } from '@/lib/intent';

/** The slice of a projection the intent logic reads, with two players. */
function view(overrides: Partial<Projection> = {}): Projection {
  const hand = (n: number) => Array.from({ length: n }, (_, index) => ({ index, occupied: true, gen: 0, face: null }));
  return {
    youId: 'p1',
    tap: null,
    players: [
      { id: 'p1', hand: hand(4) },
      { id: 'p2', hand: [...hand(3), { index: 3, occupied: false, gen: 1, face: null }] },
    ],
    ...overrides,
  } as unknown as Projection;
}

describe('activeIntent — the tap bug (AGENTS.md §7)', () => {
  it('puts a player who claimed a tap into choosing, with no button pressed', () => {
    const v = view({ tap: { playerId: 'p1', deadline: 0, awaitingPlacement: false, source: null } });
    expect(activeIntent(v, 'none')).toBe('tapChoose');
  });

  it('moves them on to placing once the card came from another hand', () => {
    const v = view({ tap: { playerId: 'p1', deadline: 0, awaitingPlacement: true, source: { playerId: 'p2', slot: 0 } } });
    expect(activeIntent(v, 'swap')).toBe('tapPlace');
  });

  it("leaves everyone else's intent alone during somebody else's tap", () => {
    const v = view({ tap: { playerId: 'p2', deadline: 0, awaitingPlacement: false, source: null } });
    expect(activeIntent(v, 'swap')).toBe('swap');
  });

  it('makes cards on the table pickable once a tap is claimed', () => {
    const v = view({ tap: { playerId: 'p1', deadline: 0, awaitingPlacement: false, source: null } });
    const map = selectableFor(v, activeIntent(v, 'none'));
    expect(map.get('p1')).toEqual([0, 1, 2, 3]);
    expect(map.get('p2')).toEqual([0, 1, 2]);
  });
});

describe('selectableFor', () => {
  it('offers only your own cards for a swap', () => {
    expect([...selectableFor(view(), 'swap').keys()]).toEqual(['p1']);
  });

  it("never offers your own cards for a look at someone else's", () => {
    expect([...selectableFor(view(), 'effectOther').keys()]).toEqual(['p2']);
  });

  it("never offers your own cards as the Queen's target — the engine would refuse it", () => {
    expect([...selectableFor(view(), 'queenTarget').keys()]).toEqual(['p2']);
  });

  it('offers every occupied card on the table for a blind swap, and never an empty slot', () => {
    const map = selectableFor(view(), 'blindFirst');
    expect(map.get('p1')).toHaveLength(4);
    expect(map.get('p2')).toEqual([0, 1, 2]);
  });

  it('offers nothing with no intent', () => {
    expect(selectableFor(view(), 'none').size).toBe(0);
  });
});
