import { readFileSync } from 'node:fs';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const PEAK_RSS_BUDGET_BYTES = 256 * 1024 * 1024;
const root = dirname(dirname(fileURLToPath(import.meta.url)));

await run('cargo', [
  'build',
  '--release',
  '--locked',
  '--package',
  'renamewright-platform',
  '--example',
  'large_batch_budget',
]);

const executable = join(
  root,
  'target',
  'release',
  'examples',
  process.platform === 'win32' ? 'large_batch_budget.exe' : 'large_batch_budget'
);
const measurement = await run(executable, [], process.platform === 'linux');
const resultLine = measurement.stdout
  .trim()
  .split('\n')
  .findLast((line) => line.startsWith('{'));
if (!resultLine) {
  throw new Error('The native performance probe did not emit a JSON result.');
}
const result = JSON.parse(resultLine);
if (measurement.peakRssBytes !== undefined && measurement.peakRssBytes > PEAK_RSS_BUDGET_BYTES) {
  throw new Error(
    `Native peak RSS ${measurement.peakRssBytes} exceeded ${PEAK_RSS_BUDGET_BYTES} bytes.`
  );
}

console.log(
  JSON.stringify({
    runtime: 'native',
    ...result,
    peakRssBytes: measurement.peakRssBytes ?? null,
    peakRssBudgetBytes:
      measurement.peakRssBytes === undefined ? null : PEAK_RSS_BUDGET_BYTES,
  })
);

function run(command, arguments_, sampleRss = false) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, arguments_, {
      cwd: root,
      env: process.env,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    let stdout = '';
    let stderr = '';
    let peakRssBytes = sampleRss ? readRssBytes(child.pid) : undefined;
    const sampler = sampleRss
      ? setInterval(() => {
          peakRssBytes = Math.max(peakRssBytes ?? 0, readRssBytes(child.pid) ?? 0);
        }, 5)
      : undefined;

    child.stdout.on('data', (chunk) => {
      stdout += chunk;
    });
    child.stderr.on('data', (chunk) => {
      stderr += chunk;
    });
    child.on('error', (error) => {
      if (sampler) clearInterval(sampler);
      reject(error);
    });
    child.on('close', (code, signal) => {
      if (sampler) clearInterval(sampler);
      if (code !== 0) {
        reject(
          new Error(
            `${command} failed with ${signal ?? `exit ${code ?? 'unknown'}`}.\n${stdout}${stderr}`
          )
        );
        return;
      }
      resolve({ stdout, stderr, peakRssBytes });
    });
  });
}

function readRssBytes(processId) {
  if (!processId) return undefined;
  try {
    const status = readFileSync(`/proc/${processId}/status`, 'utf8');
    const match = /^VmRSS:\s+(\d+)\s+kB$/mu.exec(status);
    return match ? Number.parseInt(match[1], 10) * 1024 : undefined;
  } catch {
    return undefined;
  }
}
