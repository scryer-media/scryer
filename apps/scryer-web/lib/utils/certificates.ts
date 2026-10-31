import type { TrustedCertificateEntry } from "@/lib/types/settings";

const CERTIFICATE_PEM_BLOCK_RE =
  /-----BEGIN CERTIFICATE-----[\s\S]*?-----END CERTIFICATE-----/g;
const SHA256_K = new Uint32Array([
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1,
  0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
  0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
  0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
  0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147,
  0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
  0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
  0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
  0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
  0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
]);

export type TrustedCertificateUploadErrorCode =
  | "pem_bundle_missing_certificate"
  | "pem_bundle_trailing_text"
  | "pem_bundle_invalid_certificate";

export class TrustedCertificateUploadError extends Error {
  readonly code: TrustedCertificateUploadErrorCode;

  constructor(code: TrustedCertificateUploadErrorCode) {
    super(code);
    this.code = code;
  }
}

export async function readTrustedCertificateEntriesFromFiles(
  files: readonly File[],
): Promise<TrustedCertificateEntry[]> {
  const entries: TrustedCertificateEntry[] = [];
  for (const file of files) {
    const buffer = await file.arrayBuffer();
    const bytes = new Uint8Array(buffer);
    const text = new TextDecoder().decode(bytes);
    const normalizedPem = text.includes("-----BEGIN CERTIFICATE-----")
      ? normalizePemCertificateBundle(text)
      : derBytesToPem(bytes);
    const fileEntries = await summarizeTrustedCertificateBundle(normalizedPem);
    entries.push(...fileEntries);
  }
  return entries;
}

export function mergeTrustedCertificateEntries(
  existing: readonly TrustedCertificateEntry[],
  incoming: readonly TrustedCertificateEntry[],
): TrustedCertificateEntry[] {
  const merged = new Map<string, TrustedCertificateEntry>();
  for (const entry of existing) {
    merged.set(entry.fingerprintSha256, entry);
  }
  for (const entry of incoming) {
    merged.set(entry.fingerprintSha256, entry);
  }
  return [...merged.values()];
}

export function bundlePemFromTrustedCertificateEntries(
  entries: readonly TrustedCertificateEntry[],
): string {
  if (entries.length === 0) {
    return "";
  }
  return `${entries.map((entry) => entry.pem.trim()).join("\n\n")}\n`;
}

export function normalizePemCertificateBundle(bundlePem: string): string {
  const blocks = collectPemBlocks(bundlePem);
  return `${blocks.join("\n")}\n`;
}

async function summarizeTrustedCertificateBundle(
  bundlePem: string,
): Promise<TrustedCertificateEntry[]> {
  const blocks = collectPemBlocks(bundlePem);
  const entries: TrustedCertificateEntry[] = [];
  for (const block of blocks) {
    const derBytes = pemBlockToDerBytes(block);
    const fingerprintSha256 = await sha256Hex(derBytes);
    entries.push({
      fingerprintSha256,
      pem: `${block.trim()}\n`,
    });
  }
  return entries;
}

function collectPemBlocks(bundlePem: string): string[] {
  const trimmed = bundlePem.trim();
  if (!trimmed) {
    return [];
  }
  const matches = trimmed.match(CERTIFICATE_PEM_BLOCK_RE) ?? [];
  if (matches.length === 0) {
    throw new TrustedCertificateUploadError("pem_bundle_missing_certificate");
  }
  const remainder = trimmed.replace(CERTIFICATE_PEM_BLOCK_RE, "").trim();
  if (remainder.length > 0) {
    throw new TrustedCertificateUploadError("pem_bundle_trailing_text");
  }
  return matches.map((block) => {
    const normalized = block.trim();
    if (!normalized.includes("-----END CERTIFICATE-----")) {
      throw new TrustedCertificateUploadError("pem_bundle_invalid_certificate");
    }
    return normalized;
  });
}

function pemBlockToDerBytes(blockPem: string): Uint8Array {
  const base64 = blockPem
    .replace("-----BEGIN CERTIFICATE-----", "")
    .replace("-----END CERTIFICATE-----", "")
    .replace(/\s+/g, "");
  if (!base64) {
    throw new TrustedCertificateUploadError("pem_bundle_invalid_certificate");
  }
  try {
    const binary = atob(base64);
    const bytes = new Uint8Array(binary.length);
    for (let index = 0; index < binary.length; index += 1) {
      bytes[index] = binary.charCodeAt(index);
    }
    return bytes;
  } catch {
    throw new TrustedCertificateUploadError("pem_bundle_invalid_certificate");
  }
}

