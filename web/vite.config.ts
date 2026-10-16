import { fileURLToPath, URL } from 'node:url';
import { defineConfig } from 'vitest/config';
import vue from '@vitejs/plugin-vue';

/**
 * `bun run dev:web` serves the app with hot reload and proxies the API — HTTP
 * and the room WebSockets — to `wrangler dev` on :8787. In production one
 * Worker serves both, so the app only ever talks to its own origin.
 */
export default defineConfig({
  plugins: [vue()],
  resolve: { alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) } },
  server: {
    proxy: { '/api': { target: 'http://127.0.0.1:8787', ws: true } },
  },
  test: {
    environment: 'jsdom',
    include: ['tests/**/*.test.ts'],
  },
});
