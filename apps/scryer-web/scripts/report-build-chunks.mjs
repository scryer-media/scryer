import fs from "node:fs";
import path from "node:path";
import { brotliDecompressSync, gzipSync } from "node:zlib";
import { fileURLToPath } from "node:url";

const webRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const distRoot = path.join(webRoot, "dist");
const manifestPath = path.join(distRoot, ".vite", "manifest.json");
const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
const records = Object.values(manifest);

function readRegularFile(filePath) {
  let descriptor;
  try {
    descriptor = fs.openSync(filePath, "r");
  } catch (error) {
    if (error?.code === "ENOENT") return null;
    throw error;
  }

  try {
    if (!fs.fstatSync(descriptor).isFile()) return null;
    return fs.readFileSync(descriptor);
  } finally {
    fs.closeSync(descriptor);
  }
}

function staticClosure(entry) {
  const files = new Set();
  const visit = (record) => {
    if (!record || files.has(record.file)) return;
    files.add(record.file);
    for (const importedKey of record.imports ?? []) {
      visit(manifest[importedKey]);
    }
  };
  visit(entry);
  return [...files];
}

function assetSizes(relativePath) {
  const bytes = readRegularFile(path.join(distRoot, relativePath));
  if (bytes === null) {
    throw new Error(`Missing build asset: ${relativePath}`);
  }
  const brotliPath = path.join(distRoot, `${relativePath}.br`);
  const brotli = readRegularFile(brotliPath);
  return {
    raw: bytes.length,
    gzip: gzipSync(bytes, { level: 9 }).length,
    brotli: brotli?.length ?? null,
  };
}

function formatBytes(bytes) {
  return `${(bytes / 1024).toFixed(1)} KiB`;
}

function formatSizes(relativePath) {
  const sizes = assetSizes(relativePath);
  return [
    `raw ${formatBytes(sizes.raw)}`,
    `gzip ${formatBytes(sizes.gzip)}`,
    `br ${sizes.brotli === null ? "missing" : formatBytes(sizes.brotli)}`,
  ].join(", ");
}

const entries = records.filter((record) => record.isEntry);
for (const entry of entries) {
  const startupFiles = staticClosure(entry).filter((file) => file.endsWith(".js"));
  const totals = startupFiles.reduce(
    (sum, file) => {
      const sizes = assetSizes(file);
      sum.raw += sizes.raw;
      sum.gzip += sizes.gzip;
      sum.brotli += sizes.brotli ?? 0;
      return sum;
    },
    { raw: 0, gzip: 0, brotli: 0 },
  );
  console.log(
    `[bundle] startup ${entry.src ?? entry.file}: ${startupFiles.length} JS files, ` +
      `raw ${formatBytes(totals.raw)}, gzip ${formatBytes(totals.gzip)}, ` +
      `br ${formatBytes(totals.brotli)}`,
  );
}

const dynamicEntries = records
  .filter((record) => record.isDynamicEntry && record.file.endsWith(".js"))
  .sort((left, right) => assetSizes(right.file).brotli - assetSizes(left.file).brotli);
for (const record of dynamicEntries) {
  console.log(
    `[bundle] lazy ${record.src ?? record.file}: ${formatSizes(record.file)}`,
  );
}

const nonEnglishLocales = records
  .filter(
    (record) =>
      record.src?.includes("lib/i18n/locales/") &&
      !record.src.endsWith("/en.ts"),
  )
  .sort((left, right) => left.src.localeCompare(right.src));
for (const record of nonEnglishLocales) {
  console.log(`[bundle] locale ${record.src}: ${formatSizes(record.file)}`);
}

const compressibleExtension = /\.(?:js|css|svg|webmanifest|json)$/i;
const brotliErrors = [];
for (const relativePath of fs.readdirSync(distRoot, { recursive: true })) {
  if (
    typeof relativePath !== "string" ||
    relativePath.endsWith(".br") ||
    relativePath === "service-worker.js" ||
    !compressibleExtension.test(relativePath)
  ) {
    continue;
  }

  const sourcePath = path.join(distRoot, relativePath);
  const source = readRegularFile(sourcePath);
  if (source === null) continue;
  const brotliPath = `${sourcePath}.br`;
  const brotli = readRegularFile(brotliPath);
  if (brotli === null || brotli.length === 0) {
    brotliErrors.push(`${relativePath}: missing or empty .br sibling`);
    continue;
  }

  const decoded = brotliDecompressSync(brotli);
  if (!source.equals(decoded)) {
    brotliErrors.push(`${relativePath}: .br content does not match source`);
  }
}

if (brotliErrors.length > 0) {
  for (const error of brotliErrors) console.error(`[brotli] ${error}`);
  process.exitCode = 1;
} else {
  console.log("[brotli] verified all compressible build assets");
}
