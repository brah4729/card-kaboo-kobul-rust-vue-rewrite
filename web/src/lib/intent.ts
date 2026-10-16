import type { Projection } from '@/generated/Projection';

/**
 * What the player is in the middle of choosing. Each needs a slot picked; the
 * button that started it set it — except a claimed tap, which no button starts.
 */
export type Intent =
  | 'none'
  | 'swap'
  | 'finalSwap'
  | 'tapChoose'
  | 'tapPlace'
  | 'effectOwn'
  | 'effectOther'
  | 'blindFirst'
  | 'blindSecond'
  | 'queenOwn'
  | 'queenTarget'
  | 'kingFirst'
  | 'kingSecond';

/**
 * The intents whose every choice is a card of the player's own — the only ones
 * where lighting up the player's hand is the honest cue. A tap, a blind swap or
 * a King look can be answered from any hand, and lighting only the player's own
 * cards would mislead them into a wrong tap and a penalty card.
 */
export const OWN_HAND_DECISIONS: ReadonlySet<Intent> = new Set<Intent>([
  'swap',
  'finalSwap',
  'tapPlace',
  'effectOwn',
  'queenOwn',
]);

/**
 * The intent actually in force.
 *
 * A claimed tap is the one decision a player does not start with a button: the
 * claim puts them in it and `legalActions` then reads TAP_CHOOSE. So it is
 * derived from the VIEW, and it wins while it lasts. Without this, every card
 * stayed inert after a claim and the tap could only end on its deadline — as a
 * wrong tap and a penalty card (AGENTS.md §7, the tap bug).
 */
export function activeIntent(view: Projection, chosen: Intent): Intent {
  if (view.tap?.playerId === view.youId) return view.tap.awaitingPlacement ? 'tapPlace' : 'tapChoose';
  return chosen;
}

/** Which slots of which hands are pickable for `intent`, from the projection alone. */
export function selectableFor(view: Projection, intent: Intent): Map<string, number[]> {
  const map = new Map<string, number[]>();
  const occupied = (id: string) =>
    (view.players.find((p) => p.id === id)?.hand ?? []).filter((s) => s.occupied).map((s) => s.index);
  const you = view.youId;

  switch (intent) {
    case 'swap':
    case 'finalSwap':
    case 'tapPlace':
    case 'effectOwn':
    case 'queenOwn':
      map.set(you, occupied(you));
      break;
    case 'effectOther':
      // Another player's card — never your own.
      for (const p of view.players) if (p.id !== you) map.set(p.id, occupied(p.id));
      break;
    // A tap may match from ANY hand, including your own, and so may a blind
    // swap or a King look; a Queen's target is any other card. AGENTS.md §5.
    case 'queenTarget':
      for (const p of view.players) if (p.id !== you) map.set(p.id, occupied(p.id));
      break;
    case 'tapChoose':
    case 'blindFirst':
    case 'blindSecond':
    case 'kingFirst':
    case 'kingSecond':
      for (const p of view.players) map.set(p.id, occupied(p.id));
      break;
    default:
      break;
  }
  return map;
}
