<script setup lang="ts">
/**
 * Every button maps to exactly one engine `Action`, and which buttons exist is
 * driven by the projection's own `legalActions` — the engine's honest list of
 * what it will accept right now. The UI never re-derives a rule. AGENTS.md §9.
 */
import { computed } from 'vue';
import type { ActionType } from '@/generated/ActionType';
import type { DrawSource } from '@/generated/DrawSource';
import type { Projection } from '@/generated/Projection';
import type { Intent } from '@/lib/intent';

const props = defineProps<{
  view: Projection;
  intent: Intent;
  waitingOn: string | null;
  /**
   * A private notice is still on screen. Looking at your own two cards IS the
   * peek phase, so Ready waits for it; and no turn action is offered until a
   * card you were shown is closed. A client-side gate on purpose: the server
   * keeps no record of a notice (AGENTS.md §7), and the deadlines still run, so
   * this can never strand a table.
   */
  mustAcknowledge: boolean;
}>();

const emit = defineEmits<{
  intent: [intent: Intent];
  draw: [from: DrawSource];
  discard: [];
  kaboo: [];
  counterKaboo: [];
  ready: [];
  skipEffect: [];
  kingDecide: [swap: boolean];
  finalDecline: [];
  declinePlacement: [];
  continue: [];
}>();

const can = (action: ActionType) => props.view.legalActions.includes(action);
const isYourTurn = computed(() => props.view.turnPlayerId === props.view.youId);
const closeFirst = computed(() => props.mustAcknowledge);
const ownTap = computed(() => (props.view.tap?.playerId === props.view.youId ? props.view.tap : null));
const effect = computed(() => (props.view.phase === 'effect' ? props.view.effect : null));
const takeLabel = computed(() => {
  const top = props.view.discardTop;
  return top ? `${top.rank}${top.suit ?? ''}` : 'pile';
});

/** Why the turn buttons are inert while a card you were shown is still open. */
const CLOSE_FIRST = 'Tap Remember it to close the card you saw first.';

const LABELS: Record<string, string> = {
  '7': 'Seven — look at one of your own cards.',
  '8': 'Eight — look at one of your own cards.',
  '9': 'Nine — look at one of another player’s cards.',
  '10': 'Ten — look at one of another player’s cards.',
  J: 'Jack — swap any two cards without looking at either.',
  Q: 'Queen — swap one of yours with any other card; you may look at what you get.',
  K: 'King — look at any two cards, then decide whether to swap them.',
};
</script>

