import { deparse, TermType } from "rego-deparser";
import type { Body, Expr, Module, Rule, Term } from "rego-deparser";
import { generate } from "oniguruma-parser/generator";
import { parse } from "oniguruma-parser/parser";

import { compileLeafSpecification } from "./specifications.ts";

import type {
  ArrSpecification,
  Diagnostic,
  ImportedCustomFormat,
  TranslationResult,
  TranslatedFormat,
} from "./types";

export type { TranslationResult, TranslatedFormat } from "./types";

const MAX_REGEX_LENGTH = 8_192;
const MAX_SPECIFICATIONS = 1_000;

type CompiledSpecification = {
  body: Body;
  alternatives?: Body[];
  rules?: Rule[];
  diagnostics: Diagnostic[];
  handlesNegation?: boolean;
};

function term(type: string, value: unknown): Term {
  return { type, value: value as never };
}
function text(value: string): Term { return term(TermType.STRING, value); }
function number(value: number): Term { return term(TermType.NUMBER, value); }
function bool(value: boolean): Term { return term(TermType.BOOLEAN, value); }
function variable(value: string): Term { return term(TermType.VAR, value); }
function reference(...path: string[]): Term {
  return term(TermType.REF, path.map((part, index) => index === 0 ? variable(part) : text(part)));
}
function referenceTerms(...path: Term[]): Term { return term(TermType.REF, path); }
function array(...values: Term[]): Term { return term(TermType.ARRAY, values); }
function call(name: string[], ...args: Term[]): Expr {
  return { terms: [reference(...name), ...args] };
}
function compare(operator: "eq" | "gte" | "lte", left: Term, right: Term): Expr {
  return { terms: [variable(operator), left, right] };
}
function predicate(name: string, negated = false): Expr {
  return { terms: variable(name), negated };
}
function diagnostic(
  code: string,
  message: string,
  formatId: string,
  specificationIndex?: number,
): Diagnostic {
  return { code, level: "error", message, formatId, specificationIndex };
}
function identifier(value: string): string {
  const normalized = value.toLowerCase().replaceAll(/[^a-z0-9_]+/g, "_").replaceAll(/^_+|_+$/g, "");
  return normalized && /^[a-z_]/.test(normalized) ? normalized : "imported_format";
}
function specificationName(formatIndex: number, specIndex: number): string {
  return `_arr_${formatIndex}_spec_${specIndex}`;
}
function groupName(formatIndex: number, groupIndex: number): string {
  return `_arr_${formatIndex}_group_${groupIndex}`;
}
function formatName(formatIndex: number): string { return `_arr_${formatIndex}_matches`; }

