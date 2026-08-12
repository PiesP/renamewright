export interface TauriNsisPayloadVerification {
  portableApplicationSha256: string;
  installedApplicationSha256: string;
  expectedNsisApplicationSha256: string;
  bundleMarkerOffset: number;
}

export function verifyTauriNsisPayload(
  portableBytes: Uint8Array,
  installedBytes: Uint8Array
): TauriNsisPayloadVerification;
