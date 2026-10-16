<script setup lang="ts">
/**
 * The game surface — mobile-first, then a three-column desktop layout.
 *
 * Everything it renders comes from the projection the SERVER sent for this one
 * seat. There is no `GameState` in the browser at all: a card this player may
 * not see was never transmitted, so no amount of clicking can reveal one.
 */
import { computed, nextTick, ref, watch } from 'vue';
import type { CardRef } from '@/generated/CardRef';
import type { Projection } from '@/generated/Projection';
import { OWN_HAND_DECISIONS, activeIntent, selectableFor, type Intent } from '@/lib/intent';
import type { Game } from '@/lib/useGame';
import ActionBar from './ActionBar.vue';
import Card from './Card.vue';
import CardFlights from './CardFlights.vue';
import GameLog from './GameLog.vue';
import Hand from './Hand.vue';
import PeekTray from './PeekTray.vue';
import Scoreboard from './Scoreboard.vue';
import StatusPill from './StatusPill.vue';

const props = defineProps<{ game: Game; view: Projection }>();
const emit = defineEmits<{ quit: [] }>();

const chosen = ref<Intent>('none');
const picks = ref<CardRef[]>([]);

const you = computed(() => props.view.youId);
const intent = computed(() => activeIntent(props.view, chosen.value));
const tapping = computed(() => intent.value === 'tapChoose' || intent.value === 'tapPlace');

/**
 * A card the player was just shown has to be closed before they act again:
 * the notice is the only record of what they saw. Not during the opening peek
 * (notices pile up by design, Ready has its own gate) and never during a tap,
 * which is a race against a deadline.
 */
const blocked = computed(() => props.game.peeks.value.length > 0 && props.view.phase !== 'peek' && !tapping.value);

/** Which slots are tappable right now. Nothing is, while a notice is open. */
const selectable = computed(() => (blocked.value ? new Map<string, number[]>() : selectableFor(props.view, intent.value)));

const me = computed(() => props.view.players.find((p) => p.id === you.value));
const opponents = computed(() => props.view.players.filter((p) => p.id !== you.value));
const turnPlayer = computed(() => props.view.players.find((p) => p.id === props.view.turnPlayerId));

/** The player's own pickable cards; in the peek phase, the peekable ones. */
const ownSelectable = computed(
  () => selectable.value.get(you.value) ?? (props.view.phase === 'peek' ? props.view.peekableSlots : []),
);

/**
 * The cards to draw attention to: this hand, and only when the question is
 * about this hand. For a tap or a blind swap the whole table is in play, and
 * lighting only the player's own cards would mislead them into a penalty card.
 */
const liveSlots = computed(() => {
  const own = intent.value === 'none' ? props.view.phase === 'peek' : OWN_HAND_DECISIONS.has(intent.value);
  return own ? ownSelectable.value : [];
});

/** Which countdown this phase is on. Only one is ever live. */
const deadline = computed(() => {
  const d = props.view.deadlines;
  if (props.view.phase === 'peek') return d.peek;
  if (props.view.phase === 'finalSwap') return d.finalSwap;
  if (props.view.tap) return d.tap;
  return d.turn;
});

function setIntent(next: Intent): void {
  picks.value = [];
  chosen.value = next;
}

