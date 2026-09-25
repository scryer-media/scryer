import { readdir, readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const appRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const ENGLISH_DICTIONARY = "lib/i18n/locales/en.ts";
const SCAN_DIRECTORIES = ["app", "components", "lib", "src", "workers"];
const SCANNED_EXTENSIONS = new Set([".ts", ".tsx"]);
const SKIPPED_DIRECTORIES = new Set(["node_modules", "dist", "locales"]);
const TEST_FILE = /\.test\.[cm]?[jt]sx?$/;

/**
 * Keys handed to the translator as quoted string literals. Only literals are
 * checked: a key assembled at runtime, including one written with a template
 * literal, cannot be verified without rendering the component that builds it.
 */
const TRANSLATION_CALL = /\b(?:t|translate)\(\s*["']([^"'\\]+)["']\s*\)/g;

/**
 * Entries of a flat locale dictionary: `"namespace.key": "..."` and the
 * handful of bare identifiers the dictionary also carries. Comment lines are
 * skipped so a commented-out entry is not counted as present.
 */
const DICTIONARY_ENTRY = /^(?:"([^"]+)"|([A-Za-z_$][\w$]*))\s*:/;

export function translationKeysInSource(source) {
  return [...source.matchAll(TRANSLATION_CALL)].map((match) => match[1]);
}

export function dictionaryKeysInSource(source) {
  const keys = new Set();
  for (const line of source.split("\n")) {
    const trimmed = line.trimStart();
    if (trimmed.startsWith("//") || trimmed.startsWith("*") || trimmed.startsWith("/*")) {
      continue;
    }
    const match = DICTIONARY_ENTRY.exec(trimmed);
    if (match) {
      keys.add(match[1] ?? match[2]);
    }
  }
  return keys;
}

/**
 * Keys the UI asks for that the English dictionary cannot answer. English is
 * the fallback every other locale resolves against, so a key missing here
 * renders as the raw key in every language.
 */
export function missingEnglishKeys({ dictionaryKeys, usages }) {
  return [...usages.entries()]
    .filter(([key]) => !dictionaryKeys.has(key))
    .map(([key, files]) => ({ key, files: [...files].sort() }))
    .sort((left, right) => left.key.localeCompare(right.key));
}

async function sourceFiles(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    const full = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      if (!SKIPPED_DIRECTORIES.has(entry.name) && !entry.name.startsWith(".")) {
        files.push(...(await sourceFiles(full)));
      }
      continue;
    }
    if (SCANNED_EXTENSIONS.has(path.extname(entry.name)) && !TEST_FILE.test(entry.name)) {
      files.push(full);
    }
  }
  return files;
}

async function collectUsages() {
  const usages = new Map();
  for (const directory of SCAN_DIRECTORIES) {
    const root = path.join(appRoot, directory);
    const files = await sourceFiles(root).catch((error) => {
      if (error.code === "ENOENT") {
        return [];
      }
      throw error;
    });
    for (const file of files) {
      for (const key of translationKeysInSource(await readFile(file, "utf8"))) {
        const filesForKey = usages.get(key) ?? new Set();
        filesForKey.add(path.relative(appRoot, file));
        usages.set(key, filesForKey);
      }
    }
  }
  return usages;
}

async function main() {
  const dictionaryKeys = dictionaryKeysInSource(
    await readFile(path.join(appRoot, ENGLISH_DICTIONARY), "utf8"),
  );
  const usages = await collectUsages();
  const missing = missingEnglishKeys({ dictionaryKeys, usages });

  if (missing.length > 0) {
    console.error(
      `${missing.length} translation key(s) are used by the UI but are missing from ${ENGLISH_DICTIONARY}:`,
    );
    for (const { key, files } of missing) {
      console.error(`  ${key}  <- ${files.join(", ")}`);
    }
    console.error(
      "Add the English entry, or point the call site at an existing key. English is the fallback every other locale resolves against.",
    );
    return 1;
  }

  console.log(
    `Translation keys: ${usages.size} used by the UI, all present in ${ENGLISH_DICTIONARY}.`,
  );
  return 0;
}

const invokedPath = process.argv[1] ? fileURLToPath(import.meta.url) : null;
if (invokedPath && process.argv[1] === invokedPath) {
  process.exitCode = await main();
}