<template>
  <div class="kb-actions" role="toolbar" aria-label="Actions">
    <!-- reveal / game over -->
    <template v-if="view.phase === 'reveal' || view.phase === 'gameOver'">
      <button v-if="can('CONTINUE')" type="button" class="kb-btn kb-btn--primary" @click="emit('continue')">
        {{ view.gameResult ? 'Play again' : 'Next round' }}
      </button>
      <p v-else class="kb-actions__hint">Game over.</p>
    </template>

    <!-- the opening peek: say why Ready is inert, rather than fading it out -->
    <template v-else-if="view.phase === 'peek'">
      <p class="kb-actions__hint">
        {{ mustAcknowledge ? 'Close what you saw first — tap Remember it above.' : 'Tap cards 3 and 4 to look — once each.' }}
      </p>
      <button v-if="can('READY')" type="button" class="kb-btn kb-btn--primary" :disabled="mustAcknowledge" @click="emit('ready')">
        Ready
      </button>
    </template>

    <!-- a tap I claimed: a race, so it never waits on a dialog -->
    <template v-else-if="ownTap">
      <p class="kb-actions__hint">
        {{ ownTap.awaitingPlacement ? 'Drop one of your cards into the empty spot.' : 'Pick the card that matches the top of the pile.' }}
      </p>
      <button v-if="ownTap.awaitingPlacement" type="button" class="kb-btn" @click="emit('declinePlacement')">
        Leave it empty
      </button>
    </template>

    <!-- deck out -->
    <template v-else-if="view.phase === 'finalSwap' && can('FINAL_SWAP')">
      <p class="kb-actions__hint">{{ closeFirst ? CLOSE_FIRST : 'You drew the last card — swap it in, or decline.' }}</p>
      <button
        type="button"
        class="kb-btn kb-btn--primary"
        :disabled="closeFirst"
        :aria-pressed="intent === 'finalSwap'"
        @click="emit('intent', 'finalSwap')"
      >
        Swap it in
      </button>
      <button type="button" class="kb-btn" :disabled="closeFirst" @click="emit('finalDecline')">Decline</button>
    </template>

    <!-- an effect -->
    <template v-else-if="effect">
      <p v-if="effect.by !== view.youId" class="kb-actions__hint">An effect is being played.</p>
      <!-- A King that has looked can only be answered with the decision: the
           engine refuses a skip now, and so do we. Exempt from closeFirst — the
           two cards are what the decision is about. -->
      <template v-else-if="effect.awaitingKingDecision">
        <p class="kb-actions__hint">Swap those two cards?</p>
        <button type="button" class="kb-btn kb-btn--primary" @click="emit('kingDecide', true)">Swap them</button>
        <button type="button" class="kb-btn" @click="emit('kingDecide', false)">Leave them</button>
      </template>
      <template v-else>
        <p class="kb-actions__hint">{{ closeFirst ? CLOSE_FIRST : (LABELS[effect.rank] ?? 'Use the effect') }}</p>
        <button
          v-if="can('EFFECT_PEEK_OWN')"
          type="button"
          class="kb-btn kb-btn--primary"
          :disabled="closeFirst"
          :aria-pressed="intent === 'effectOwn'"
          @click="emit('intent', 'effectOwn')"
        >
          Look at my card
        </button>
        <button
          v-if="can('EFFECT_PEEK_OTHER')"
          type="button"
          class="kb-btn kb-btn--primary"
          :disabled="closeFirst"
          :aria-pressed="intent === 'effectOther'"
          @click="emit('intent', 'effectOther')"
        >
          Look at their card
        </button>
        <button
          v-if="can('EFFECT_BLIND_SWAP')"
          type="button"
          class="kb-btn kb-btn--primary"
          :disabled="closeFirst"
          :aria-pressed="intent === 'blindFirst' || intent === 'blindSecond'"
          @click="emit('intent', 'blindFirst')"
        >
          Blind-swap two cards
        </button>
        <button
          v-if="can('EFFECT_QUEEN_SWAP')"
          type="button"
          class="kb-btn kb-btn--primary"
          :disabled="closeFirst"
          :aria-pressed="intent === 'queenOwn' || intent === 'queenTarget'"
          @click="emit('intent', 'queenOwn')"
        >
          Swap one of mine
        </button>
        <button
          v-if="can('EFFECT_KING_PEEK')"
          type="button"
          class="kb-btn kb-btn--primary"
          :disabled="closeFirst"
          :aria-pressed="intent === 'kingFirst' || intent === 'kingSecond'"
          @click="emit('intent', 'kingFirst')"
        >
          Look at two cards
        </button>
        <button v-if="can('EFFECT_SKIP')" type="button" class="kb-btn" :disabled="closeFirst" @click="emit('skipEffect')">
          Skip
        </button>
      </template>
    </template>

    <!-- choosing which card to swap the drawn one into -->
    <template v-else-if="intent === 'swap'">
      <p class="kb-actions__hint">Tap one of your cards to replace it with the one you drew.</p>
      <button type="button" class="kb-btn" @click="emit('intent', 'none')">Cancel</button>
    </template>

    <!-- turn start: nothing drawn yet -->
    <template v-else-if="isYourTurn && can('DRAW')">
      <p v-if="closeFirst" class="kb-actions__hint">{{ CLOSE_FIRST }}</p>
      <button type="button" class="kb-btn kb-btn--primary" :disabled="closeFirst" @click="emit('draw', 'deck')">Draw</button>
      <button
        type="button"
        class="kb-btn"
        :disabled="closeFirst || view.discardTop === null"
        @click="emit('draw', 'discard')"
      >
        Take the {{ takeLabel }}
      </button>
      <button type="button" class="kb-btn kb-btn--kaboo" :disabled="closeFirst || !can('CALL_KABOO')" @click="emit('kaboo')">
        KABOO
      </button>
      <button v-if="can('COUNTER_KABOO')" type="button" class="kb-btn" :disabled="closeFirst" @click="emit('counterKaboo')">
        Counter-KABOO
      </button>
    </template>

    <!-- drawn: swap or discard -->
    <template v-else-if="can('SWAP') || can('DISCARD')">
      <p v-if="closeFirst" class="kb-actions__hint">{{ CLOSE_FIRST }}</p>
      <button
        type="button"
        class="kb-btn kb-btn--primary"
        :disabled="closeFirst || !can('SWAP')"
        @click="emit('intent', 'swap')"
      >
        Swap
      </button>
      <button type="button" class="kb-btn" :disabled="closeFirst || !can('DISCARD')" @click="emit('discard')">Discard</button>
    </template>

    <p v-else class="kb-actions__hint">{{ waitingOn ? `Waiting for ${waitingOn}.` : 'Nothing to do.' }}</p>
  </div>
</template>
