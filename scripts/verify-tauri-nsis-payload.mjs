import { createHash, timingSafeEqual } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

const UNKNOWN_BUNDLE_MARKER = Buffer.from('__TAURI_BUNDLE_TYPE_VAR_UNK', 'ascii');
const NSIS_BUNDLE_MARKER = Buffer.from('__TAURI_BUNDLE_TYPE_VAR_NSS', 'ascii');

function sha256(bytes) {
  return createHash('sha256').update(bytes).digest('hex');
}

function uniqueOffset(bytes, marker, description) {
  const first = bytes.indexOf(marker);
  if (first < 0 || first !== bytes.lastIndexOf(marker)) {
    throw new Error(`${description} must contain exactly one expected Tauri bundle marker.`);
  }
  return first;
}

export function verifyTauriNsisPayload(portableBytes, installedBytes) {
  if (
    !ArrayBuffer.isView(portableBytes) ||
    !ArrayBuffer.isView(installedBytes) ||
    portableBytes.BYTES_PER_ELEMENT !== 1 ||
    installedBytes.BYTES_PER_ELEMENT !== 1
  ) {
    throw new TypeError('Payload verification requires two byte buffers.');
  }
  const portableBuffer = Buffer.from(portableBytes);
  const installedBuffer = Buffer.from(installedBytes);
  if (portableBuffer.length !== installedBuffer.length) {
    throw new Error('The independently built portable and installed payload sizes differ.');
  }

  const portableMarkerOffset = uniqueOffset(
    portableBuffer,
    UNKNOWN_BUNDLE_MARKER,
    'The independently built portable payload'
  );
  const installedMarkerOffset = uniqueOffset(
    installedBuffer,
    NSIS_BUNDLE_MARKER,
    'The installed payload'
  );
  if (
    portableBuffer.indexOf(NSIS_BUNDLE_MARKER) >= 0 ||
    installedBuffer.indexOf(UNKNOWN_BUNDLE_MARKER) >= 0 ||
    portableMarkerOffset !== installedMarkerOffset
  ) {
    throw new Error('The Tauri bundle marker transition is not the expected UNK-to-NSS change.');
  }

  const expectedInstalledBytes = Buffer.from(portableBuffer);
  NSIS_BUNDLE_MARKER.copy(expectedInstalledBytes, portableMarkerOffset);
  if (!timingSafeEqual(expectedInstalledBytes, installedBuffer)) {
    throw new Error(
      'The installed application differs from the independent portable outside the Tauri NSIS marker.'
    );
  }

  return {
    portableApplicationSha256: sha256(portableBuffer),
    installedApplicationSha256: sha256(installedBuffer),
    expectedNsisApplicationSha256: sha256(expectedInstalledBytes),
    bundleMarkerOffset: portableMarkerOffset,
  };
}

async function run() {
  const portableIndex = process.argv.indexOf('--portable');
  const installedIndex = process.argv.indexOf('--installed');
  const portablePath = portableIndex >= 0 ? process.argv[portableIndex + 1] : undefined;
  const installedPath = installedIndex >= 0 ? process.argv[installedIndex + 1] : undefined;
  if (!portablePath || !installedPath) {
    throw new Error('Both portable and installed payload inputs are required.');
  }

  let portableBytes;
  let installedBytes;
  try {
    [portableBytes, installedBytes] = await Promise.all([
      readFile(portablePath),
      readFile(installedPath),
    ]);
  } catch {
    throw new Error('A Windows application payload could not be read.');
  }
  process.stdout.write(`${JSON.stringify(verifyTauriNsisPayload(portableBytes, installedBytes))}\n`);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  run().catch((error) => {
    const message = error instanceof Error ? error.message : 'Windows payload verification failed.';
    process.stderr.write(`${message}\n`);
    process.exitCode = 1;
  });
}
