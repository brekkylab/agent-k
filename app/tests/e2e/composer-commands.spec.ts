/**
 * E2E tests for composer commands: '#' shared-file attach and '@' mention
 * (team messages).
 *
 * Requires a running dev stack (app/scripts/run_cowork_demo.sh). Demo
 * credentials: olive / cowork-demo → project "klientco-q2" (shared_chat
 * sessions seeded, members olive/milo/owen).
 *
 * Korean IME composition behavior is covered by unit tests
 * (CommandSuggestionPopup.test.tsx) — Playwright IME automation is unreliable.
 *
 * Note: the team-message test persists a message into the demo DB; the demo
 * reset script restores it.
 */
import { test, expect, type Page } from '@playwright/test';

const BASE_URL = process.env.PLAYWRIGHT_BASE_URL ?? 'http://127.0.0.1:4110';
const DEMO_USERNAME = 'olive';
const DEMO_PASSWORD = 'cowork-demo';
const PROJECT_SLUG = 'klientco-q2';

async function loginAndOpenSession(page: Page): Promise<void> {
  await page.goto(`${BASE_URL}/login`);
  await page.locator('input[name="username"]').fill(DEMO_USERNAME);
  await page.locator('input[name="password"]').fill(DEMO_PASSWORD);
  await page.locator('button[type="submit"]').click();
  await page.waitForURL(`${BASE_URL}/projects**`);
  await page.goto(`${BASE_URL}/projects/${PROJECT_SLUG}`);
  // The sidebar lists the project's sessions; the home pane has no cards.
  await page.locator('.cw-session-row').first().click();
  await page.waitForURL('**/sessions/**');
  await expect(page.locator('.cw-composer textarea')).toBeEnabled();
}

function composer(page: Page) {
  return page.locator('.cw-composer textarea');
}

test('"#" opens the shared-file popup, filters, and attaches a chip', async ({ page }) => {
  await loginAndOpenSession(page);

  await composer(page).click();
  await composer(page).pressSequentially('#');
  const listbox = page.locator('.cw-cmd-popup[role="listbox"]');
  await expect(listbox).toBeVisible();
  await expect(listbox.locator('[role="option"]').first()).toBeVisible();

  const firstLabel = await listbox.locator('[role="option"] .cw-cmd-label').first().textContent();
  await composer(page).press('Enter');

  // Token removed; the picked file shows up as an attachment chip.
  await expect(composer(page)).toHaveValue('');
  await expect(listbox).toBeHidden();
  await expect(page.locator('.cw-attach-tray')).toContainText(firstLabel ?? '');
});

test('mid-word "#" does not open the popup', async ({ page }) => {
  await loginAndOpenSession(page);
  await composer(page).click();
  await composer(page).pressSequentially('see foo#bar');
  await expect(page.locator('.cw-cmd-popup')).toHaveCount(0);
});

test('"@" mention flips to team mode and sends a dashed team bubble without an agent run', async ({
  page,
}) => {
  await loginAndOpenSession(page);

  await composer(page).click();
  await composer(page).pressSequentially('@');
  const listbox = page.locator('.cw-cmd-popup[role="listbox"]');
  await expect(listbox).toBeVisible();
  await composer(page).press('Enter');

  // Mention inserted, quiet team-mode indicators on.
  await expect(composer(page)).toHaveValue(/@.+ /);
  await expect(page.locator('.cw-composer-team-hint')).toBeVisible();
  await expect(page.locator('.cw-composer-box')).toHaveAttribute('data-team-mode', 'true');
  // The mention token is tinted in the backdrop highlight overlay.
  await expect(page.locator('.cw-composer-highlight .cw-mention-hl')).toHaveCount(1);

  await composer(page).pressSequentially('please take a look (e2e)');
  await composer(page).press('Enter');

  // Team bubble renders with the not-for-agent mark; no agent streaming starts
  // (the composer stays enabled — an agent run would disable it).
  await expect(page.locator('.cw-message-bubble-team').last()).toBeVisible();
  await expect(page.locator('.cw-team-mark').last()).toBeVisible();
  // The "@username" token in the posted body is tinted.
  await expect(
    page.locator('.cw-message-bubble-team').last().locator('.cw-mention-token'),
  ).toHaveCount(1);
  await expect(composer(page)).toBeEnabled();

  // Hand-deleting the mention text drops team mode again.
  await composer(page).pressSequentially('@');
  await expect(listbox).toBeVisible();
  await composer(page).press('Enter');
  await expect(page.locator('.cw-composer-team-hint')).toBeVisible();
  await composer(page).fill('plain text without mentions');
  await expect(page.locator('.cw-composer-team-hint')).toHaveCount(0);
});