/** The parser is the structural gate; RE2-incompatible features never leak into generated source. */
export function compileRegex(value: unknown, input: Term, formatId: string, spec: ArrSpecification, relationPrefix: string): CompiledSpecification {
  if (typeof value !== "string" || !value) {
    return { body: [], rules: [], diagnostics: [diagnostic("regex.value_invalid", "Regex specifications need a non-empty string value.", formatId, spec.index)] };
  }
  if ([...value].some((character) => character.charCodeAt(0) < 32 || character.charCodeAt(0) === 127)) {
    return { body: [], rules: [], diagnostics: [diagnostic("regex.control_character", "Regex control characters cannot be safely emitted by the Rego deparser.", formatId, spec.index)] };
  }
  if (value.length > MAX_REGEX_LENGTH) {
    return { body: [], rules: [], diagnostics: [diagnostic("regex.too_large", `Regex exceeds the ${MAX_REGEX_LENGTH}-character import limit.`, formatId, spec.index)] };
  }
  // Backreferences require capture state, which position relations do not model.
  if (/\\(?:[1-9]|k<)/.test(value)) {
    return { body: [], rules: [], diagnostics: [diagnostic("regex.backreference_unsupported", "Backreferences cannot be represented by Rego's regex engine.", formatId, spec.index)] };
  }
  if (/\(\?>|(?:\*|\+|\?|\{\d+(?:,\d*)?\})\+|\\K|\(\?[x-]/.test(value)) {
    return { body: [], rules: [], diagnostics: [diagnostic("regex.re2_incompatible", "This .NET regex construct has no equivalent in Rego's regex engine.", formatId, spec.index)] };
  }
  try {
    // Arr accepts variable-width .NET lookbehind. The standard convenience
    // parser rejects it using Oniguruma's narrower validation, so retain the
    // AST with that validation disabled and lower its position semantics here.
    const ast = parse(value, { skipLookbehindValidation: true });
    checkRegexCapabilities(ast);
    if (containsLookaround(ast)) {
      if (hasQuantifiedAssertion(ast)) {
        return { body: [], rules: [], diagnostics: [diagnostic("regex.quantified_lookaround_unsupported", "A quantified positional assertion cannot be lowered without unbounded expansion.", formatId, spec.index)] };
      }
      const lowered = lowerPositionalRegex(ast as Record<string, unknown>, input, relationPrefix);
      return { body: [{ terms: referenceTerms(variable(lowered.name), array(variable("_"), variable("_"))) }], rules: lowered.rules, diagnostics: [] };
    }
  } catch (error) {
    const detail = error instanceof Error ? error.message : "unknown regex parser error";
    return { body: [], rules: [], diagnostics: [diagnostic("regex.parse_failed", `Regex could not be parsed: ${detail}`, formatId, spec.index)] };
  }
  // Arr's CustomFormat regexes use IgnoreCase unless a specification carries
  // an explicit case-sensitive option (not present in its JSON export shape).
  return { body: [call(["regex", "match"], text(`(?i)${value}`), input)], rules: [], diagnostics: [] };
}

function checkRegexCapabilities(value: unknown): void {
  if (!value || typeof value !== "object") return;
  if (Array.isArray(value)) { value.forEach(checkRegexCapabilities); return; }
  const node = value as Record<string, unknown>;
  if (node.type === "Backreference" || node.type === "Subroutine" || node.type === "AbsentFunction" || node.type === "Callout") throw new Error(`Unsupported regex node ${node.type}`);
  if (node.type === "Assertion" && !["word_boundary", "line_start", "line_end", "string_start", "string_end", "string_end_newline"].includes(String(node.kind))) throw new Error(`Unsupported assertion ${String(node.kind)}`);
  if (node.type === "CharacterSet" && !["dot", "digit", "space", "word"].includes(String(node.kind))) throw new Error(`Unsupported character set ${String(node.kind)}`);
  if (node.type === "CharacterClass" && node.kind !== "union") throw new Error("Character-class operations are unsupported");
  if (node.atomic === true || node.kind === "possessive") throw new Error("Atomic regex matching is unsupported");
  // The Arr default is already case insensitive. Other inline modes have
  // different meanings in .NET and Oniguruma, so never infer their semantics.
  if (node.type === "Flags" && Object.entries(node).some(([key, enabled]) => enabled && !["type", "ignoreCase"].includes(key))) throw new Error("Unsupported regex flags");
  if (node.enable && Object.keys(node.enable as object).some((key) => key !== "ignoreCase")) throw new Error("Unsupported inline regex flags");
  if (node.disable && Object.keys(node.disable as object).length) throw new Error("Disabling regex flags is unsupported");
  Object.values(node).forEach(checkRegexCapabilities);
}

function containsAssertion(value: unknown): boolean {
  if (!value || typeof value !== "object") return false;
  if (Array.isArray(value)) return value.some(containsAssertion);
  const node = value as Record<string, unknown>;
  return node.type === "Assertion" || node.type === "LookaroundAssertion" || Object.values(node).some(containsAssertion);
}

function containsLookaround(value: unknown): boolean {
  if (!value || typeof value !== "object") return false;
  if (Array.isArray(value)) return value.some(containsLookaround);
  const record = value as Record<string, unknown>;
  if (record.type === "LookaroundAssertion") return true;
  return Object.values(record).some(containsLookaround);
}

function hasQuantifiedAssertion(value: unknown): boolean {
  if (!value || typeof value !== "object") return false;
  if (Array.isArray(value)) return value.some(hasQuantifiedAssertion);
  const record = value as Record<string, unknown>;
  if (record.type === "Quantifier" && containsAssertion(record.body)) return true;
  return Object.values(record).some(hasQuantifiedAssertion);
}

type LoweredRelation = { name: string; rules: Rule[] };

/** Fixed widths avoid enumerating every substring for literal/group fragments. */
function fixedWidth(value: unknown): number | undefined {
  if (Array.isArray(value)) {
    const widths = value.map(fixedWidth);
    return widths.every((width) => width !== undefined) ? widths.reduce((sum, width) => sum + width, 0) : undefined;
  }
  if (!value || typeof value !== "object") return undefined;
  const node = value as Record<string, unknown>;
  if (["Character", "CharacterSet", "CharacterClass"].includes(String(node.type))) return 1;
  if (node.type === "Directive") return 0;
  if (node.type === "Alternative") return fixedWidth(node.body);
  if (node.type === "Group" || node.type === "CapturingGroup") {
    const widths = (node.body as unknown[]).map(fixedWidth);
    return widths.length && widths.every((width) => width === widths[0]) ? widths[0] : undefined;
  }
  if (node.type === "Quantifier" && node.min === node.max) {
    const width = fixedWidth(node.body);
    return width === undefined ? undefined : width * Number(node.min);
  }
  return undefined;
}

/**
 * Lower lookarounds to relations keyed by [start, end] offsets in the original
 * string. A lookahead is a zero-width relation whose child starts at that
 * offset; a lookbehind is one whose child ends there. This keeps local negative
 * assertions local, including when the same token occurs more than once.
 */
function lowerPositionalRegex(ast: Record<string, unknown>, input: Term, prefix: string): LoweredRelation {
  let next = 0;
  const makeName = () => `${prefix}_position_${next++}`;
  const rules: Rule[] = [];
  const key = (name: string, start: Term, end: Term): Expr => ({ terms: referenceTerms(variable(name), array(start, end)) });
  const fragment = (nodes: unknown[], flags: unknown): LoweredRelation => {
    const name = makeName();
    const generated = generate({ type: "Regex", body: [{ type: "Alternative", body: nodes }], flags } as never).pattern;
    const width = fixedWidth(nodes);
    const starts = `${name}_starts`;
    const ends = `${name}_ends`;
    const slice = `${name}_slice`;
    const length = `${name}_length`;
    rules.push({
      head: { name, key: array(variable(`${name}_start`), variable(`${name}_end`)), value: bool(true), assign: true },
      body: [
        { terms: [variable("assign"), variable(starts), callTerm(["numbers", "range"], number(0), callTerm(["count"], input))] },
        { terms: [variable("assign"), variable(`${name}_start`), referenceTerms(variable(starts), variable("_"))] },
        ...(width === undefined ? [
          { terms: [variable("assign"), variable(ends), callTerm(["numbers", "range"], variable(`${name}_start`), callTerm(["count"], input))] },
          { terms: [variable("assign"), variable(`${name}_end`), referenceTerms(variable(ends), variable("_"))] },
        ] : [
          { terms: [variable("assign"), variable(`${name}_end`), callTerm(["plus"], variable(`${name}_start`), number(width))] },
          compare("lte", variable(`${name}_end`), callTerm(["count"], input)),
        ]),
        { terms: [variable("assign"), variable(length), callTerm(["minus"], variable(`${name}_end`), variable(`${name}_start`))] },
        { terms: [variable("assign"), variable(slice), callTerm(["substring"], input, variable(`${name}_start`), variable(length))] },
        call(["regex", "match"], text(`(?i)\\A(?:${generated})\\z`), variable(slice)),
      ],
    });
    return { name, rules };
  };
  const sequence = (nodes: unknown[], flags: unknown): LoweredRelation => {
    const pieces: LoweredRelation[] = [];
    let regular: unknown[] = [];
    const flush = () => { if (regular.length) { pieces.push(fragment(regular, flags)); regular = []; } };
    for (const node of nodes) {
      if (node && typeof node === "object" && (node as Record<string, unknown>).type === "Assertion") {
        const assertion = node as Record<string, unknown>;
        const kind = assertion.kind;
        if (["word_boundary", "line_start", "string_start", "search_start", "line_end", "string_end", "string_end_newline"].includes(String(kind))) {
          flush();
          const name = makeName();
          const position = `${name}_at`;
          const positions = `${name}_positions`;
          const head = { name, key: array(variable(position), variable(position)), value: bool(true), assign: true };
          const enumerate: Body = [
            { terms: [variable("assign"), variable(positions), callTerm(["numbers", "range"], number(0), callTerm(["count"], input))] },
            { terms: [variable("assign"), variable(position), referenceTerms(variable(positions), variable("_"))] },
          ];
          if (kind === "word_boundary") {
            const prefix = variable(`${name}_prefix`);
            const suffix = variable(`${name}_suffix`);
            const context: Body = [
              ...enumerate,
              { terms: [variable("assign"), prefix, callTerm(["substring"], input, number(0), variable(position))] },
              { terms: [variable("assign"), suffix, callTerm(["substring"], input, variable(position), number(-1))] },
            ];
            const before = call(["regex", "match"], text("\\w\\z"), prefix);
            const after = call(["regex", "match"], text("\\A\\w"), suffix);
            // Use the target engine's word class at adjacent original offsets.
            // Boundary is XOR; non-boundary is equality, without recursion.
            rules.push({ head, body: [...context, before, { ...after, negated: assertion.negate !== true }] });
            rules.push({ head, body: [...context, { ...before, negated: true }, { ...after, negated: assertion.negate === true }] });
          } else {
            const atStart = kind === "line_start" || kind === "string_start" || kind === "search_start";
            const relation = { terms: [variable("equal"), variable(position), atStart ? number(0) : callTerm(["count"], input)] } as Expr;
            rules.push({ head, body: [...enumerate, assertion.negate === true ? { ...relation, negated: true } : relation] });
            // .NET `$` and `\Z` also match immediately before a final newline.
            if (kind === "line_end" || kind === "string_end_newline") {
              rules.push({ head, body: [...enumerate, compare("eq", callTerm(["substring"], input, variable(position), number(-1)), text("\n"))] });
            }
          }
          pieces.push({ name, rules });
          continue;
        }
      }
      if (node && typeof node === "object" && (node as Record<string, unknown>).type === "LookaroundAssertion") {
        flush();
        const assertion = node as Record<string, unknown>;
        const body = Array.isArray(assertion.body) ? assertion.body : [];
        const alternatives = body.map((alternative) => lowerAlternative(alternative as Record<string, unknown>, flags));
        const name = makeName();
        const position = `${name}_at`;
        const positions = `${name}_positions`;
        const at = (alternative: LoweredRelation): Expr => assertion.kind === "lookbehind"
          ? key(alternative.name, variable("_"), variable(position))
          : key(alternative.name, variable(position), variable("_"));
        const enumerate: Body = [
          { terms: [variable("assign"), variable(positions), callTerm(["numbers", "range"], number(0), callTerm(["count"], input))] },
          { terms: [variable("assign"), variable(position), referenceTerms(variable(positions), variable("_"))] },
        ];
        if (assertion.negate === true) {
          // `(?!a|b)` succeeds only where neither alternative matches.
          rules.push({
            head: { name, key: array(variable(position), variable(position)), value: bool(true), assign: true },
            body: [...enumerate, ...alternatives.map((alternative) => ({ ...at(alternative), negated: true }))],
          });
        } else {
          for (const alternative of alternatives) {
            rules.push({
              head: { name, key: array(variable(position), variable(position)), value: bool(true), assign: true },
              body: [...enumerate, at(alternative)],
            });
          }
        }
        pieces.push({ name, rules });
      } else if (containsAssertion(node) && node && typeof node === "object" && Array.isArray((node as Record<string, unknown>).body)) {
        // Noncapturing/capturing groups retain no observable capture state once
        // backreferences have been rejected, so their alternatives can share a
        // position relation directly.
        flush();
        const alternatives = ((node as Record<string, unknown>).body as unknown[])
          .map((alternative) => lowerAlternative(alternative as Record<string, unknown>, flags));
        const name = makeName();
        for (const alternative of alternatives) {
          rules.push({
            head: { name, key: array(variable(`${name}_start`), variable(`${name}_end`)), value: bool(true), assign: true },
            body: [key(alternative.name, variable(`${name}_start`), variable(`${name}_end`))],
          });
        }
        pieces.push({ name, rules });
      } else {
        regular.push(node);
      }
    }
    flush();
    if (pieces.length === 0) return fragment([], flags);
    if (pieces.length === 1) return pieces[0]!;
    const name = makeName();
    const starts = pieces.map((_, index) => variable(`${name}_p${index}`));
    const ends = pieces.map((_, index) => variable(`${name}_p${index + 1}`));
    rules.push({ head: { name, key: array(starts[0]!, ends.at(-1)!), value: bool(true), assign: true }, body: pieces.map((piece, index) => key(piece.name, starts[index]!, ends[index]!)) });
    return { name, rules };
  };
  const lowerAlternative = (alternative: Record<string, unknown>, flags: unknown): LoweredRelation => {
    const body = Array.isArray(alternative.body) ? alternative.body : [];
    return sequence(body, flags);
  };
  const alternatives = Array.isArray(ast.body) ? ast.body.map((item) => lowerAlternative(item as Record<string, unknown>, ast.flags)) : [];
  if (alternatives.length === 1) return { name: alternatives[0]!.name, rules };
  const name = makeName();
  for (const alternative of alternatives) {
    rules.push({ head: { name, key: array(variable(`${name}_start`), variable(`${name}_end`)), value: bool(true), assign: true }, body: [key(alternative.name, variable(`${name}_start`), variable(`${name}_end`))] });
  }
  return { name, rules };
}

function compileSpecification(spec: ArrSpecification, format: ImportedCustomFormat, relationPrefix: string): CompiledSpecification {
  const input = (field: string) => reference("input", "release", field);
  if (spec.implementation === "ReleaseTitleSpecification") {
    if (Object.keys(spec.fields).some((key) => key !== "value")) return { body: [], diagnostics: [diagnostic("arr.fields.unsupported", "Release title regex fields are not supported.", format.id, spec.index)] };
    const title = format.source === "radarr"
      ? callTerm(["regex", "replace"], input("raw_title"), text("\\s*(?:[<>?*|])"), text(""))
      : input("raw_title");
    return compileRegex(spec.fields.value, title, format.id, spec, relationPrefix);
  }
  if (spec.implementation === "ReleaseGroupSpecification") {
    if (Object.keys(spec.fields).some((key) => key !== "value")) return { body: [], diagnostics: [diagnostic("arr.fields.unsupported", "Release group regex fields are not supported.", format.id, spec.index)] };
    return compileRegex(spec.fields.value, input("release_group"), format.id, spec, relationPrefix);
  }
  if (spec.implementation === "EditionSpecification") {
    if (Object.keys(spec.fields).some((key) => key !== "value")) return { body: [], diagnostics: [diagnostic("arr.fields.unsupported", "Edition regex fields are not supported.", format.id, spec.index)] };
    if (format.source !== "radarr") return { body: [], diagnostics: [diagnostic("arr.edition.unmapped", "Edition specifications are Radarr-specific.", format.id, spec.index)] };
    return compileRegex(spec.fields.value, input("edition"), format.id, spec, relationPrefix);
  }
  return compileLeafSpecification(spec, format) ?? {
    body: [],
    diagnostics: [diagnostic("arr.implementation.unsupported", `Unsupported Arr implementation ${spec.implementation}.`, format.id, spec.index)],
  };
}

function callTerm(name: string[], ...args: Term[]): Term {
  return term(TermType.CALL, [reference(...name), ...args]);
}

function helperRule(name: string, body: Body): Rule {
  return { head: { name }, body };
}

function comments(lines: string[]): string {
  return lines.map((line) => `# ${line.replaceAll(/[\r\n]+/g, " ")}`).join("\n");
}

function scoreKey(format: ImportedCustomFormat, position: number): string {
  return `${identifier(format.name)}_${position + 1}`;
}

/**
 * Compiles normalized formats into Rego AST and deparses it. Every executable
 * expression below is represented as a `rego-deparser` AST node.
 */
export function translateCustomFormats(
  formats: ImportedCustomFormat[],
  scores: Record<string, number>,
): TranslationResult {
  const diagnostics: Diagnostic[] = [];
  const rules: Rule[] = [];
  const annotations: string[] = ["Generated from Arr custom formats. Review disabled entries before saving."];
  const translated: TranslatedFormat[] = [];
  let hasRadarr = false;
  let hasSonarr = false;

  for (const [formatIndex, format] of formats.entries()) {
    hasRadarr ||= format.source === "radarr";
    hasSonarr ||= format.source === "sonarr";
    const local: Diagnostic[] = [...(format.inspectionDiagnostics ?? [])];
    if (format.specifications.length > MAX_SPECIFICATIONS) {
      local.push(diagnostic("arr.format.too_large", `Format exceeds the ${MAX_SPECIFICATIONS}-specification import limit.`, format.id));
    }
    if (format.specifications.length === 0) local.push(diagnostic("arr.specifications.empty", "A format needs at least one valid condition.", format.id));
    const compiled = format.specifications.slice(0, MAX_SPECIFICATIONS).map((spec, position) => compileSpecification(spec, format, `_arr_${formatIndex}_spec_${position}`));
    for (const item of compiled) local.push(...item.diagnostics);
    const score = scores[format.id];
    const validScore = Number.isInteger(score) && score >= -2_147_483_648 && score <= 2_147_483_647;
    if (!validScore) local.push(diagnostic("arr.score.invalid", "Choose a signed 32-bit integer score before translating this format.", format.id));

    const disabled = local.some((item) => item.level === "error");
    const key = scoreKey(format, formatIndex);
    if (disabled) {
      annotations.push(`${format.name}: disabled (${local.map((item) => item.message).join("; ")})`);
      rules.push({ head: { name: "score_entry", key: text(key), value: number(validScore ? score : 0), assign: true }, body: [{ terms: bool(false) }] });
      translated.push({ id: format.id, name: format.name, status: "disabled", diagnostics: local });
      diagnostics.push(...local);
      continue;
    }

    const byImplementation = new Map<string, number[]>();
    for (const [position, spec] of format.specifications.entries()) {
      const grouped = byImplementation.get(spec.implementation) ?? [];
      grouped.push(position);
      byImplementation.set(spec.implementation, grouped);
      rules.push(...(compiled[position]!.rules ?? []));
      rules.push(helperRule(specificationName(formatIndex, position), compiled[position]!.body));
      for (const alternative of compiled[position]!.alternatives ?? []) {
        rules.push(helperRule(specificationName(formatIndex, position), alternative));
      }
    }
    let groupIndex = 0;
    for (const indexes of byImplementation.values()) {
      const required = indexes.filter((index) => format.specifications[index]!.required);
      const optional = indexes.filter((index) => !format.specifications[index]!.required);
      const match = (index: number): Expr => predicate(
        specificationName(formatIndex, index),
        format.specifications[index]!.negate && !compiled[index]!.handlesNegation,
      );
      // Arr's DidMatch requires every required spec and at least one spec in
      // the group. Any required match already satisfies the latter condition.
      if (required.length > 0) {
        rules.push(helperRule(groupName(formatIndex, groupIndex), required.map(match)));
      } else {
        for (const option of optional) {
          rules.push(helperRule(groupName(formatIndex, groupIndex), [
            ...required.map(match),
            match(option),
          ]));
        }
      }
      groupIndex += 1;
    }
    rules.push(helperRule(formatName(formatIndex), Array.from({ length: groupIndex }, (_, index) => predicate(groupName(formatIndex, index)))));
    rules.push({
      head: { name: "score_entry", key: text(key), value: number(score), assign: true },
      body: [predicate(formatName(formatIndex))],
    });
    translated.push({ id: format.id, name: format.name, status: "translated", diagnostics: [] });
  }

  const module: Module = {
    imports: [{ path: reference("rego", "v1") }],
    rules,
  };
  const rego = deparse(module, { indent: "  " });
  const appliedFacets = hasRadarr && hasSonarr ? ["movie", "series", "anime"] : hasRadarr ? ["movie"] : ["series", "anime"];
  return {
    regoSource: `${rego}\n\n${comments(annotations)}\n`,
    name: formats.length === 1 ? formats[0]!.name : "Imported Arr custom formats",
    description: "Generated from pasted Arr custom-format JSON.",
    appliedFacets,
    formats: translated,
    diagnostics,
  };
}