function derBytesToPem(bytes: Uint8Array): string {
  const base64 = base64FromBytes(bytes);
  const lines = base64.match(/.{1,64}/g)?.join("\n") ?? "";
  if (!lines) {
    throw new TrustedCertificateUploadError("pem_bundle_invalid_certificate");
  }
  return `-----BEGIN CERTIFICATE-----\n${lines}\n-----END CERTIFICATE-----\n`;
}

async function sha256Hex(bytes: Uint8Array): Promise<string> {
  if (globalThis.crypto?.subtle) {
    try {
      const digest = await globalThis.crypto.subtle.digest(
        "SHA-256",
        bytes.slice(),
      );
      return bytesToHex(new Uint8Array(digest));
    } catch {
      // Fall back for non-secure contexts such as browser-led local dev and e2e.
    }
  }

  return sha256HexFallback(bytes);
}

function base64FromBytes(bytes: Uint8Array): string {
  let binary = "";
  bytes.forEach((byte) => {
    binary += String.fromCharCode(byte);
  });
  return btoa(binary);
}

function sha256HexFallback(bytes: Uint8Array): string {
  const bitLength = bytes.length * 8;
  const paddedLength = Math.ceil((bytes.length + 9) / 64) * 64;
  const padded = new Uint8Array(paddedLength);
  padded.set(bytes);
  padded[bytes.length] = 0x80;

  const view = new DataView(padded.buffer);
  const bitLengthHigh = Math.floor(bitLength / 0x1_0000_0000);
  const bitLengthLow = bitLength >>> 0;
  view.setUint32(paddedLength - 8, bitLengthHigh);
  view.setUint32(paddedLength - 4, bitLengthLow);

  let h0 = 0x6a09e667;
  let h1 = 0xbb67ae85;
  let h2 = 0x3c6ef372;
  let h3 = 0xa54ff53a;
  let h4 = 0x510e527f;
  let h5 = 0x9b05688c;
  let h6 = 0x1f83d9ab;
  let h7 = 0x5be0cd19;

  const schedule = new Uint32Array(64);
  for (let chunkOffset = 0; chunkOffset < paddedLength; chunkOffset += 64) {
    for (let index = 0; index < 16; index += 1) {
      schedule[index] = view.getUint32(chunkOffset + index * 4);
    }
    for (let index = 16; index < 64; index += 1) {
      const s0 =
        rotateRight(schedule[index - 15], 7) ^
        rotateRight(schedule[index - 15], 18) ^
        (schedule[index - 15] >>> 3);
      const s1 =
        rotateRight(schedule[index - 2], 17) ^
        rotateRight(schedule[index - 2], 19) ^
        (schedule[index - 2] >>> 10);
      schedule[index] =
        (schedule[index - 16] + s0 + schedule[index - 7] + s1) >>> 0;
    }

    let a = h0;
    let b = h1;
    let c = h2;
    let d = h3;
    let e = h4;
    let f = h5;
    let g = h6;
    let h = h7;

    for (let index = 0; index < 64; index += 1) {
      const sum1 = rotateRight(e, 6) ^ rotateRight(e, 11) ^ rotateRight(e, 25);
      const choice = (e & f) ^ (~e & g);
      const temp1 =
        (h + sum1 + choice + SHA256_K[index] + schedule[index]) >>> 0;
      const sum0 = rotateRight(a, 2) ^ rotateRight(a, 13) ^ rotateRight(a, 22);
      const majority = (a & b) ^ (a & c) ^ (b & c);
      const temp2 = (sum0 + majority) >>> 0;

      h = g;
      g = f;
      f = e;
      e = (d + temp1) >>> 0;
      d = c;
      c = b;
      b = a;
      a = (temp1 + temp2) >>> 0;
    }

    h0 = (h0 + a) >>> 0;
    h1 = (h1 + b) >>> 0;
    h2 = (h2 + c) >>> 0;
    h3 = (h3 + d) >>> 0;
    h4 = (h4 + e) >>> 0;
    h5 = (h5 + f) >>> 0;
    h6 = (h6 + g) >>> 0;
    h7 = (h7 + h) >>> 0;
  }

  return [h0, h1, h2, h3, h4, h5, h6, h7]
    .map((word) => word.toString(16).padStart(8, "0"))
    .join("");
}

function rotateRight(value: number, bits: number): number {
  return (value >>> bits) | (value << (32 - bits));
}

function bytesToHex(bytes: Uint8Array): string {
  return [...bytes].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}
