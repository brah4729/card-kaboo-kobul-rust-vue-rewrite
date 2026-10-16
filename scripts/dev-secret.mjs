/**
 * Make sure `wrangler dev` has a ROOM_SECRET to sign sessions with.
 *
 * Writes a random one into `.dev.vars` (gitignored) the first time, and leaves
 * an existing file alone so local sessions survive a restart. Production sets
 * its own with `wrangler secret put ROOM_SECRET`.
 */
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { randomBytes } from 'node:crypto';

const FILE = new URL('../.dev.vars', import.meta.url);
const current = existsSync(FILE) ? readFileSync(FILE, 'utf8') : '';
if (!/^ROOM_SECRET=.+/m.test(current)) {
  const line = `ROOM_SECRET=${randomBytes(32).toString('hex')}\n`;
  writeFileSync(FILE, current + (current && !current.endsWith('\n') ? '\n' : '') + line);
  console.log('dev-secret: wrote a local ROOM_SECRET to .dev.vars');
}
