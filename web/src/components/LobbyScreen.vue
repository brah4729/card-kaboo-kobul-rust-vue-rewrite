<script setup lang="ts">
/**
 * The lobby: read the code out, and wait. The code is the only thing anybody
 * has to communicate, so it is the largest thing on the screen. The roster is
 * shown rather than a count, because "has Dave arrived yet" is the question.
 */
import { computed } from 'vue';
import type { RoomView } from '@/generated/RoomView';
import { MAX_PLAYERS, MIN_PLAYERS } from '@/lib/protocol';

type Lobby = Extract<RoomView, { status: 'lobby' }>;

const props = defineProps<{ room: Lobby; busy: boolean; refused: boolean }>();
const emit = defineEmits<{ deal: []; leave: [] }>();

const host = computed(() => props.room.roster.find((r) => r.id === props.room.hostId));
const needed = computed(() => MIN_PLAYERS - props.room.roster.length);
const note = computed(() => {
  if (needed.value > 0) return `Waiting for ${needed.value} more ${needed.value === 1 ? 'player' : 'players'}.`;
  return props.room.host ? 'Everyone is here.' : `Waiting for ${host.value?.name ?? 'the host'} to deal.`;
});
</script>

<template>
  <main class="kb-table kb-table--gate">
    <h1 class="kb-title">Kaboo</h1>
    <p class="kb-gate__body">Read this out. Everyone joins from their own phone.</p>

    <span class="kb-chip kb-chip--dark kb-chip--room" :aria-label="`Room code ${room.code}`">{{ room.code }}</span>

    <section class="kb-panel" aria-label="Players">
      <h2 class="kb-panel__title kb-header">{{ room.roster.length }} of {{ MAX_PLAYERS }} here</h2>
      <ol class="kb-scores">
        <li v-for="r in room.roster" :key="r.id" :class="['kb-scores__row', { 'kb-scores__row--you': r.id === room.you }]">
          <span class="kb-scores__name">
            {{ r.name }}
            <span v-if="r.id === room.you" class="kb-chip kb-chip--soft">you</span>
            <span v-if="r.id === host?.id" class="kb-chip kb-chip--dark">host</span>
          </span>
        </li>
      </ol>
      <p class="kb-panel__note">{{ note }}</p>
    </section>

    <button v-if="room.host" type="button" class="kb-btn kb-btn--primary" :disabled="needed > 0 || busy" @click="emit('deal')">
      {{ needed > 0 ? `Need ${needed} more` : 'Deal the cards' }}
    </button>
    <p v-else class="kb-label" role="status">Waiting for the host…</p>

    <p v-if="refused" class="kb-error" role="alert">That did not work.</p>

    <button type="button" class="kb-btn kb-btn--small" @click="emit('leave')">Leave</button>
  </main>
</template>
