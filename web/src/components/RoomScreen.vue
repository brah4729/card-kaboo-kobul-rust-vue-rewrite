<script setup lang="ts">
/**
 * The live connection, from the moment a seat exists until it is given up.
 *
 * `useGame` is called here and nowhere else, so the lobby and the table are
 * two renders of the same socket. The deal flips `room.status` on the server
 * and this component swaps which screen it shows — the socket, the cursor and
 * the peek queue all survive that, because none of them were rebuilt.
 */
import type { StoredSession } from '@/lib/session';
import { useGame } from '@/lib/useGame';
import GameScreen from './GameScreen.vue';
import LobbyScreen from './LobbyScreen.vue';

const props = defineProps<{ session: StoredSession }>();
const emit = defineEmits<{ leave: []; lost: [] }>();

const game = useGame(props.session, () => emit('lost'));

function leave(): void {
  game.leave();
  emit('leave');
}
</script>

<template>
  <main v-if="!game.room.value" class="kb-table kb-table--gate">
    <p class="kb-label" role="status">Finding the table…</p>
  </main>
  <LobbyScreen
    v-else-if="game.room.value.status === 'lobby'"
    :room="game.room.value"
    :busy="game.busy.value"
    :refused="game.refused.value"
    @deal="game.deal"
    @leave="leave"
  />
  <GameScreen v-else :game="game" :view="game.room.value.view" @quit="leave" />
</template>
