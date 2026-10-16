<script setup lang="ts">
/**
 * A hand: one row of cards that scrolls sideways.
 *
 * Slots are rendered by index and never compacted: a player remembers
 * positions, so an emptied slot stays visibly empty and in place. A hand can be
 * longer than four (penalty cards), so the row scrolls rather than refusing
 * the extra. See AGENTS.md §9.
 */
import type { SlotView } from '@/generated/SlotView';
import { placeKey } from '@/lib/flights';
import Card from './Card.vue';

withDefaults(
  defineProps<{
    slots: SlotView[];
    label: string;
    /** Slot indices the caller may tap right now. Everything else is inert. */
    selectableSlots?: number[];
    /**
     * The subset of `selectableSlots` to draw attention to. Separate, because
     * a Jack makes every card on the table selectable, and a table where
     * everything pulses says nothing.
     */
    liveSlots?: number[];
    selectedSlot?: number | null;
    compact?: boolean;
    /** Whose hand this is: names each slot's `data-flight` anchor for `CardFlights`. */
    ownerId?: string;
  }>(),
  { selectableSlots: () => [], liveSlots: () => [], selectedSlot: null, compact: false },
);

const emit = defineEmits<{ select: [slot: number] }>();
</script>

<template>
  <div :class="['kb-hand', { 'kb-hand--compact': compact }]" :aria-label="label" role="group">
    <div
      v-for="slot in slots"
      :key="slot.index"
      class="kb-hand__cell"
      :data-flight="ownerId ? placeKey({ zone: 'slot', playerId: ownerId, slot: slot.index }) : undefined"
    >
      <!-- `gen` in the key: when the slot's contents change the card remounts.
           A null face means face-down, and no component may overrule that. -->
      <Card
        :key="`${slot.index}:${slot.gen}`"
        :card="slot.face"
        :face-down="slot.occupied && !slot.face"
        :empty="!slot.occupied"
        :selected="selectedSlot === slot.index"
        :live="liveSlots.includes(slot.index)"
        :label="`${label} slot ${slot.index + 1}`"
        :clickable="selectableSlots.includes(slot.index)"
        @select="emit('select', slot.index)"
      />
      <span class="kb-hand__index" aria-hidden="true">{{ slot.index + 1 }}</span>
    </div>
  </div>
</template>
