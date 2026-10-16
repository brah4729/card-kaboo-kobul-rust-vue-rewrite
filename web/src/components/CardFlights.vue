<script setup lang="ts">
/**
 * CARD FLIGHTS — the table moving, instead of the table changing.
 *
 * Every new snapshot is compared with the last (`planFlights`), and each card
 * that moved is drawn as a "ghost" travelling from where it was to where it is
 * now, while the real card at the destination stays hidden until it lands. The
 * state is already correct the instant it arrives; the ghost only shows the
 * player where it came from.
 *
 * Ghosts live outside Vue, in `document.body`, so a re-render mid-flight cannot
 * cancel one, and they never take a click. Positions come from `data-flight`
 * anchors (Hand for slots, GameScreen for the piles).
 *
 * The watcher is `flush: 'post'`: it runs after the DOM is patched and before
 * the browser paints, so a destination is hidden before it can flash into place.
 * Under `prefers-reduced-motion` the ghost is skipped and the landing ring kept:
 * the movement is decoration, "this is the card that changed" is not.
 */
import { onUnmounted, watch } from 'vue';
import type { Projection } from '@/generated/Projection';
import type { PublicCard } from '@/generated/PublicCard';
import { cardFile } from '@/lib/cards';
import { placeKey, planFlights, type Move, type Place } from '@/lib/flights';

const props = defineProps<{ view: Projection }>();

/** Long enough to follow with the eye, short enough that nobody waits on it. */
const FLIGHT_MS = 420;
/** How long the ring stays on the card that arrived. */
const LANDED_MS = 1200;
const EASE = 'cubic-bezier(0.4, 0, 0.2, 1)';

const inFlight = new Set<() => void>();

watch(
  () => props.view,
  (view, before) => {
    if (!before || document.visibilityState !== 'visible') return;
    const moves = planFlights(before, view);
    if (moves.length === 0) return;
    const reduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    for (const move of moves) {
      const cancel = fly(move, reduced);
      if (cancel) inFlight.add(cancel);
    }
  },
  { flush: 'post' },
);

// Leaving the table mid-flight must not leave a card hanging over the page.
onUnmounted(() => {
  for (const cancel of inFlight) cancel();
  inFlight.clear();
});

function anchorFor(place: Place): HTMLElement | null {
  const key = placeKey(place).replace(/["\\]/g, '\\$&');
  return document.querySelector<HTMLElement>(`[data-flight="${key}"]`);
}

/** The card inside an anchor, or the anchor itself when it is the card. */
const cardIn = (el: HTMLElement): HTMLElement => el.querySelector<HTMLElement>('.kb-card') ?? el;

function fly(move: Move, reduced: boolean): (() => void) | null {
  const fromEl = anchorFor(move.from);
  const toEl = anchorFor(move.to);
  if (!fromEl || !toEl) return null;

  // A hand scrolls sideways, so a card can land out of sight: bring the spot
  // into view first (`nearest` is a no-op when it already is).
  if (move.to.zone === 'slot') toEl.scrollIntoView({ block: 'nearest', inline: 'nearest' });

  const target = cardIn(toEl);
  const a = fromEl.getBoundingClientRect();
  const b = toEl.getBoundingClientRect();
  if (a.width === 0 || b.width === 0) return null;

  if (reduced) {
    markLanded(target);
    return null;
  }

  const ghost = document.createElement('div');
  ghost.className = 'kb-card kb-ghost';
  ghost.setAttribute('aria-hidden', 'true');
  Object.assign(ghost.style, {
    left: `${a.left}px`,
    top: `${a.top}px`,
    width: `${a.width}px`,
    height: `${a.height}px`,
    transformOrigin: 'top left',
  });

  // One face when the card looks the same at both ends; two, crossfaded at the
  // midpoint, when it turns over on the way.
  const start = faceLayer(move.startFace);
  ghost.append(start);
  let end: HTMLElement | null = null;
  if (move.startFace?.id !== move.endFace?.id) {
    end = faceLayer(move.endFace);
    end.style.opacity = '0';
    ghost.append(end);
  }
  document.body.append(ghost);

  const visibility = target.style.visibility;
  target.style.visibility = 'hidden';

  const motion = ghost.animate(
    [
      { transform: 'translate(0px, 0px) scale(1)' },
      { transform: `translate(${b.left - a.left}px, ${b.top - a.top}px) scale(${b.width / a.width})` },
    ],
    { duration: FLIGHT_MS, easing: EASE, fill: 'forwards' },
  );
  if (end) {
    const timing: KeyframeAnimationOptions = { duration: FLIGHT_MS, fill: 'forwards' };
    start.animate([{ opacity: 1 }, { opacity: 1, offset: 0.45 }, { opacity: 0, offset: 0.55 }, { opacity: 0 }], timing);
    end.animate([{ opacity: 0 }, { opacity: 0, offset: 0.45 }, { opacity: 1, offset: 0.55 }, { opacity: 1 }], timing);
  }

  let cleaned = false;
  const cleanup = () => {
    if (cleaned) return;
    cleaned = true;
    ghost.remove();
    target.style.visibility = visibility;
  };
  const cancel = () => {
    motion.cancel();
    cleanup();
  };

  motion.finished.then(
    () => {
      cleanup();
      markLanded(target);
      inFlight.delete(cancel);
    },
    () => {
      cleanup();
      inFlight.delete(cancel);
    },
  );
  return cancel;
}

/** One face of a ghost: the card's art, or the CSS back when it is face-down. */
function faceLayer(card: PublicCard | null): HTMLElement {
  const layer = document.createElement('div');
  const file = card ? cardFile(card) : null;
  if (file) {
    layer.className = 'kb-ghost__layer';
    const img = document.createElement('img');
    img.className = 'kb-card__face';
    img.src = `/cards/${file}.png`;
    img.alt = '';
    img.draggable = false;
    layer.append(img);
  } else {
    layer.className = 'kb-ghost__layer kb-ghost__layer--back';
    const back = document.createElement('span');
    back.className = 'kb-card__back';
    layer.append(back);
  }
  return layer;
}

/** A ring on the card that just arrived: the answer to "where did it go?" */
function markLanded(el: HTMLElement): void {
  el.classList.add('kb-card--landed');
  window.setTimeout(() => el.classList.remove('kb-card--landed'), LANDED_MS);
}
</script>

<template>
  <span hidden />
</template>
