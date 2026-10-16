// Drives two real browser contexts (two phones) through a game of Kaboo via
// the UI only: create, join, deal, the opening peek and its Ready gate, turns,
// every effect sub-flow (7/8, 9/10, J, Q, K), Skip, and an off-turn tap.
// Run through the Playwright MCP `browser_run_code` with this file.
async (page) => {
  const BASE = 'http://127.0.0.1:8787'; // edit to drive a deployed URL
  const browser = page.context().browser();
  const log = [];
  const fail = [];
  const check = (cond, label) => {
    log.push(`${cond ? 'PASS' : 'FAIL'}  ${label}`);
    if (!cond) fail.push(label);
  };

  const mk = async (name) => {
    const ctx = await browser.newContext({ viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true });
    const p = await ctx.newPage();
    const errors = [];
    p.on('pageerror', (e) => errors.push(String(e)));
    p.on('console', (m) => m.type() === 'error' && errors.push(m.text()));
    return { name, ctx, p, errors };
  };
  const ana = await mk('Ana');
  const ben = await mk('Ben');
  const players = [ana, ben];
  const other = (pl) => (pl === ana ? ben : ana);
  const settle = (ms = 350) => page.waitForTimeout(ms);
  const status = async (pl) => (await pl.p.locator('.kb-status .kb-chip').textContent()).trim();
  const tray = (pl) => pl.p.locator('.kb-peeks');
  const trayLabels = async (pl) => (await tray(pl).count()) ? await tray(pl).locator('figcaption').allTextContents() : [];
  const closeTray = async (pl) => {
    if (await tray(pl).count()) {
      await pl.p.getByRole('button', { name: 'Remember it' }).click();
      await settle(150);
    }
  };
  const own = (pl, n) => {
    at = `${pl.name}: own slot ${n}`;
    return pl.p.getByRole('button', { name: new RegExp(`^your cards slot ${n}:`) });
  };
  const theirs = (pl, n) => {
    at = `${pl.name}: their slot ${n}`;
    return pl.p.getByRole('button', { name: new RegExp(`^${other(pl).name}'s cards slot ${n}:`) });
  };
  let at = 'setup';
  const button = (pl, name) => {
    at = `${pl.name}: button ${name}`;
    return pl.p.getByRole('button', { name, exact: true });
  };

  try {
    // --- create, join, deal --------------------------------------------------
    await ana.p.goto(BASE);
    await ana.p.getByLabel('Your name').fill('Ana');
    await button(ana, 'Start a game').click();
    const code = (await ana.p.locator('.kb-chip--room').textContent({ timeout: 8000 })).trim();
    check(/^[A-Z2-9]{4}$/.test(code), `Ana created room ${code}`);

    await ben.p.goto(BASE);
    await ben.p.getByLabel('Your name').fill('Ben');
    await ben.p.getByLabel(/Room code/).fill(` ${code.toLowerCase()} `);
    check((await ben.p.getByLabel(/Room code/).inputValue()) === code, 'the code input normalizes what was typed');
    await button(ben, 'Join the game').click();
    await ana.p.getByText('2 of 8 here').first().waitFor({ timeout: 8000 });
    check(true, "Ben's arrival was pushed to Ana's lobby");
    check(!(await ben.p.getByRole('button', { name: 'Deal the cards' }).count()), 'only the host is offered the deal');

    await button(ana, 'Deal the cards').click();
    await ben.p.getByText('peek at cards 3 and 4').first().waitFor({ timeout: 8000 });
    check(true, "the deal was pushed to Ben's lobby, and the socket survived it");

    // --- the opening peek ----------------------------------------------------
    for (const pl of players) {
      check((await pl.p.locator('.kb-you img.kb-card__face').count()) === 0, `${pl.name}: no face is shown in their own hand`);
      check((await pl.p.locator('.kb-opponents img.kb-card__face').count()) === 0, `${pl.name}: no opponent face is shown`);
      await own(pl, 3).click();
      await settle();
      check((await trayLabels(pl)).includes('slot 3'), `${pl.name}: peeking card 3 opens the tray, labelled "slot 3"`);
      check(await button(pl, 'Ready').isDisabled(), `${pl.name}: Ready waits while the tray is open`);
      await own(pl, 4).click();
      await settle();
      check((await trayLabels(pl)).length === 2, `${pl.name}: the second peek joins the first`);
      check(!(await own(pl, 3).count()), `${pl.name}: card 3 is no longer pickable once seen`);
      await closeTray(pl);
      check(await button(pl, 'Ready').isEnabled(), `${pl.name}: Ready is offered once the tray is closed`);
      await button(pl, 'Ready').click();
      await settle();
    }
    await ben.p.locator('.kb-status').getByText(/^Your turn/).waitFor({ timeout: 8000 });
    check(true, 'both ready: the turn goes to the seat after the dealer (Ben)');

    // --- play until every effect has been driven -------------------------------
    const want = new Map([
      ['own', /^(seven|eight) of/],
      ['other', /^(nine|ten) of/],
      ['jack', /^Jack of/],
      ['queen', /^Queen of/],
      ['king', /^King of/],
    ]);
    const done = new Set();
    let skipped = false;
    let tapped = false;

    for (let turn = 0; turn < 90 && done.size < want.size; turn += 1) {
      const pl = (await status(ana)).startsWith('Your turn') ? ana : (await status(ben)).startsWith('Your turn') ? ben : null;
      if (!pl) {
        const s = await status(ana);
        if (/Reveal|Game over/.test(s)) {
          log.push(`round ended at turn ${turn}`);
          const next = ana.p.getByRole('button', { name: /Next round|Play again/ });
          if (await next.count()) await next.click();
          await settle(800);
          for (const p2 of players) {
            if (await own(p2, 3).count()) await own(p2, 3).click();
            await settle(200);
            if (await own(p2, 4).count()) await own(p2, 4).click();
            await settle(200);
            await closeTray(p2);
            if (await button(p2, 'Ready').count()) await button(p2, 'Ready').click();
            await settle(200);
          }
          continue;
        }
        await settle(500);
        continue;
      }
      const opp = other(pl);

      // An off-turn tap, once, through the opponent's own "Tap the pile" button.
      if (!tapped && (await button(opp, 'Tap the pile').count())) {
        await button(opp, 'Tap the pile').click();
        await settle();
        const hint = await opp.p.locator('.kb-actions__hint').first().textContent();
        const pickable = await opp.p.locator('.kb-hand button.kb-card').count();
        check(/matches the top of the pile/.test(hint) && pickable > 0, `off-turn tap: ${opp.name} is asked to pick, with ${pickable} cards pickable`);
        check((await status(pl)).includes('tapped'), `${pl.name} is told ${opp.name} tapped`);
        await opp.p.getByRole('button', { name: new RegExp(`^your cards slot 1:`) }).click();
        await settle(500);
        if (await button(opp, 'Leave it empty').count()) await button(opp, 'Leave it empty').click();
        await settle(400);
        check(!(await opp.p.locator('.kb-actions__hint').first().textContent()).includes('matches the top'), 'the tap resolved (right or wrong), and the table moved on');
        tapped = true;
        continue;
      }

      await closeTray(pl);
      await button(pl, 'Draw').click();
      await settle();
      const drawn = await pl.p.locator('[data-flight="drawn"] .kb-card').getAttribute('aria-label');
      const face = (drawn ?? '').replace(/^the card you drew: /, '');
      const oppDrawn = await opp.p.locator('[data-flight="drawn"] .kb-card').getAttribute('aria-label');
      if (turn < 3) check(/face-down/.test(oppDrawn ?? ''), `${opp.name} sees only a face-down drawn card (${oppDrawn})`);
      await closeTray(pl); // the draw's own notice

      log.push(`turn ${turn}: ${pl.name} drew ${face}`);
      const kind = [...want].find(([k, re]) => !done.has(k) && re.test(face))?.[0];
      if (!kind) {
        if (!skipped && /^(seven|eight|nine|ten|Jack|Queen|King) of/.test(face)) {
          await button(pl, 'Discard').click();
          await settle();
          await button(pl, 'Skip').click();
          await settle();
          check(!(await pl.p.locator('.kb-actions__hint').first().textContent()).includes('—'), `Skip ends ${face}'s effect`);
          skipped = true;
        } else {
          await button(pl, 'Swap').click();
          await own(pl, 1).click();
          await settle();
        }
        continue;
      }

      await button(pl, 'Discard').click();
      await settle();
      if (kind === 'own') {
        await button(pl, 'Look at my card').click();
        await own(pl, 1).click();
        await settle();
        check((await trayLabels(pl)).includes('slot 1'), `${face}: look at my card → tray "slot 1"`);
        check(!(await tray(opp).count()), `${face}: ${opp.name} is shown nothing`);
      } else if (kind === 'other') {
        await button(pl, 'Look at their card').click();
        check(!(await own(pl, 1).count()), `${face}: my own cards are not pickable for a look at theirs`);
        await theirs(pl, 1).click();
        await settle();
        check((await trayLabels(pl)).some((l) => l === `${opp.name}'s card · slot 1`), `${face}: tray names whose card was seen`);
      } else if (kind === 'jack') {
        await button(pl, 'Blind-swap two cards').click();
        await own(pl, 2).click();
        await theirs(pl, 2).click();
        await settle();
        check(!(await tray(pl).count()), `${face}: a blind swap shows nobody anything`);
      } else if (kind === 'queen') {
        await button(pl, 'Swap one of mine').click();
        await own(pl, 1).click();
        check(!(await own(pl, 2).count()), `${face}: the Queen's target is never one of my own cards`);
        await theirs(pl, 1).click();
        await settle();
        check((await trayLabels(pl)).includes('the card you were given'), `${face}: tray shows the card received`);
      } else if (kind === 'king') {
        await button(pl, 'Look at two cards').click();
        await own(pl, 1).click();
        await theirs(pl, 1).click();
        await settle();
        check((await trayLabels(pl)).length === 2, `${face}: the King shows both cards`);
        check(await button(pl, 'Swap them').isEnabled(), `${face}: the swap decision is offered with the tray still open`);
        await button(pl, 'Swap them').click();
        await settle();
      }
      done.add(kind);
      await closeTray(pl);
      await settle(300);
      check(!(await status(pl)).startsWith('Your turn'), `${face}: the turn passed after the effect`);
    }

    for (const [k] of want) check(done.has(k), `effect sub-flow driven: ${k}`);
    check(skipped, 'an effect was skipped');
    check(tapped, 'an off-turn tap was driven');
    for (const pl of players) check(pl.errors.length === 0, `${pl.name}: no console errors (${pl.errors.slice(0, 3).join(' | ')})`);
  } catch (error) {
    log.push(`ERROR at [${at}] ${String(error).split('\n')[0]}`);
    fail.push('exception');
    log.push((await ana.p.locator('main').textContent().catch(() => '')).slice(0, 600));
    log.push((await ben.p.locator('main').textContent().catch(() => '')).slice(0, 600));
  } finally {
    await ana.ctx.close();
    await ben.ctx.close();
  }
  return `${log.join('\n')}\n\n${fail.length === 0 ? 'ALL GOOD' : `${fail.length} FAILED`}`;
}