function onSlot(target: CardRef): void {
  if (blocked.value) return;
  const send = props.game.send;
  const playerId = you.value;
  const first = picks.value[0];
  // Read BEFORE resetting: `intent` is computed from `chosen`, so resetting
  // first would turn every pick into a pick made with no intent at all.
  const current = intent.value;
  chosen.value = 'none';

  switch (current) {
    case 'none':
      if (props.view.phase === 'peek' && target.playerId === playerId) send({ type: 'PEEK', playerId, slot: target.slot });
      break;
    case 'swap':
      send({ type: 'SWAP', playerId, slot: target.slot });
      break;
    case 'finalSwap':
      send({ type: 'FINAL_SWAP', playerId, slot: target.slot });
      break;
    case 'tapChoose':
      send({ type: 'TAP_CHOOSE', playerId, target });
      break;
    case 'tapPlace':
      send({ type: 'TAP_PLACE', playerId, slot: target.slot });
      break;
    case 'effectOwn':
      send({ type: 'EFFECT_PEEK_OWN', playerId, slot: target.slot });
      break;
    case 'effectOther':
      send({ type: 'EFFECT_PEEK_OTHER', playerId, targetPlayerId: target.playerId, slot: target.slot });
      break;
    case 'queenOwn':
      picks.value = [target];
      chosen.value = 'queenTarget';
      return;
    case 'queenTarget':
      if (first) send({ type: 'EFFECT_QUEEN_SWAP', playerId, slot: first.slot, target });
      break;
    case 'blindFirst':
      picks.value = [target];
      chosen.value = 'blindSecond';
      return;
    case 'blindSecond':
      if (first) send({ type: 'EFFECT_BLIND_SWAP', playerId, a: first, b: target });
      break;
    case 'kingFirst':
      picks.value = [target];
      chosen.value = 'kingSecond';
      return;
    case 'kingSecond':
      if (first) send({ type: 'EFFECT_KING_PEEK', playerId, a: first, b: target });
      break;
  }
  picks.value = [];
}

/** A picked first card is only meaningful while its two-step choice is live. */
watch(
  () => props.view.phase,
  () => {
    picks.value = [];
    chosen.value = 'none';
  },
);

/**
 * Bring the player's own cards into view when it becomes their move — and when
 * the peek tray opens, because the tray shortens the board and the row it pushes
 * off the bottom is the row the player was told to look at. `block: 'nearest'`
 * is a no-op on every screen that already fits; instant, because a control the
 * player cannot see is worse than a jump they did not ask for.
 */
const ownCards = ref<HTMLElement | null>(null);
const acting = computed(
  () =>
    props.view.phase === 'peek' ||
    props.view.turnPlayerId === you.value ||
    props.view.tap?.playerId === you.value ||
    props.view.drawn?.isYours === true,
);
const ownCardCount = computed(() => me.value?.hand.filter((s) => s.occupied).length ?? 0);
const trayOpen = computed(() => props.game.peeks.value.length > 0);
watch(
  [acting, ownCardCount, trayOpen],
  async () => {
    if (!acting.value) return;
    await nextTick();
    ownCards.value?.scrollIntoView({ block: 'nearest' });
  },
  { immediate: true },
);

const tap = () => props.game.send({ type: 'TAP', playerId: you.value, seenSeq: props.view.discardSeq });
</script>

