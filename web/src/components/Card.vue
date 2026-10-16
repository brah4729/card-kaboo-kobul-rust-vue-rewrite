<script setup lang="ts">
/**
 * A playing card: the real thing, from a public-domain deck.
 *
 * Faces are PNGs under `/public/cards`, rasterised at 3x the widest the UI ever
 * draws a card. The back is CSS, in the design system's own ink-block language,
 * so a face-down card can never be mistaken for a face-up one. See AGENTS.md §8.
 *
 * Callers key this on the slot's `gen`, so a slot's contents changing remounts
 * the card — that is why the projection ships `gen` at all: a face-down card
 * cannot ship its id without leaking its face (AGENTS.md §7).
 */
import { computed } from 'vue';
import type { PublicCard } from '@/generated/PublicCard';
import { cardFile, cardName } from '@/lib/cards';

const props = withDefaults(
  defineProps<{
    card?: PublicCard | null;
    faceDown?: boolean;
    selected?: boolean;
    dimmed?: boolean;
    /**
     * This card is what the current decision is about: the loudest thing on the
     * table. Only ever set on a card the viewer may act on right now.
     */
    live?: boolean;
    empty?: boolean;
    label?: string;
    /** Render as a real <button>. Without it the card is an image. */
    clickable?: boolean;
    disabled?: boolean;
  }>(),
  { card: null, faceDown: false, selected: false, dimmed: false, live: false, empty: false, clickable: false, disabled: false },
);

const emit = defineEmits<{ select: [] }>();

const classes = computed(() => ({
  'kb-card': true,
  'kb-card--down': props.faceDown,
  'kb-card--selected': props.selected,
  'kb-card--dimmed': props.dimmed,
  // Selection wins: the two share the ring, and a committed card answers a
  // different question than "which card is live".
  'kb-card--live': props.live && !props.selected,
}));

const file = computed(() => (props.faceDown || !props.card ? null : cardFile(props.card)));
const name = computed(() => (props.faceDown ? 'face-down card' : props.card ? cardName(props.card) : 'card'));
const accessible = computed(() => (props.label ? `${props.label}: ${name.value}` : name.value));
</script>

<template>
  <div v-if="empty" class="kb-card kb-card--empty" aria-hidden="true" />
  <!-- Real <button> elements: keyboard, focus ring and hit target come free,
       and a div with a click handler is not a control. AGENTS.md §9. -->
  <button
    v-else-if="clickable"
    type="button"
    :class="classes"
    :disabled="disabled"
    :aria-label="accessible"
    :aria-pressed="selected"
    @click="emit('select')"
  >
    <!-- Empty alt: the control already carries the card's name. -->
    <img v-if="file" class="kb-card__face" :src="`/cards/${file}.png`" alt="" draggable="false" />
    <span v-else class="kb-card__back" aria-hidden="true" />
  </button>
  <div v-else :class="classes" role="img" :aria-label="accessible">
    <img v-if="file" class="kb-card__face" :src="`/cards/${file}.png`" alt="" draggable="false" />
    <span v-else class="kb-card__back" aria-hidden="true" />
  </div>
</template>
