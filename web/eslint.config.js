import pluginVue from 'eslint-plugin-vue';
import { defineConfigWithVueTs, vueTsConfigs } from '@vue/eslint-config-typescript';

export default defineConfigWithVueTs(
  { ignores: ['dist/**', 'src/generated/**'] },
  pluginVue.configs['flat/essential'],
  vueTsConfigs.recommended,
  {
    rules: {
      // Every target is a real <button>; a div with a click handler is not a control.
      'vue/no-v-html': 'error',
      'vue/require-default-prop': 'off',
      // Card, Hand, Scoreboard: the domain's own words, not generic HTML ones.
      'vue/multi-word-component-names': 'off',
    },
  },
);
