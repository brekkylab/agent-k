/**
 * E2E tests for the file preview modal (FilePreviewModal + preview/* sub-views).
 *
 * Requires a running dev stack. Start it with:
 *   app/scripts/run_cowork_demo.sh
 * or manually:
 *   backend + VITE_BACKEND_V2_URL=http://127.0.0.1:8080 pnpm -C app dev
 *
 * Demo credentials used:  olive / cowork-demo  → project "klientco-q2"
 *
 * The seeded shared files are .md and .csv only, so each test uploads a small
 * fixture before exercising the preview behaviour, then leaves the uploaded
 * file in place (parallel workers use uniquely suffixed names to avoid races).
 */
import { test, expect, type Page } from '@playwright/test';

// ─── helpers ──────────────────────────────────────────────────────────────

const BASE_URL = process.env.PLAYWRIGHT_BASE_URL ?? 'http://127.0.0.1:4110';
const DEMO_USERNAME = 'olive';
const DEMO_PASSWORD = 'cowork-demo';
const PROJECT_SLUG = 'klientco-q2';
const FILES_URL = `/projects/${PROJECT_SLUG}/files`;

async function loginAndGoToFiles(page: Page): Promise<void> {
  await page.goto(`${BASE_URL}/login`);
  await page.locator('input[name="username"]').fill(DEMO_USERNAME);
  await page.locator('input[name="password"]').fill(DEMO_PASSWORD);
  await page.locator('button[type="submit"]').click();
  // Wait for redirect to /projects then navigate to the Files page
  await page.waitForURL(`${BASE_URL}/projects**`);
  await page.goto(`${BASE_URL}${FILES_URL}`);
  // File list must be visible before we interact with rows
  await expect(page.locator('.cw-file-body')).toBeVisible();
}

/**
 * Upload a small Buffer as a file into the current folder of the Files page.
 * Returns the exact filename used so callers can locate the row.
 */
async function uploadFixture(
  page: Page,
  filename: string,
  content: Buffer,
  mimeType: string,
): Promise<string> {
  const input = page.locator('input[type="file"][hidden]');
  await input.setInputFiles({
    name: filename,
    mimeType,
    buffer: content,
  });
  // Wait for the row to appear in the file list
  await expect(page.locator('.cw-file-row').filter({ hasText: filename })).toBeVisible({
    timeout: 10_000,
  });
  return filename;
}

async function openPreviewModal(page: Page, filename: string): Promise<void> {
  const row = page.locator('.cw-file-row').filter({ hasText: filename });
  await row.dblclick();
  await expect(page.getByRole('dialog')).toBeVisible({ timeout: 10_000 });
}

async function closeModal(page: Page): Promise<void> {
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog')).toBeHidden({ timeout: 5_000 });
}

// ─── minimal fixture content ───────────────────────────────────────────────

// 1×1 transparent PNG (67 bytes) — valid image, tiny.
const TINY_PNG = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==',
  'base64',
);

// Minimal single-page PDF. No xref table is emitted on purpose — computing exact
// byte offsets by hand is brittle, so we let pdf.js rebuild the cross-reference
// via its recovery path ("Indexing all PDF objects"), which still renders a
// <canvas>. A wrong-offset xref is worse than none (it can defeat recovery).
const TINY_PDF = Buffer.from(
  '%PDF-1.1\n' +
  '1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n' +
  '2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n' +
  '3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]>>endobj\n' +
  'trailer<</Root 1 0 R>>\n%%EOF',
);

const TINY_HTML = Buffer.from('<html><body><p>hello</p></body></html>');

// .docx-like: just a fake binary so the backend accepts it.
const TINY_DOCX = Buffer.from('PK\x03\x04fake-docx-content');

// ─── tests ────────────────────────────────────────────────────────────────

/**
 * Use a test.describe block with a beforeEach that logs in once per test.
 * Playwright isolates browser contexts by default — each test gets a fresh
 * page, so login is repeated cheaply via the API form.
 */
test.describe('file preview', () => {
  // Each test uses a unique suffix to avoid filename collisions when workers
  // run in parallel or the tests are re-run without resetting seed data.
  let suffix: string;

  test.beforeEach(async ({}, testInfo) => {
    suffix = testInfo.workerIndex.toString();
  });

  test('double-clicking an image file opens the preview modal', async ({ page }) => {
    await loginAndGoToFiles(page);
    const filename = `fixture-image-${suffix}.png`;
    await uploadFixture(page, filename, TINY_PNG, 'image/png');

    await openPreviewModal(page, filename);
    const modal = page.getByRole('dialog');

    // The ImageView renders an <img> inside .cw-preview-image
    await expect(modal.locator('img')).toBeVisible({ timeout: 15_000 });

    await closeModal(page);
  });

  test('html preview iframe is sandboxed without same-origin', async ({ page }) => {
    await loginAndGoToFiles(page);
    const filename = `fixture-page-${suffix}.html`;
    await uploadFixture(page, filename, TINY_HTML, 'text/html');

    await openPreviewModal(page, filename);
    const modal = page.getByRole('dialog');

    // HtmlView renders <iframe sandbox="allow-scripts">
    const iframe = modal.locator('iframe');
    await expect(iframe).toBeVisible({ timeout: 15_000 });
    await expect(iframe).toHaveAttribute('sandbox', 'allow-scripts');
    const sandbox = await iframe.getAttribute('sandbox');
    expect(sandbox).not.toContain('allow-same-origin');

    await closeModal(page);
  });

  test('pdf preview renders without worker version mismatch', async ({ page }) => {
    await loginAndGoToFiles(page);
    const filename = `fixture-doc-${suffix}.pdf`;
    await uploadFixture(page, filename, TINY_PDF, 'application/pdf');

    const errors: string[] = [];
    page.on('console', (msg) => {
      if (msg.type() === 'error') errors.push(msg.text());
    });

    await openPreviewModal(page, filename);
    const modal = page.getByRole('dialog');

    // react-pdf renders a <canvas> per page once the worker loads.
    // We wait up to 20 s because the PDF worker may need to initialise.
    await expect(modal.locator('canvas').first()).toBeVisible({ timeout: 20_000 });

    // The specific string emitted when pdfjs-dist and its worker are mismatched:
    // "Warning: Setting up fake worker."  /  "API version X does not match the Worker version Y"
    expect(errors.join('\n')).not.toMatch(/API version .* does not match the Worker version/);

    await closeModal(page);
  });

  test('unsupported file shows fallback download card', async ({ page }) => {
    await loginAndGoToFiles(page);
    const filename = `fixture-unsupported-${suffix}.docx`;
    await uploadFixture(page, filename, TINY_DOCX, 'application/vnd.openxmlformats-officedocument.wordprocessingml.document');

    await openPreviewModal(page, filename);
    const modal = page.getByRole('dialog');

    // FallbackCard renders a button with t('preview.download') = "Download" / "다운로드"
    await expect(
      modal.getByRole('button', { name: /^(Download|다운로드)$/i }),
    ).toBeVisible({ timeout: 10_000 });

    await closeModal(page);
  });
});
