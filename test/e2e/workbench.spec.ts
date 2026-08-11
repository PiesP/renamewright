import { expect, test } from '@playwright/test';

test('previews a safe prefix and blocks invalid Windows names', async ({ page }) => {
  const consoleErrors: string[] = [];
  page.on('console', (message) => {
    if (message.type() === 'error') {
      consoleErrors.push(message.text());
    }
  });

  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'No sources in this plan' })).toBeVisible();
  await page.getByRole('button', { name: 'Load sample' }).click();
  await page.getByRole('textbox', { name: 'Prefix' }).fill('2026-');
  await expect(page.getByText('2026-Quarterly review.pdf')).toBeVisible();
  await expect(page.getByText('3 changes')).toBeVisible();
  await page.getByRole('button', { name: 'Inspect JSON' }).click();
  await expect(page.getByRole('dialog')).toContainText('"schemaVersion": 1');
  await expect(page.getByRole('dialog')).not.toContainText('/home/');
  await page.getByRole('button', { name: 'Close' }).click();

  await page.getByRole('textbox', { name: 'Prefix' }).fill('?');
  await expect(page.getByText('3 blocked')).toBeVisible();
  await expect(page.getByRole('status')).toContainText('3 names are blocked');
  await page.getByRole('button', { name: 'Blocked 3' }).click();
  await expect(page.getByText('Showing 3 of 3')).toBeVisible();
  await expect(page.getByRole('row')).toHaveCount(4);
  await expect(page.getByRole('button', { name: 'Execution unavailable' })).toBeDisabled();
  expect(consoleErrors).toEqual([]);
});

test('keeps the workbench inside narrow viewports', async ({ page }) => {
  for (const width of [320, 375, 414, 768]) {
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/');
    await page.getByRole('button', { name: 'Load sample' }).click();

    const sizes = await page.evaluate(() => ({
      client: document.documentElement.clientWidth,
      scroll: document.documentElement.scrollWidth,
    }));
    expect(sizes.scroll, `horizontal overflow at ${width}px`).toBeLessThanOrEqual(sizes.client);
    await expect(page.getByRole('button', { name: 'Execution unavailable' })).toBeVisible();
  }
});

test('renders the path-free startup ledger at supported narrow widths', async ({ page }) => {
  await page.addInitScript(() => {
    window.__TAURI_INTERNALS__ = {
      invoke: async (command: string) => {
        if (command === 'list_ledger') {
          return [
            {
              ledgerId: 1,
              planId: 67,
              sourceGeneration: 3,
              schemaVersion: 2,
              sourceCount: 4,
              status: 'reconciliationRequired',
              attentionStep: 2,
              recoveryAvailable: true,
            },
          ];
        }
        if (command === 'poll_source_changes') {
          return null;
        }
        if (command === 'inspect_recovery') {
          return {
            ledgerId: 1,
            direction: 'forward',
            stepIndex: 2,
            readiness: 'reconciliationRequired',
            disposition: 'notApplied',
            resumeAvailable: false,
            rollbackAvailable: false,
            reconcileAvailable: true,
          };
        }
        throw new Error(`Unexpected command: ${command}`);
      },
    };
  });

  for (const width of [320, 375, 414, 768]) {
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/');

    await expect(page.getByRole('heading', { name: 'Rename Ledger' })).toBeVisible();
    await expect(page.getByText('Plan 67')).toBeVisible();
    await expect(page.getByText('Inspection required')).toBeVisible();
    await page.getByRole('button', { name: 'Inspect plan 67 recovery' }).click();
    const inspection = page.getByRole('status').filter({ hasText: 'Observation ready to record' });
    await expect(inspection).toBeVisible();
    const inspectionBox = await inspection.boundingBox();
    const reviewBarBox = await page.locator('.review-bar').boundingBox();
    expect(inspectionBox).not.toBeNull();
    expect(reviewBarBox).not.toBeNull();
    expect(
      (inspectionBox?.y ?? 0) + (inspectionBox?.height ?? 0),
      `inspection obscured by review bar at ${width}px`
    ).toBeLessThanOrEqual(reviewBarBox?.y ?? 0);
    await expect(
      page.getByRole('button', { name: /^(resume|roll back|record observation)$/iu })
    ).toHaveCount(0);

    const sizes = await page.evaluate(() => ({
      client: document.documentElement.clientWidth,
      scroll: document.documentElement.scrollWidth,
    }));
    expect(sizes.scroll, `ledger overflow at ${width}px`).toBeLessThanOrEqual(sizes.client);
  }
});
