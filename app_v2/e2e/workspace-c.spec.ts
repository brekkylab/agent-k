import { expect, test } from '@playwright/test';

test.describe('/workspace-c cultivation canvas', () => {
  test('renders as 가꾸기 캔버스 instead of Kuse형', async ({ page }) => {
    await page.goto('/workspace-c');

    await expect(page.getByText('WS · 가꾸기 캔버스')).toBeVisible();
    await expect(page.getByRole('heading', { name: /가꾸기 캔버스/ })).toBeVisible();
    await expect(page.getByText('Kuse형')).toHaveCount(0);
    await expect(page.getByTestId('material-tray')).toBeVisible();
    await expect(page.getByTestId('collection-bed-contracts')).toBeVisible();
    await expect(page.getByTestId('collection-bed-crm')).toBeVisible();
  });

  test('moves material into collection intake and approves it from the collection', async ({ page }) => {
    test.skip((page.viewportSize()?.width ?? 0) < 700, 'desktop-focused collection drag flow');
    await page.goto('/workspace-c');

    await page.getByTestId('tray-item-legal-review').dragTo(page.getByTestId('collection-target-contracts'), {
      targetPosition: { x: 42, y: 42 },
    });

    await expect(page.getByTestId('card-legal-review')).toHaveCount(0);
    await expect(page.getByTestId('tray-item-legal-review')).toHaveCount(0);
    await expect(page.getByTestId('collection-target-contracts')).toHaveClass(/is-targeted/);
    await expect(page.getByTestId('interaction-guide')).toContainText('Contracts 근거함에서 승인 대기 중');

    const intake = page.getByTestId('collection-intake-contracts');
    await expect(intake).toContainText('승인 대기');
    await expect(page.getByTestId('evidence-pending-legal-review')).toContainText('법무팀 검토 의견서');
    await expect(page.getByTestId('evidence-pending-legal-review')).toContainText('Contracts.risk');
    await page.getByTestId('evidence-pending-legal-review').getByRole('button', { name: '승인' }).click();

    const contracts = page.getByTestId('collection-bed-contracts');
    await expect(page.getByTestId('evidence-pending-legal-review')).toHaveCount(0);
    await expect(page.getByTestId('evidence-accepted-legal-review')).toContainText('법무팀 검토 의견서');
    await expect(contracts.getByText('9조 배상 상한 200%')).toBeVisible();
    await expect(contracts).toContainText('법무팀 검토 의견서');
    await expect(contracts.getByText('Gap cleared')).toBeVisible();
  });

  test('supports quick send from the material tray without using the canvas', async ({ page }) => {
    await page.goto('/workspace-c');

    await page.getByTestId('tray-item-legal-review').getByRole('button', { name: '빠른 심기' }).click();

    await expect(page.getByTestId('tray-item-legal-review')).toHaveCount(0);
    await expect(page.getByTestId('card-legal-review')).toHaveCount(0);
    await expect(page.getByTestId('evidence-pending-legal-review')).toContainText('법무팀 검토 의견서');
    await expect(page.getByTestId('interaction-guide')).toContainText('빠른 심기');
  });

  test('uses the canvas pile to combine multiple sources into a richer proposal', async ({ page }) => {
    test.skip((page.viewportSize()?.width ?? 0) < 700, 'desktop-focused canvas synthesis flow');
    await page.goto('/workspace-c');

    await expect(page.getByTestId('synthesis-pile')).toContainText('B사 계약 리스크 검토');
    await expect(page.getByTestId('synthesis-pile')).toContainText('캔버스에서만');

    await page.getByTestId('card-contract-draft').getByRole('button', { name: '검토 묶음에 추가' }).click();
    await page.getByTestId('card-quote-mail').getByRole('button', { name: '검토 묶음에 추가' }).click();
    await page.getByTestId('tray-item-legal-review').getByRole('button', { name: '작업대에 올리기' }).click();
    await page.getByTestId('card-legal-review').getByRole('button', { name: '검토 묶음에 추가' }).click();

    await expect(page.getByTestId('card-contract-draft')).toHaveCount(0);
    await expect(page.getByTestId('card-quote-mail')).toHaveCount(0);
    await expect(page.getByTestId('card-legal-review')).toHaveCount(0);
    await expect(page.getByTestId('synthesis-pile')).toContainText('신규 서비스 계약서 2026-07.docx');
    await expect(page.getByTestId('synthesis-pile')).toContainText('A클라우드 최종 견적 메일');
    await expect(page.getByTestId('synthesis-pile')).toContainText('법무팀 검토 의견서');
    await expect(page.getByTestId('synthesis-pile')).toContainText('3개 자료로 제안 만들기');

    await page.getByRole('button', { name: '3개 자료로 제안 만들기' }).click();
    await expect(page.getByTestId('synthesis-proposal')).toContainText('Contracts.risk 복합 제안');
    await expect(page.getByTestId('synthesis-proposal')).toContainText('계약서 조항');
    await expect(page.getByTestId('synthesis-proposal')).toContainText('법무 의견');
    await expect(page.getByTestId('synthesis-proposal')).toContainText('견적 메일');

    await page.getByRole('button', { name: '복합 근거 승인' }).click();
    await expect(page.getByTestId('evidence-accepted-pile-contract-risk')).toContainText('B사 계약 리스크 검토');
    await expect(page.getByTestId('collection-bed-contracts')).toContainText('복합 근거 3개');
    await expect(page.getByTestId('collection-bed-contracts')).toContainText('Gap cleared');
  });

  test('returns a pending collection item back to the workbench', async ({ page }) => {
    test.skip((page.viewportSize()?.width ?? 0) < 700, 'desktop-focused collection drag flow');
    await page.goto('/workspace-c');

    await page.getByTestId('tray-item-legal-review').dragTo(page.getByTestId('collection-target-contracts'), {
      targetPosition: { x: 42, y: 42 },
    });
    await expect(page.getByTestId('evidence-pending-legal-review')).toBeVisible();
    await page.getByTestId('evidence-pending-legal-review').getByRole('button', { name: '되돌리기' }).click();

    await expect(page.getByTestId('evidence-pending-legal-review')).toHaveCount(0);
    await expect(page.getByTestId('card-legal-review')).toBeVisible();
    await expect(page.getByTestId('card-legal-review')).toContainText('9조 손해배상 상한');
  });

  test('resolves seeded field conflict and supports file drop analysis', async ({ page }) => {
    await page.goto('/workspace-c');

    await expect(page.getByTestId('conflict-chip')).toBeVisible();
    await page.getByRole('button', { name: '충돌 열기' }).click();
    await expect(page.getByTestId('conflict-panel')).toBeVisible();
    await page.getByRole('button', { name: '최신 근거 채택' }).click();
    await expect(page.getByTestId('conflict-chip')).toHaveCount(0);
    await expect(page.getByText('Conflict resolved')).toBeVisible();

    const dataTransfer = await page.evaluateHandle(() => {
      const dt = new DataTransfer();
      dt.items.add(new File(['감사 결과: 접근 권한 이상 없음'], 'security-audit.pdf', { type: 'application/pdf' }));
      return dt;
    });

    await page.getByTestId('canvas-dropzone').dispatchEvent('dragenter', { dataTransfer });
    await expect(page.getByTestId('file-drop-overlay')).toBeVisible();
    await page.getByTestId('canvas-dropzone').dispatchEvent('drop', { dataTransfer });

    await expect(page.getByTestId('card-file-drop')).toContainText('분석 중');
    await expect(page.getByTestId('card-file-drop')).toContainText('보안 감사 요약');
  });

  test('lets canvas cards move and return to the tray', async ({ page }) => {
    test.skip((page.viewportSize()?.width ?? 0) < 700, 'desktop-focused canvas manipulation flow');
    await page.goto('/workspace-c');

    const card = page.getByTestId('card-quote-mail');
    const before = await card.boundingBox();
    expect(before).not.toBeNull();

    await card.getByTestId('card-drag-handle').dragTo(page.getByTestId('canvas-dropzone'), {
      targetPosition: { x: 260, y: 430 },
    });

    const after = await card.boundingBox();
    expect(after).not.toBeNull();
    expect(Math.abs(after!.x - before!.x)).toBeGreaterThan(40);

    await card.getByRole('button', { name: '작업대에서 빼기' }).click();
    await expect(page.getByTestId('card-quote-mail')).toHaveCount(0);
    await expect(page.getByTestId('tray-item-quote-mail')).toBeVisible();
  });

  test('can collapse the collection sidebar and keeps collection targets legible', async ({ page }) => {
    await page.goto('/workspace-c');

    const panel = page.getByTestId('collection-panel');
    await expect(panel).toBeVisible();
    await expect(page.getByTestId('collection-target-contracts')).toBeVisible();

    await page.getByRole('button', { name: '컬렉션 패널 접기' }).click();
    await expect(panel).toHaveClass(/is-collapsed/);

    await page.getByRole('button', { name: '컬렉션 패널 펼치기' }).click();
    await expect(panel).not.toHaveClass(/is-collapsed/);
    await expect(page.getByTestId('collection-target-contracts')).toBeVisible();
  });

  test('shows multi-source material lanes in the tray', async ({ page }) => {
    await page.goto('/workspace-c');

    await expect(page.getByTestId('source-lane-all')).toContainText('All');
    await expect(page.getByTestId('source-lane-drive')).toContainText('Drive');
    await expect(page.getByTestId('source-lane-gmail')).toContainText('Gmail');
    await expect(page.getByTestId('source-lane-session')).toContainText('Session');

    await page.getByTestId('source-lane-session').click();
    await expect(page.getByTestId('tray-item-session-artifact')).toBeVisible();
    await expect(page.getByTestId('tray-item-legal-review')).toHaveCount(0);

    await page.getByTestId('source-lane-drive').click();
    await expect(page.getByTestId('tray-item-legal-review')).toBeVisible();
    await expect(page.getByTestId('tray-item-traffic-data')).toBeVisible();
  });

  test('makes canvas card to collection interaction visible with collection intake', async ({ page }) => {
    test.skip((page.viewportSize()?.width ?? 0) < 700, 'desktop-focused collection drag flow');
    await page.goto('/workspace-c');

    await expect(page.getByTestId('collection-bridge')).toContainText('근거함으로 보내기 전');

    await page.getByTestId('card-contract-draft').getByTestId('card-drag-handle').dragTo(
      page.getByTestId('collection-target-contracts'),
      { targetPosition: { x: 42, y: 42 } },
    );

    await expect(page.getByTestId('card-contract-draft')).toHaveCount(0);
    await expect(page.getByTestId('evidence-pending-contract-draft')).toContainText('신규 서비스 계약서');
    await expect(page.getByTestId('collection-target-contracts')).toHaveClass(/is-targeted/);
    await expect(page.getByTestId('collection-bridge')).toContainText('Contracts 근거함');
    await expect(page.getByTestId('interaction-guide')).toContainText('승인 또는 되돌리기');
  });

  test('restores catalog search and growth replay', async ({ page }) => {
    await page.goto('/workspace-c');

    await page.getByPlaceholder('자료 검색').fill('트래픽');
    await expect(page.getByTestId('tray-item-traffic-data')).toBeVisible();
    await expect(page.getByTestId('tray-item-legal-review')).toHaveCount(0);
    await page.getByPlaceholder('자료 검색').fill('');

    await page.getByRole('button', { name: '성장 리플레이' }).click();
    await expect(page.getByTestId('growth-replay-status')).toContainText('자료가 workspace에 다시 심어지는 중');
  });
});
