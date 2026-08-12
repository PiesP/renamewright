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
  await expect(page.getByRole('dialog')).toContainText('"schemaVersion": 6');
  await expect(page.getByRole('dialog')).not.toContainText('/home/');
  await expect(page.getByRole('button', { name: 'Export CSV…' })).toBeDisabled();
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

test('edits and reorders an ordered text rule pipeline', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('button', { name: 'Load sample' }).click();
  await page.getByRole('textbox', { name: 'Prefix' }).fill('draft-');
  await page.getByRole('combobox', { name: 'New rule' }).selectOption('literalReplace');
  await page.getByRole('button', { name: 'Add rule' }).click();

  const replaceEditor = page.locator('.rule-editor').filter({ hasText: 'Replace text' });
  await replaceEditor.getByRole('textbox', { name: 'Find' }).fill('draft');
  await replaceEditor.getByRole('textbox', { name: 'Replace with' }).fill('final');
  await expect(page.getByText('final-Quarterly review.pdf')).toBeVisible();

  await page.getByRole('button', { name: 'Move Replace text up' }).click();
  await expect(page.getByText('draft-Quarterly review.pdf')).toBeVisible();

  await page.getByRole('combobox', { name: 'New rule' }).selectOption('regexReplace');
  await page.getByRole('button', { name: 'Add rule' }).click();
  const regexEditor = page.locator('.rule-editor').filter({ hasText: 'Replace by pattern' });
  await regexEditor.getByRole('textbox', { name: 'Rust regex' }).fill('(');
  await expect(regexEditor).toContainText('Rule 3 uses an invalid regular expression.');
  await expect(page.getByRole('button', { name: 'Execution unavailable' })).toBeDisabled();
});

test('allocates sequence numbers before preview rows are rendered', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('button', { name: 'Load sample' }).click();
  await page.getByRole('combobox', { name: 'New rule' }).selectOption('sequence');
  await page.getByRole('button', { name: 'Add rule' }).click();

  const sequenceEditor = page.locator('.rule-editor').filter({ hasText: 'Add sequence' });
  await sequenceEditor.getByRole('combobox', { name: 'Number by' }).selectOption('nameAscending');
  await sequenceEditor.getByRole('spinbutton', { name: 'Start' }).fill('10');
  await sequenceEditor.getByRole('spinbutton', { name: 'Step' }).fill('5');
  await sequenceEditor.getByRole('spinbutton', { name: 'Padding' }).fill('2');

  await expect(page.getByText('10-Quarterly review.pdf')).toBeVisible();
  await expect(page.getByText('20-team-photo 01.jpg')).toBeVisible();
  await expect(page.getByText('15-project-notes.txt')).toBeVisible();
  await expect(sequenceEditor).toContainText(
    'Number allocation is fixed before preview rows are rendered.'
  );
});

