import { regoCodeAt } from "./rego-lexical.ts";

// Match the backend package/import insertion without inventing positions.
export function regoSourceLine(source: string, compiledLine: number): number {
  const lines = source.split("\n");
  const packageIndex = lines.findIndex((line) =>
    line.trim().startsWith("package "),
  );
  const hasImport = lines.some((line) => line.trim() === "import rego.v1");
  let line = compiledLine - (packageIndex < 0 ? 1 : 0);
  if (!hasImport && line > (packageIndex < 0 ? 0 : packageIndex + 1)) line--;
  return line;
}

export type RuleValidationDiagnostic = {
  code: string;
  message: string;
  line?: number | null;
  column?: number | null;
  path?: string | null;
};
export type RegoValidationResult = {
  valid: boolean;
  errors: string[];
  diagnostics?: RuleValidationDiagnostic[];
  unavailable?: boolean;
};
export type RegoDiagnostic = { line: number; column: number; message: string };

const DIAGNOSTIC_KEYS: Readonly<Record<string, string>> = {
  RULE_UNKNOWN_INPUT_PATH: "error.ruleUnknownInputPath",
  RULE_DYNAMIC_INPUT_PATH: "error.ruleDynamicInputPath",
  RULE_PACKAGE_REQUIRED: "error.rulePackageRequired",
  RULE_INPUT_IMPORT_UNSUPPORTED: "error.ruleInputImportUnsupported",
  RULE_MATCH_REQUIRED: "error.ruleMatchRequired",
  RULE_VOTE_REQUIRED: "error.ruleVoteRequired",
  RULE_TARGETS_PERSON: "error.ruleTargetsPerson",
};

/** Compiler-owned diagnostics keep their detail; owned conditions use i18n. */
export function regoDiagnosticMessage(
  diagnostic: RuleValidationDiagnostic,
  translate: (
    key: string,
    values?: Record<string, string | number | boolean | null | undefined>,
  ) => string,
): string {
  const key = DIAGNOSTIC_KEYS[diagnostic.code];
  return key
    ? translate(key, { path: diagnostic.path ?? "" })
    : diagnostic.message;
}

export function regoDiagnostics(
  source: string,
  result: RegoValidationResult | null,
  translate?: Parameters<typeof regoDiagnosticMessage>[1],
): RegoDiagnostic[] {
  if (!result || result.valid || result.unavailable) return [];
  return (result.diagnostics ?? []).flatMap((diagnostic) => {
    const { path } = diagnostic;
    const message = translate
      ? regoDiagnosticMessage(diagnostic, translate)
      : diagnostic.message;
    if (diagnostic.line && diagnostic.column) {
      const line = regoSourceLine(source, diagnostic.line);
      const column = diagnostic.column;
      if (line > 0 && line <= source.split("\n").length && column > 0)
        return [{ line, column, message }];
      return [];
    }
    if (!path) return [];
    const index = source.indexOf(path);
    if (
      index < 0 ||
      source.indexOf(path, index + 1) >= 0 ||
      !regoCodeAt(source, index)
    )
      return [];
    const before = source.slice(0, index);
    return [
      {
        line: before.split("\n").length,
        column: index - before.lastIndexOf("\n"),
        message,
      },
    ];
  });
}

function diagnosticRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Save and validate mutations share the same structured diagnostic boundary. */
export function graphQlRuleValidationResult(
  error: unknown,
  message: string,
): RegoValidationResult {
  const diagnostics: RuleValidationDiagnostic[] = [];
  if (diagnosticRecord(error) && Array.isArray(error.graphQLErrors)) {
    for (const entry of error.graphQLErrors) {
      if (
        !diagnosticRecord(entry) ||
        !diagnosticRecord(entry.extensions) ||
        entry.extensions.code !== "VALIDATION_ERROR"
      )
        continue;
      const values = entry.extensions.diagnostics;
      if (Array.isArray(values)) {
        for (const value of values) {
          if (
            !diagnosticRecord(value) ||
            typeof value.code !== "string" ||
            typeof value.message !== "string"
          )
            continue;
          const coordinate = (v: unknown) =>
            typeof v === "number" && Number.isSafeInteger(v) && v > 0
              ? v
              : null;
          diagnostics.push({
            code: value.code,
            message: value.message,
            line: coordinate(value.line),
            column: coordinate(value.column),
            path: typeof value.path === "string" ? value.path : null,
          });
        }
      }
      if (
        !Array.isArray(values) &&
        typeof entry.extensions.reason === "string"
      ) {
        diagnostics.push({ code: entry.extensions.reason, message });
      }
    }
  }
  return { valid: false, errors: [message], diagnostics };
}