<template>
  <main class="kb-table kb-table--game">
    <CardFlights :view="view" />
    <header class="kb-topbar">
      <span class="kb-chip kb-chip--dark">Room {{ view.roomCode }}</span>
      <span class="kb-label">Round {{ view.round }}/{{ view.rounds }}</span>
      <!-- Honest about the transport: without the socket, moves appear more slowly. -->
      <span
        class="kb-chip"
        :title="game.live.value ? 'Live updates are on.' : 'Reconnecting — the table refreshes every few seconds meanwhile.'"
      >
        {{ game.live.value ? 'live' : 'reconnecting' }}
      </span>
      <button type="button" class="kb-btn kb-btn--small" @click="emit('quit')">Leave</button>
    </header>

    <StatusPill :view="view" :deadline="deadline" :now="game.now.value" :can-tap="view.canTap && view.tap === null && view.legalActions.includes('TAP')" @tap="tap" />

    <div class="kb-board">
      <Scoreboard :view="view" />

      <section class="kb-centre" aria-label="Table">
        <div class="kb-opponents">
          <article v-for="p in opponents" :key="p.id" class="kb-seat">
            <h3 class="kb-label">{{ p.name }} {{ p.handCount === 0 ? '· empty' : `· ${p.handCount} cards` }}</h3>
            <Hand
              :slots="p.hand"
              :label="`${p.name}'s cards`"
              :owner-id="p.id"
              compact
              :selectable-slots="selectable.get(p.id) ?? []"
              :selected-slot="picks[0]?.playerId === p.id ? picks[0].slot : null"
              @select="(slot) => onSlot({ playerId: p.id, slot })"
            />
          </article>
        </div>

        <div class="kb-piles">
          <div class="kb-pile">
            <div class="kb-pile__head">
              <span class="kb-label">Draw</span>
              <span class="kb-chip kb-chip--count" aria-hidden="true">{{ view.drawPileCount }}</span>
            </div>
            <div class="kb-pile__stack" data-flight="draw">
              <Card v-if="view.drawPileCount > 0" face-down :label="`draw pile, ${view.drawPileCount} cards left`" />
              <Card v-else empty />
            </div>
          </div>

          <div class="kb-pile kb-pile--discard">
            <div class="kb-pile__head">
              <span class="kb-label">Discard</span>
              <span class="kb-chip kb-chip--count" aria-hidden="true">{{ view.discardCount }}</span>
            </div>
            <div class="kb-pile__stack" data-flight="discard">
              <Card v-if="view.discardTop" :card="view.discardTop" label="top of the discard pile" />
              <Card v-else empty />
            </div>
          </div>

          <div class="kb-pile">
            <div class="kb-pile__head">
              <span class="kb-label">Drawn card</span>
            </div>
            <div class="kb-pile__stack" data-flight="drawn">
              <!-- The card you drew is the live one while you hold it. -->
              <Card v-if="view.drawn?.isYours && view.drawn.card" :card="view.drawn.card" label="the card you drew" live />
              <Card v-else-if="view.drawn" face-down label="a card drawn by another player" />
              <Card v-else-if="view.heldCard?.isYours && view.heldCard.card" :card="view.heldCard.card" label="the last card" />
              <Card v-else empty />
            </div>
          </div>
        </div>

        <section ref="ownCards" class="kb-you" aria-label="Your cards">
          <h3 class="kb-label">{{ me?.name }} {{ view.phase === 'peek' ? '· peek at cards 3 and 4' : '' }}</h3>
          <Hand
            :slots="me?.hand ?? []"
            label="your cards"
            :owner-id="you"
            :selectable-slots="ownSelectable"
            :live-slots="liveSlots"
            :selected-slot="picks[0]?.playerId === you ? picks[0].slot : null"
            @select="(slot) => onSlot({ playerId: you, slot })"
          />
        </section>
      </section>

      <GameLog :view="view" />
    </div>

    <p class="kb-footnote">Peek at cards 3 and 4 at the start. Card positions never move — that is the memory game.</p>

    <!-- Everything you can do, and anything the server refused, in one bar
         pinned to the bottom — where the thumb already is on a phone. -->
    <div class="kb-dock">
      <PeekTray :peeks="game.peeks.value" @dismiss="game.dismissPeeks" />
      <p v-if="game.refused.value" class="kb-error" role="alert">That did not work.</p>
      <ActionBar
        :view="view"
        :intent="intent"
        :must-acknowledge="game.peeks.value.length > 0"
        :waiting-on="turnPlayer && view.turnPlayerId !== you ? turnPlayer.name : null"
        @intent="setIntent"
        @draw="(from) => game.send({ type: 'DRAW', playerId: you, from })"
        @discard="game.send({ type: 'DISCARD', playerId: you })"
        @kaboo="game.send({ type: 'CALL_KABOO', playerId: you })"
        @counter-kaboo="game.send({ type: 'COUNTER_KABOO', playerId: you })"
        @ready="game.send({ type: 'READY', playerId: you })"
        @skip-effect="game.send({ type: 'EFFECT_SKIP', playerId: you })"
        @king-decide="(swap) => game.send({ type: 'EFFECT_KING_DECIDE', playerId: you, swap })"
        @final-decline="game.send({ type: 'FINAL_DECLINE', playerId: you })"
        @decline-placement="game.send({ type: 'TAP_DECLINE_PLACE', playerId: you })"
        @continue="game.send({ type: 'CONTINUE', playerId: you })"
      />
    </div>
  </main>
</template>