test('previews filename structure rules and keeps invalid extensions blocked', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('button', { name: 'Load sample' }).click();
  await page.getByRole('textbox', { name: 'Prefix' }).fill('  Final   ');

  await page.getByRole('combobox', { name: 'New rule' }).selectOption('extension');
  await page.getByRole('button', { name: 'Add rule' }).click();
  const extensionEditor = page.locator('.rule-editor').filter({ hasText: 'Change extension' });
  await extensionEditor.getByRole('combobox', { name: 'Operation' }).selectOption('replace');
  await extensionEditor.getByRole('textbox', { name: 'New extension (without dot)' }).fill('md');

  await page.getByRole('combobox', { name: 'New rule' }).selectOption('whitespaceCleanup');
  await page.getByRole('button', { name: 'Add rule' }).click();
  const whitespaceEditor = page.locator('.rule-editor').filter({ hasText: 'Clean whitespace' });
  await whitespaceEditor.getByRole('textbox', { name: 'Collapse runs to' }).fill('-');
  await expect(page.getByText('Final-Quarterly-review.md')).toBeVisible();

  await page.getByRole('combobox', { name: 'New rule' }).selectOption('case');
  await page.getByRole('button', { name: 'Add rule' }).click();
  const caseEditor = page.locator('.rule-editor').filter({ hasText: 'Change case' });
  await caseEditor.getByRole('combobox', { name: 'Apply to' }).selectOption('extension');
  await caseEditor.getByRole('combobox', { name: 'Case' }).selectOption('uppercase');
  await expect(page.getByText('Final-Quarterly-review.MD')).toBeVisible();

  await page.getByRole('combobox', { name: 'New rule' }).selectOption('unicodeNormalization');
  await page.getByRole('button', { name: 'Add rule' }).click();
  const normalizationEditor = page.locator('.rule-editor').filter({ hasText: 'Normalize Unicode' });
  await expect(
    normalizationEditor.getByRole('combobox', { name: 'Normalization form' })
  ).toHaveValue('nfc');

  await extensionEditor
    .getByRole('textbox', { name: 'New extension (without dot)' })
    .fill('.private');
  await expect(extensionEditor).toContainText('Rule 2 needs an extension without a leading dot.');
  await expect(page.getByRole('button', { name: 'Execution unavailable' })).toBeDisabled();
});

test('previews character ranges and Unicode character classes', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('button', { name: 'Load sample' }).click();
  await page.getByRole('combobox', { name: 'New rule' }).selectOption('range');
  await page.getByRole('button', { name: 'Add rule' }).click();
  const rangeEditor = page.locator('.rule-editor').filter({ hasText: 'Select character range' });
  await rangeEditor.getByRole('spinbutton', { name: 'Range length' }).fill('9');
  await expect(page.getByText('Quarterly.pdf')).toBeVisible();

  await page.getByRole('combobox', { name: 'New rule' }).selectOption('characterClass');
  await page.getByRole('button', { name: 'Add rule' }).click();
  const classEditor = page.locator('.rule-editor').filter({ hasText: 'Filter character class' });
  await classEditor.getByRole('combobox', { name: 'Class action' }).selectOption('keep');
  await classEditor.getByRole('combobox', { name: 'Unicode class' }).selectOption('letter');
  await expect(classEditor).toContainText('Uses Unicode properties, not ASCII-only ranges.');
  await expect(page.getByText('Quarterly.pdf')).toBeVisible();
});

