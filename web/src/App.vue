<script setup lang="ts">
/**
 * THREE SCREENS: join, lobby, table. There is no fourth.
 *
 * Everybody is on their own device, so there is no "pass the phone" gate: a
 * seat is proven by the token in `localStorage`, and two players on one machine
 * are two browser windows — the same code path as two countries.
 *
 * Driven by ONE question — is there a stored seat? — and then by the server's
 * own answer about the room. Nothing here decides how far along the game is.
 */
import { ref } from 'vue';
import JoinScreen from './components/JoinScreen.vue';
import RoomScreen from './components/RoomScreen.vue';
import { clearSession, saveSession, session, type StoredSession } from './lib/session';

/** Why the last seat was dropped. Shown once, on the way back to join. */
const notice = ref<string | null>(null);

function seated(next: StoredSession): void {
  notice.value = null;
  saveSession(next);
}

function lost(): void {
  clearSession();
  notice.value = 'That room is no longer open. Start a new one, or join with a fresh code.';
}
</script>

<template>
  <JoinScreen v-if="!session" :notice="notice" @seated="seated" />
  <!-- Keyed on the token: a different seat is a different connection, and the
       next seat's cursor must start clean or it would swallow a first peek. -->
  <RoomScreen v-else :key="session.token" :session="session" @leave="clearSession" @lost="lost" />
</template>