test('"@" everyone item flips to team mode and posts a team bubble without an agent run', async ({
  page,
}) => {
  await loginAndOpenSession(page);

  await composer(page).click();
  await composer(page).pressSequentially('@');
  const listbox = page.locator('.cw-cmd-popup[role="listbox"]');
  await expect(listbox).toBeVisible();
  // The synthetic "everyone" item is the first option; Enter selects it.
  await composer(page).press('Enter');

  // Inserts the locale literal (@all / @모두); team mode + highlight on.
  await expect(composer(page)).toHaveValue(/@(all|모두) /);
  await expect(page.locator('.cw-composer-team-hint')).toBeVisible();
  await expect(page.locator('.cw-composer-box')).toHaveAttribute('data-team-mode', 'true');
  await expect(page.locator('.cw-composer-highlight .cw-mention-hl')).toHaveCount(1);

  await composer(page).pressSequentially('everyone heads up (e2e)');
  await composer(page).press('Enter');

  // Team bubble with the not-for-agent mark; the everyone token is tinted; the
  // composer stays enabled (an agent run would disable it).
  await expect(page.locator('.cw-message-bubble-team').last()).toBeVisible();
  await expect(page.locator('.cw-team-mark').last()).toBeVisible();
  await expect(
    page.locator('.cw-message-bubble-team').last().locator('.cw-mention-token'),
  ).toHaveCount(1);
  await expect(composer(page)).toBeEnabled();
});

test('hand-typed "@username" (no popup pick) is still recognised as a mention', async ({ page }) => {
  await loginAndOpenSession(page);
  await composer(page).click();
  // Read a real member handle from the popup, then type it by hand + a trailing
  // space (which closes the popup without selecting), so nothing is "picked".
  await composer(page).pressSequentially('@');
  const sub = page.locator('.cw-cmd-popup [role="option"] .cw-cmd-sub').first();
  await expect(sub).toBeVisible();
  const handle = (await sub.textContent())?.trim() ?? ''; // e.g. "@milo"
  await composer(page).fill('');
  await composer(page).pressSequentially(`${handle} 직접 타이핑 `);
  // No popup selection happened, yet team mode is on (matched against members).
  await expect(page.locator('.cw-composer-team-hint')).toBeVisible();
  await expect(page.locator('.cw-composer-highlight .cw-mention-hl')).toHaveCount(1);
  await composer(page).press('Enter');
  await expect(page.locator('.cw-message-bubble-team').last()).toBeVisible();
  await expect(
    page.locator('.cw-message-bubble-team').last().locator('.cw-mention-token'),
  ).toHaveCount(1);
});

test('a mentioned user gets a live @ badge and sees the team bubble on entry', async ({
  browser,
}) => {
  // Two real clients: olive posts, milo watches his sidebar.
  const oliveCtx = await browser.newContext();
  const miloCtx = await browser.newContext();
  const olive = await oliveCtx.newPage();
  const milo = await miloCtx.newPage();

  await loginAndOpenSession(olive);
  const sessionUrl = olive.url();

  await milo.goto(`${BASE_URL}/login`);
  await milo.locator('input[name="username"]').fill('milo');
  await milo.locator('input[name="password"]').fill(DEMO_PASSWORD);
  await milo.locator('button[type="submit"]').click();
  await milo.waitForURL(`${BASE_URL}/projects**`);
  // Sit on the project home — the sidebar subscribes to the project's sessions.
  await milo.goto(`${BASE_URL}/projects/${PROJECT_SLUG}`);
  await expect(milo.locator('.cw-session-row').first()).toBeVisible();

  // Olive mentions milo via the '@' popup.
  await composer(olive).click();
  await composer(olive).pressSequentially('@mi');
  await expect(olive.locator('.cw-cmd-popup [role="option"]')).toHaveCount(1);
  await composer(olive).press('Enter');
  await composer(olive).pressSequentially('badge check (e2e)');
  await composer(olive).press('Enter');
  await expect(olive.locator('.cw-message-bubble-team').last()).toBeVisible();

  // Milo's sidebar shows the mention dot — distinct from the unread badge.
  // Reload to read the persisted state (the live WS path can coalesce rapid
  // events; what matters is milo can find the session he was mentioned in).
  await milo.reload();
  await expect(milo.locator('.cw-session-row').first()).toBeVisible();
  const mentionDot = milo.locator('.cw-mention-dot');
  await expect(mentionDot.first()).toBeVisible({ timeout: 10_000 });

  // The mention also surfaces at the projects-list level (sidebar PROJECTS nav
  // + project card) so it's findable right after login, before opening one.
  await milo.goto(`${BASE_URL}/projects`);
  await expect(milo.locator('.cw-project-nav-row .cw-mention-dot').first()).toBeVisible({ timeout: 10_000 });
  await expect(milo.locator('.cw-project-card .cw-mention-dot').first()).toBeVisible();

  // Baseline the session-row mention dots before reading. (The demo seed may
  // pre-plant other mentions for milo, so assert the count DROPS after reading
  // the mentioned session rather than reaching a global zero — robust to seed.)
  await milo.goto(`${BASE_URL}/projects/${PROJECT_SLUG}`);
  await milo.reload();
  await expect(milo.locator('.cw-session-row').first()).toBeVisible();
  const dotsBefore = await milo.locator('.cw-session-row .cw-mention-dot').count();
  expect(dotsBefore).toBeGreaterThan(0);

  // Entering the mentioned session shows the team bubble and clears ITS dot.
  await milo.goto(sessionUrl);
  await expect(milo.locator('.cw-message-bubble-team').last()).toContainText('badge check (e2e)');
  await milo.goto(`${BASE_URL}/projects/${PROJECT_SLUG}`);
  await milo.reload();
  await expect(milo.locator('.cw-session-row').first()).toBeVisible();
  await expect
    .poll(() => milo.locator('.cw-session-row .cw-mention-dot').count(), { timeout: 10_000 })
    .toBeLessThan(dotsBefore);

  await oliveCtx.close();
  await miloCtx.close();
});
