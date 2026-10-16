<script setup lang="ts">
/**
 * Transient notices for cards this viewer was privately told about. Not
 * state and not remembered: the peek arrived as an event, and once dismissed
 * it is gone. A player who does not read their notice has forgotten the card.
 */
import type { Peek } from '@/lib/useGame';
import Card from './Card.vue';

defineProps<{ peeks: Peek[] }>();
const emit = defineEmits<{ dismiss: [] }>();
</script>

<template>
  <aside v-if="peeks.length > 0" class="kb-peeks" role="status">
    <div class="kb-peeks__head">
      <span class="kb-chip kb-chip--soft">you saw</span>
      <button type="button" class="kb-btn kb-btn--small kb-btn--primary" @click="emit('dismiss')">Remember it</button>
    </div>
    <div class="kb-peeks__cards">
      <figure v-for="p in peeks" :key="p.key" class="kb-peeks__item">
        <Card :card="p.card" :label="p.label" />
        <figcaption class="kb-label">{{ p.label }}</figcaption>
      </figure>
    </div>
  </aside>
</template>
