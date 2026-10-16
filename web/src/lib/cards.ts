import type { PublicCard } from '@/generated/PublicCard';

/**
 * The artwork's filename for a card: rank then suit, as the deck ships
 * (`public/cards/QH.png`). The two jokers are ids 52 and 53; the artwork calls
 * them `Joker1`/`Joker2`, and the id is the only thing that tells them apart.
 */
export function cardFile(card: PublicCard): string | null {
  if (card.rank === 'JOKER') {
    if (card.id === 52) return 'Joker1';
    if (card.id === 53) return 'Joker2';
    return null;
  }
  if (!card.suit) return null;
  return `${card.rank}${card.suit}`;
}

const RANKS: Record<string, string> = {
  A: 'Ace',
  '2': 'two',
  '3': 'three',
  '4': 'four',
  '5': 'five',
  '6': 'six',
  '7': 'seven',
  '8': 'eight',
  '9': 'nine',
  '10': 'ten',
  J: 'Jack',
  Q: 'Queen',
  K: 'King',
  JOKER: 'Joker',
};

const SUITS: Record<string, string> = { S: 'spades', H: 'hearts', D: 'diamonds', C: 'clubs' };

/**
 * "Queen of hearts", not "QH": a screen reader will not read the art, so this
 * is the whole of what a player relying on one hears about a card.
 */
export function cardName(card: PublicCard): string {
  const rank = RANKS[card.rank] ?? card.rank;
  const suit = card.suit ? SUITS[card.suit] : null;
  return suit ? `${rank} of ${suit}` : rank;
}
