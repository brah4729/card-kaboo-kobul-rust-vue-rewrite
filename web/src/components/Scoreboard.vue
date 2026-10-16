<script setup lang="ts">
/**
 * Cumulative scores only. A player's own live total is never shown, and the
 * projection never computes one — so there is nothing here to leak.
 */
import type { Projection } from '@/generated/Projection';

defineProps<{ view: Projection }>();
</script>

<template>
  <section class="kb-panel kb-panel--scores" aria-label="Scoreboard">
    <h2 class="kb-panel__title kb-header">Scores</h2>
    <ol class="kb-scores">
      <li
        v-for="p in view.players"
        :key="p.id"
        :class="['kb-scores__row', { 'kb-scores__row--turn': p.isTurn, 'kb-scores__row--you': p.id === view.youId }]"
      >
        <span class="kb-scores__name">
          {{ p.name }}
          <span v-if="p.id === view.youId" class="kb-chip kb-chip--soft">you</span>
          <span v-if="p.calledKaboo" class="kb-chip kb-chip--dark">kaboo</span>
        </span>
        <span class="kb-scores__cards" :aria-label="`${p.handCount} cards`">{{ p.handCount }}</span>
        <span class="kb-scores__total">{{ p.score }}</span>
      </li>
    </ol>
    <p class="kb-panel__note">Round {{ view.round }} of {{ view.rounds }} · draw pile {{ view.drawPileCount }}</p>
  </section>
</template>
