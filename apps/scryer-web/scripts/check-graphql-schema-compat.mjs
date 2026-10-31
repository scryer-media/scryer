import { readFile } from "node:fs/promises";
import { basename } from "node:path";
import { fileURLToPath } from "node:url";
import {
  buildSchema,
  findBreakingChanges,
  findDangerousChanges,
} from "graphql";

export function findSchemaCompatibilityChanges(oldSdl, newSdl) {
  const oldSchema = buildSchema(oldSdl);
  const newSchema = buildSchema(newSdl);
  return {
    breaking: findBreakingChanges(oldSchema, newSchema).filter(
      (change) =>
        !(
          change.type === "DIRECTIVE_LOCATION_REMOVED" &&
          (change.description ===
            "DIRECTIVE_DEFINITION was removed from deprecated." ||
            change.description ===
              "DIRECTIVE_DEFINITION was removed from @deprecated.")
        ),
    ),
    dangerous: findDangerousChanges(oldSchema, newSchema),
  };
}

export function formatSchemaCompatibilityChanges(changes) {
  const sections = [];
  if (changes.breaking.length > 0) {
    sections.push(formatChangeSection("Breaking changes", changes.breaking));
  }
  if (changes.dangerous.length > 0) {
    sections.push(formatChangeSection("Dangerous changes", changes.dangerous));
  }
  return sections.join("\n\n");
}

export function hasSchemaCompatibilityFailure(
  changes,
  { allowDangerous = false } = {},
) {
  return (
    changes.breaking.length > 0 ||
    (!allowDangerous && changes.dangerous.length > 0)
  );
}

function formatChangeSection(title, changes) {
  return [
    `${title}:`,
    ...changes.map((change) => `  - [${change.type}] ${change.description}`),
  ].join("\n");
}

async function main(args) {
  const [oldSchemaPath, newSchemaPath, ...options] = args;
  const allowDangerous =
    options.length === 1 && options[0] === "--allow-dangerous";
  if (
    !oldSchemaPath ||
    !newSchemaPath ||
    (options.length !== 0 && !allowDangerous)
  ) {
    console.error(
      "usage: node scripts/check-graphql-schema-compat.mjs OLD_SCHEMA NEW_SCHEMA [--allow-dangerous]",
    );
    return 2;
  }

  const [oldSdl, newSdl] = await Promise.all([
    readFile(oldSchemaPath, "utf8"),
    readFile(newSchemaPath, "utf8"),
  ]);
  const changes = findSchemaCompatibilityChanges(oldSdl, newSdl);
  if (hasSchemaCompatibilityFailure(changes, { allowDangerous })) {
    console.error("GraphQL API compatibility check failed.");
    console.error(`Old schema: ${oldSchemaPath}`);
    console.error(`New schema: ${newSchemaPath}`);
    console.error(formatSchemaCompatibilityChanges(changes));
    return 1;
  }

  if (allowDangerous && changes.dangerous.length > 0) {
    console.warn(
      "GraphQL dangerous changes explicitly permitted by --allow-dangerous:",
    );
    console.warn(formatChangeSection("Dangerous changes", changes.dangerous));
  }

  console.log(
    `GraphQL API compatibility check passed: ${basename(oldSchemaPath)} -> ${basename(newSchemaPath)}`,
  );
  return 0;
}

const invokedPath = process.argv[1] ? fileURLToPath(import.meta.url) : null;
if (invokedPath && process.argv[1] === invokedPath) {
  process.exitCode = await main(process.argv.slice(2));
}
