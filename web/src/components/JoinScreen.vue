<script setup lang="ts">
/**
 * Name, then a code. A blank code means "make me a room", which is why it is
 * one form and not two buttons: hosting and joining differ by one field, and
 * asking the player to classify themselves first is a worse question.
 */
import { computed, ref } from 'vue';
import { createRoom, joinRoom } from '@/lib/api';
import { isRoomError, normalizeRoomCode } from '@/lib/protocol';
import type { StoredSession } from '@/lib/session';

defineProps<{ notice: string | null }>();
const emit = defineEmits<{ seated: [session: StoredSession] }>();

const name = ref('');
const code = ref('');
const error = ref<string | null>(null);
const busy = ref(false);

const ready = computed(() => name.value.trim().length > 0 && (code.value.length === 0 || code.value.length === 4));

// Reduced as it is typed, so a code pasted in lowercase or with a hyphen
// arrives in the one form the server recognises.
function onCode(event: Event): void {
  code.value = normalizeRoomCode((event.target as HTMLInputElement).value).slice(0, 4);
  (event.target as HTMLInputElement).value = code.value;
}

async function submit(): Promise<void> {
  if (!ready.value || busy.value) return;
  busy.value = true;
  error.value = null;
  const trimmed = name.value.trim();
  const result = code.value.length === 0 ? await createRoom(trimmed) : await joinRoom(code.value, trimmed);
  busy.value = false;
  // The server's own message is written to be safe to show: it names a room
  // state, never a card.
  if (isRoomError(result)) error.value = result.message;
  else emit('seated', result);
}
</script>

<template>
  <main class="kb-table kb-table--gate">
    <h1 class="kb-title">Kaboo</h1>
    <p class="kb-gate__body">A memory card game for two to eight people, each on their own device.</p>

    <form class="kb-form" @submit.prevent="submit">
      <label class="kb-label" for="kb-name">Your name</label>
      <input
        id="kb-name"
        v-model="name"
        class="kb-input"
        maxlength="12"
        autocomplete="nickname"
        enterkeyhint="next"
        placeholder="What should the table call you?"
      />

      <label class="kb-label" for="kb-code">Room code — leave blank to start a new game</label>
      <input
        id="kb-code"
        class="kb-input"
        :value="code"
        inputmode="text"
        autocapitalize="characters"
        autocomplete="off"
        spellcheck="false"
        enterkeyhint="go"
        placeholder="ABCD"
        @input="onCode"
      />

      <button type="submit" class="kb-btn kb-btn--primary" :disabled="!ready || busy">
        {{ busy ? 'One moment…' : code.length === 0 ? 'Start a game' : 'Join the game' }}
      </button>
    </form>

    <p v-if="error ?? notice" class="kb-error" role="alert">{{ error ?? notice }}</p>
  </main>
</template>
