<script setup lang="ts">
/**
 * Every line is `who did what`. `LogEntry` carries no card face at all, so
 * this panel cannot leak one even by mistake (AGENTS.md §7).
 */
import { computed } from 'vue';
import type { Projection } from '@/generated/Projection';

const props = defineProps<{ view: Projection }>();

const names = computed(() => Object.fromEntries(props.view.players.map((p) => [p.id, p.name])));
const lines = computed(() => props.view.log.slice(-40).reverse());
const detail = (entry: { actor?: string; target?: string; text?: string }) =>
  [entry.actor && names.value[entry.actor], entry.target && names.value[entry.target], entry.text]
    .filter(Boolean)
    .join(' · ');
</script>

<template>
  <section class="kb-panel kb-panel--log" aria-label="Game log">
    <h2 class="kb-panel__title kb-header">Log</h2>
    <ol class="kb-log">
      <li v-for="(entry, i) in lines" :key="`${entry.round}:${entry.kind}:${i}`" class="kb-log__row">
        <span class="kb-log__kind">{{ entry.kind }}</span>
        <span class="kb-log__text">{{ detail(entry) }}</span>
      </li>
    </ol>
  </section>
</template>