test('keeps an inline override across shared rule changes and resets it safely', async ({
  page,
}) => {
  await page.goto('/');
  await page.getByRole('button', { name: 'Load sample' }).click();
  await page.getByRole('button', { name: 'Edit override for Quarterly review.pdf' }).click();
  const override = page.getByRole('textbox', { name: 'Override name for Quarterly review.pdf' });
  await override.fill('manual.md');
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(page.getByText('manual.md')).toBeVisible();
  await expect(page.getByText('Override', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Inspect JSON' }).click();
  await expect(page.getByRole('dialog')).toContainText('"overrides"');
  await expect(page.getByRole('dialog')).toContainText('"overrideApplied": true');
  await expect(page.getByRole('dialog')).not.toContainText('/home/');
  await page.getByRole('button', { name: 'Close' }).click();

  await page.getByRole('textbox', { name: 'Prefix' }).fill('shared-');
  await expect(page.getByText('shared-project-notes.txt')).toBeVisible();
  await expect(page.getByText('manual.md')).toBeVisible();
  await expect(page.getByText('shared-Quarterly review.pdf')).toHaveCount(0);

  await page.getByRole('button', { name: 'Edit override for Quarterly review.pdf' }).click();
  await page
    .getByRole('textbox', { name: 'Override name for Quarterly review.pdf' })
    .fill('bad?.txt');
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(page.getByText('1 blocked')).toBeVisible();
  await expect(page.getByText('Illegal Windows character')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Execution unavailable' })).toBeDisabled();

  await page.getByRole('button', { name: 'Reset override for Quarterly review.pdf' }).click();
  await expect(page.getByText('shared-Quarterly review.pdf')).toBeVisible();
  await expect(page.getByText('bad?.txt')).toHaveCount(0);
});

test('persists and reapplies a local rule preset', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('button', { name: 'Load sample' }).click();
  await page.getByRole('textbox', { name: 'Prefix' }).fill('saved-');
  await page.getByRole('textbox', { name: 'Preset name' }).fill('My local preset');
  await page.getByRole('textbox', { name: 'Preset name' }).press('Enter');
  await expect(page.getByRole('status')).toContainText('Local rule preset saved.');

  await page.getByRole('textbox', { name: 'Prefix' }).fill('other-');
  await expect(page.getByText('other-project-notes.txt')).toBeVisible();
  await page.reload();
  await expect(page.getByText('My local preset')).toBeVisible();
  await page.getByRole('button', { name: 'Load sample' }).click();
  await page.getByRole('button', { name: 'Apply' }).click();
  await expect(page.getByText('saved-project-notes.txt')).toBeVisible();

  await page.setViewportSize({ width: 320, height: 900 });
  const sizes = await page.evaluate(() => ({
    client: document.documentElement.clientWidth,
    scroll: document.documentElement.scrollWidth,
  }));
  expect(sizes.scroll).toBeLessThanOrEqual(sizes.client);
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

test('persists Korean and keeps translated controls inside supported viewports', async ({
  page,
}) => {
  await page.goto('/');
  await page.evaluate(() => localStorage.setItem('renamewright.locale', 'ko'));

  for (const width of [320, 375, 414, 768]) {
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/');

    await expect(page.locator('html')).toHaveAttribute('lang', 'ko');
    await expect(page.getByRole('heading', { name: '이름 변경 규칙' })).toBeVisible();
    await expect(page.getByRole('combobox', { name: '언어' })).toHaveValue('ko');
    await page.getByRole('button', { name: '샘플 불러오기' }).click();
    await expect(page.getByRole('columnheader', { name: '원본' })).toBeVisible();
    await page.getByRole('textbox', { name: '접두사' }).fill('?');
    await expect(page.getByRole('status')).toContainText('이름 3개가 차단되었습니다');

    const layout = await page.evaluate(() => {
      const clippedControls = [...document.querySelectorAll<HTMLElement>('button, select')]
        .filter((element) => element.offsetParent !== null)
        .filter((element) => element.scrollWidth > element.clientWidth + 1)
        .map((element) => element.textContent?.trim() ?? element.tagName);
      return {
        client: document.documentElement.clientWidth,
        scroll: document.documentElement.scrollWidth,
        clippedControls,
      };
    });
    expect(layout.scroll, `Korean overflow at ${width}px`).toBeLessThanOrEqual(layout.client);
    expect(layout.clippedControls, `clipped Korean controls at ${width}px`).toEqual([]);
  }

  await page.getByRole('combobox', { name: '언어' }).selectOption('en');
  await page.reload();
  await expect(page.locator('html')).toHaveAttribute('lang', 'en');
  await expect(page.getByRole('heading', { name: 'Rename rules' })).toBeVisible();
});

test('preserves keyboard order and Windows accessibility preferences', async ({ page }) => {
  await page.goto('/');

  await page.keyboard.press('Tab');
  await expect(page.getByRole('combobox', { name: 'Language' })).toBeFocused();
  await page.keyboard.press('Tab');
  await expect(page.getByRole('button', { name: 'Remove Add prefix' })).toBeFocused();
  await page.keyboard.press('Tab');
  await expect(page.getByRole('checkbox', { name: 'Enabled' })).toBeFocused();
  await page.keyboard.press('Tab');
  await expect(page.getByRole('textbox', { name: 'Prefix' })).toBeFocused();

  await page.setViewportSize({ width: 320, height: 900 });
  for (const control of [
    page.getByRole('button', { name: 'Remove Add prefix' }),
    page.getByRole('button', { name: 'Add rule' }),
  ]) {
    const box = await control.boundingBox();
    expect(box).not.toBeNull();
    expect(box?.width).toBeGreaterThanOrEqual(44);
    expect(box?.height).toBeGreaterThanOrEqual(44);
  }

  await page.emulateMedia({ forcedColors: 'active', reducedMotion: 'reduce' });
  const removeButton = page.getByRole('button', { name: 'Remove Add prefix' });
  await removeButton.focus();
  const accessibilityStyles = await removeButton.evaluate((element) => {
    const style = getComputedStyle(element);
    return {
      outlineStyle: style.outlineStyle,
      outlineWidth: Number.parseFloat(style.outlineWidth),
      transitionDuration: Number.parseFloat(style.transitionDuration),
    };
  });
  expect(accessibilityStyles.outlineStyle).not.toBe('none');
  expect(accessibilityStyles.outlineWidth).toBeGreaterThanOrEqual(3);
  expect(accessibilityStyles.transitionDuration).toBeLessThanOrEqual(0.001);

  const disabledButton = page.getByRole('button', { name: 'Execution unavailable' });
  await expect(disabledButton).toHaveCSS('opacity', '1');
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
              undoOfPlanId: null,
              undoAvailable: false,
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
        if (command === 'apply_recovery_action') {
          return {
            performed: true,
            outcome: 'reconciled',
            ledger: [
              {
                ledgerId: 1,
                planId: 67,
                sourceGeneration: 3,
                schemaVersion: 2,
                sourceCount: 4,
                status: 'forwardPending',
                attentionStep: 2,
                recoveryAvailable: true,
                undoOfPlanId: null,
                undoAvailable: false,
              },
            ],
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
    await page.getByRole('button', { name: 'Record observation' }).click();
    await expect(page.getByText('Forward recovery pending')).toBeVisible();
    await expect(
      page.getByText('Prepared-step observation recorded. Inspect the transaction again.')
    ).toBeVisible();
    await expect(page.getByText('Observation ready to record')).toHaveCount(0);

    const sizes = await page.evaluate(() => ({
      client: document.documentElement.clientWidth,
      scroll: document.documentElement.scrollWidth,
    }));
    expect(sizes.scroll, `ledger overflow at ${width}px`).toBeLessThanOrEqual(sizes.client);
  }
});

test('keeps forward recovery cancellation above the mobile review bar', async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 900 });
  await page.addInitScript(() => {
    window.__TAURI_INTERNALS__ = {
      invoke: async (command: string) => {
        if (command === 'list_ledger') {
          return [
            {
              ledgerId: 3,
              planId: 73,
              sourceGeneration: 5,
              schemaVersion: 2,
              sourceCount: 2,
              status: 'forwardPending',
              attentionStep: 1,
              recoveryAvailable: true,
              undoOfPlanId: null,
              undoAvailable: false,
            },
          ];
        }
        if (command === 'poll_source_changes') {
          return null;
        }
        if (command === 'inspect_recovery') {
          return {
            ledgerId: 3,
            direction: 'forward',
            stepIndex: 1,
            readiness: 'ready',
            disposition: null,
            resumeAvailable: true,
            rollbackAvailable: true,
            reconcileAvailable: false,
          };
        }
        if (command === 'apply_recovery_action') {
          return new Promise(() => undefined);
        }
        if (command === 'cancel_recovery') {
          return true;
        }
        throw new Error(`Unexpected command: ${command}`);
      },
    };
  });
  await page.goto('/');
  await page.getByRole('button', { name: 'Inspect plan 73 recovery' }).click();
  await page.getByRole('button', { name: 'Resume' }).click();
  await page.getByRole('button', { name: 'Cancel and roll back' }).click();
  await expect(page.getByRole('button', { name: 'Cancellation requested' })).toBeDisabled();

  const inspectionBox = await page
    .getByRole('status')
    .filter({ hasText: 'Identity checks passed' })
    .boundingBox();
  const reviewBarBox = await page.locator('.review-bar').boundingBox();
  expect(inspectionBox).not.toBeNull();
  expect(reviewBarBox).not.toBeNull();
  expect((inspectionBox?.y ?? 0) + (inspectionBox?.height ?? 0)).toBeLessThanOrEqual(
    reviewBarBox?.y ?? 0
  );
});

test('keeps the path-free Undo inspection usable at supported widths', async ({ page }) => {
  await page.addInitScript(() => {
    window.__TAURI_INTERNALS__ = {
      invoke: async (command: string) => {
        if (command === 'list_ledger') {
          return [
            {
              ledgerId: 4,
              planId: 80,
              sourceGeneration: 6,
              schemaVersion: 3,
              sourceCount: 3,
              status: 'completed',
              attentionStep: null,
              recoveryAvailable: false,
              undoOfPlanId: null,
              undoAvailable: true,
            },
          ];
        }
        if (command === 'poll_source_changes') {
          return null;
        }
        if (command === 'inspect_undo') {
          return {
            ledgerId: 4,
            originalPlanId: 80,
            sourceCount: 3,
            readiness: 'ready',
            blockReason: null,
            undoAvailable: true,
          };
        }
        throw new Error(`Unexpected command: ${command}`);
      },
    };
  });

  for (const width of [320, 375, 414, 768]) {
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/');
    await page.getByRole('button', { name: 'Inspect plan 80 Undo' }).click();
    const inspection = page.getByRole('status').filter({ hasText: 'Undo checks passed' });
    await expect(inspection).toBeVisible();
    const undoButton = page.getByRole('button', { name: 'Undo rename' });
    await expect(undoButton).toBeVisible();
    const buttonBox = await undoButton.boundingBox();
    expect(buttonBox?.height ?? 0).toBeGreaterThanOrEqual(44);

    const sizes = await page.evaluate(() => ({
      client: document.documentElement.clientWidth,
      scroll: document.documentElement.scrollWidth,
    }));
    expect(sizes.scroll, `Undo ledger overflow at ${width}px`).toBeLessThanOrEqual(sizes.client);
    const reviewBarBox = await page.locator('.review-bar').boundingBox();
    const inspectionBox = await inspection.boundingBox();
    expect(inspectionBox).not.toBeNull();
    expect(reviewBarBox).not.toBeNull();
    expect((inspectionBox?.y ?? 0) + (inspectionBox?.height ?? 0)).toBeLessThanOrEqual(
      reviewBarBox?.y ?? 0
    );
  }
});

test('offers safe cancellation while Undo is active', async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 900 });
  await page.addInitScript(() => {
    window.__TAURI_INTERNALS__ = {
      invoke: async (command: string) => {
        if (command === 'list_ledger') {
          return [
            {
              ledgerId: 4,
              planId: 80,
              sourceGeneration: 6,
              schemaVersion: 3,
              sourceCount: 3,
              status: 'completed',
              attentionStep: null,
              recoveryAvailable: false,
              undoOfPlanId: null,
              undoAvailable: true,
            },
          ];
        }
        if (command === 'poll_source_changes') {
          return null;
        }
        if (command === 'inspect_undo') {
          return {
            ledgerId: 4,
            originalPlanId: 80,
            sourceCount: 3,
            readiness: 'ready',
            blockReason: null,
            undoAvailable: true,
          };
        }
        if (command === 'apply_undo') {
          return new Promise(() => undefined);
        }
        if (command === 'cancel_undo') {
          return true;
        }
        throw new Error(`Unexpected command: ${command}`);
      },
    };
  });

  await page.goto('/');
  await page.getByRole('button', { name: 'Inspect plan 80 Undo' }).click();
  await page.getByRole('button', { name: 'Undo rename' }).click();
  await page.getByRole('button', { name: 'Cancel and roll back' }).click();

  await expect(page.getByRole('button', { name: 'Cancellation requested' })).toBeDisabled();
  await expect(page.getByRole('status').last()).toContainText(
    'Cancellation requested. Undo will roll back at the next safe step…'
  );
});
