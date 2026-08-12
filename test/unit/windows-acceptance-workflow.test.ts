import { expect, test } from 'vitest';
import workflow from '../../.github/workflows/windows-acceptance.yaml?raw';
import packager from '../../scripts/prepare-windows-acceptance.ps1?raw';

test('keeps Windows acceptance manual, read-only, pinned, and source-bound', () => {
  expect(workflow).toMatch(/^on:\n {2}workflow_dispatch:\s*$/mu);
  expect(workflow).not.toMatch(/^ {2}(?:push|pull_request|schedule):/mu);
  expect(workflow).toContain('permissions:\n  contents: read');
  expect(workflow).toContain('actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1');
  expect(workflow).toContain(
    'actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a # v7.0.1'
  );
  expect(workflow).toContain('name: renamewright-windows-acceptance-$' + '{{ github.sha }}');
  expect(workflow).toContain('-SourceSha $env:GITHUB_SHA');
  expect(workflow).toContain('if-no-files-found: error');
  expect(workflow).toContain('retention-days: 7');
});

test('builds a deliberately unsigned NSIS package behind the native test gate', () => {
  const testIndex = workflow.indexOf('cargo test --workspace --all-targets --locked');
  const buildIndex = workflow.indexOf('pnpm tauri build --ci --bundles nsis --no-sign');
  const uploadIndex = workflow.indexOf('actions/upload-artifact@');

  expect(testIndex).toBeGreaterThan(0);
  expect(buildIndex).toBeGreaterThan(testIndex);
  expect(uploadIndex).toBeGreaterThan(buildIndex);
  expect(workflow).not.toContain('contents: write');
  expect(workflow).not.toContain('id-token: write');
});

test('rejects stale packaging output and records immutable checksums and limitations', () => {
  expect(packager).toContain('(git -C $root rev-parse HEAD)');
  expect(packager).toContain('$actualSha -cne $expectedSha');
  expect(packager).toContain('Exactly one NSIS installer is required');
  expect(packager).toContain('The acceptance output directory already exists');
  expect(packager).toContain('Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256');
  expect(packager).toContain('newPlanApplyEnabled = $false');
  expect(packager).toContain('The acceptance package is not code-signed.');
  expect(packager).toContain('ReFS coverage requires a separate compatible Windows environment.');
});
