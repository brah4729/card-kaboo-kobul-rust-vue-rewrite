<script setup lang="ts">
/** The one-line "what is happening / what do I do" pill. */
import { computed } from 'vue';
import type { Projection } from '@/generated/Projection';

const props = defineProps<{ view: Projection; deadline: number; now: number; canTap: boolean }>();
const emit = defineEmits<{ tap: [] }>();

const text = computed(() => {
  const v = props.view;
  const name = (id: string | null) => v.players.find((p) => p.id === id)?.name ?? '?';
  const seconds = props.deadline > props.now ? Math.ceil((props.deadline - props.now) / 1000) : null;

  if (v.phase === 'peek') return 'Look at cards 3 and 4';
  if (v.phase === 'reveal') return 'Reveal';
  if (v.phase === 'gameOver') return 'Game over';
  if (v.phase === 'finalSwap') return `${name(v.turnPlayerId)} drew the last card`;
  if (v.tap) {
    const who = v.tap.playerId === v.youId ? 'You' : name(v.tap.playerId);
    return `${who} tapped — pick a matching card`;
  }
  const turn = v.turnPlayerId === v.youId ? 'Your turn' : `${name(v.turnPlayerId)}'s turn`;
  if (v.effect) return `${turn} · effect in play`;
  return seconds !== null ? `${turn} · ${seconds}s` : turn;
});
</script>

<template>
  <div class="kb-status">
    <span class="kb-chip kb-chip--dark">{{ text }}</span>
    <button v-if="canTap" type="button" class="kb-btn kb-btn--kaboo" @click="emit('tap')">Tap the pile</button>
  </div>
</template>
