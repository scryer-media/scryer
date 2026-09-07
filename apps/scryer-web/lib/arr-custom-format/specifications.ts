import type { Body, Term } from "rego-deparser";
import { call, callTerm, compare, comprehension, literal, member, ref, variable } from "./ast.ts";
import type { ArrSpecification, Diagnostic, ImportedCustomFormat } from "./types.ts";

export type LeafCompilation = { body: Body; alternatives?: Body[]; diagnostics: Diagnostic[]; handlesNegation?: boolean };

// Arr's language IDs are distinct from ISO codes. Regional variants without a
// distinct Scryer representation are deliberately absent (e.g. 30 and 37).
const LANGUAGES: Record<number, string> = {
  0: "und", 1: "eng", 2: "fra", 3: "spa", 4: "deu", 5: "ita", 6: "dan", 7: "nld",
  8: "jpn", 9: "isl", 10: "zho", 11: "rus", 12: "pol", 13: "vie", 14: "swe",
  15: "nor", 16: "fin", 17: "tur", 18: "por", 20: "ell", 21: "kor", 22: "hun",
  23: "heb", 24: "lit", 25: "ces", 26: "hin", 27: "ron", 28: "tha", 29: "bul",
  31: "ara", 32: "ukr", 33: "fas", 34: "ben", 35: "slk", 36: "lav", 38: "cat",
  39: "hrv", 40: "srp", 41: "bos", 42: "est", 43: "tam", 44: "ind", 45: "tel",
  46: "mkd", 47: "slv", 48: "mal", 49: "kan", 50: "sqi", 51: "afr", 52: "mar",
  53: "tgl", 54: "urd", 55: "roh", 56: "mon", 57: "kat",
};

const FIELDS: Record<string, string[]> = {
  ResolutionSpecification: ["value"], SourceSpecification: ["value"],
  QualityModifierSpecification: ["value"], ReleaseTypeSpecification: ["value"],
  IndexerFlagSpecification: ["value"], LanguageSpecification: ["value", "exceptLanguage"],
  SizeSpecification: ["min", "max"], YearSpecification: ["min", "max"],
};

function release(field: string): Term { return ref("input", "release", field); }
function numeric(value: unknown): value is number { return typeof value === "number" && Number.isFinite(value); }
function integer(value: unknown): value is number { return numeric(value) && Number.isSafeInteger(value); }

// Convert.ToInt64(double) in Arr uses round-to-even after converting GiB to bytes.
function gibBytes(value: number): number {
  const bytes = value * 1024 ** 3;
  const floor = Math.floor(bytes);
  return bytes - floor === 0.5 ? floor + floor % 2 : Math.round(bytes);
}

export function compileLeafSpecification(spec: ArrSpecification, format: ImportedCustomFormat): LeafCompilation | null {
  if (["ReleaseTitleSpecification", "ReleaseGroupSpecification", "EditionSpecification"].includes(spec.implementation)) return null;
  const fail = (code: string, message: string): LeafCompilation => ({ body: [], diagnostics: [{
    code, message, level: "error", formatId: format.id, specificationIndex: spec.index,
    location: `specifications[${spec.index}].fields`,
  }] });
  const ok = (body: Body, handlesNegation = false): LeafCompilation => ({ body, diagnostics: [], handlesNegation });
  const fields = FIELDS[spec.implementation];
  if (!fields) return fail("arr.implementation.unsupported", `Unsupported Arr implementation ${spec.implementation}.`);
  if (Object.keys(spec.fields).some((key) => !fields.includes(key))) return fail("arr.fields.unsupported", `Unrecognized field in ${spec.implementation}; the format is disabled to preserve its conditions.`);
  const value = spec.fields.value;
  if (fields.includes("value") && !integer(value)) return fail("arr.value.invalid", `${spec.implementation} requires an integer value.`);

  switch (spec.implementation) {
    case "ResolutionSpecification": {
      if (![0, 480, 576, 720, 1080, 2160].includes(value as number)) return fail("arr.resolution.unmapped", `Unsupported resolution ${String(value)}.`);
      if (value === 0) return ok([compare("equal", release("quality"), literal(null))]);
      // Scryer exposes upper-case resolution tokens; both scan types have the
      // same Arr resolution (1080i and 1080p are both 1080).
      return ok([member(callTerm("lower", release("quality")), [`${value}p`, `${value}i`])]);
    }
    case "SourceSpecification": {
      const normalized = callTerm("scryer.normalize_source", release("source"));
      if (value === 0) return ok([compare("equal", release("source"), literal(null))]);
      if (format.source === "radarr") {
        const map: Record<number, string[]> = { 1: ["CAM"], 2: ["TELESYNC"], 3: ["TELECINE"], 4: ["WORKPRINT"], 5: ["DVD", "DVDSCR"], 6: ["HDTV"], 7: ["WEB-DL"], 8: ["WEBRIP"], 9: ["BLURAY", "BRDISK"] };
        return map[value as number] ? ok([member(normalized, map[value as number])]) : fail("arr.source.unmapped", `Unsupported Radarr source ${String(value)}.`);
      }
      const map: Record<number, string[]> = { 1: ["HDTV"], 3: ["WEB-DL"], 4: ["WEBRIP"], 5: ["DVD"], 6: ["BLURAY"], 7: ["BLURAY"] };
      if (!map[value as number]) return fail("arr.source.unmapped", `Sonarr source ${String(value)} has no distinct Scryer input.`);
      const body = [member(normalized, map[value as number])];
      if (value === 6 || value === 7) body.push(compare("equal", release("is_remux"), literal(value === 7)));
      // Scryer folds RAWHD into HDTV; the original token distinguishes the
      // explicit raw-TV form when testing ordinary Television source.
      if (value === 1) body.push({ ...call("regex.match", literal("(?i)\\bRAW[ ._-]?HD\\b"), release("raw_title")), negated: true });
      return ok(body);
    }
    case "QualityModifierSpecification": {
      if (format.source !== "radarr") return fail("arr.quality_modifier.unmapped", "Quality modifiers are Radarr specifications.");
      if (value === 5) return ok([compare("equal", release("is_remux"), literal(true))]);
      if (value === 4) return ok([compare("equal", release("is_bd_disk"), literal(true))]);
      if (value === 2) return ok([compare("equal", callTerm("scryer.normalize_source", release("source")), literal("DVDSCR"))]);
      return fail("arr.quality_modifier.unmapped", `Quality modifier ${String(value)} cannot be distinguished reliably from existing inputs.`);
    }
    case "ReleaseTypeSpecification": {
      if (format.source !== "sonarr") return fail("arr.release_type.unmapped", "Release types are Sonarr specifications.");
      const types: Record<number, string> = { 0: "unknown", 1: "single_episode", 2: "multi_episode", 3: "season_pack" };
      const mapped = types[value as number];
      return mapped ? ok([compare("equal", release("episode_release_type"), literal(mapped))]) : fail("arr.release_type.unmapped", `Unsupported release type ${String(value)}.`);
    }
    case "IndexerFlagSpecification": {
      const extra = ref("input", "release", "extra");
      const download = callTerm("object.get", extra, literal("downloadvolumefactor"), literal(-1));
      const upload = callTerm("object.get", extra, literal("uploadvolumefactor"), literal(-1));
      if (value === 1) {
        const free = callTerm("object.get", extra, literal("freeleech"), literal(false));
        // Either Torznab's explicit flag or its normalized volume factor.
        return {
          ...ok([compare("equal", free, literal(true))]),
          alternatives: [[compare("equal", download, literal(0))]],
        };
      }
      if (value === 2) return ok([compare("equal", download, literal(0.5))]);
      if (value === 4) return ok([compare("equal", upload, literal(2))]);
      const internal = format.source === "sonarr" ? 8 : 32;
      const scene = format.source === "sonarr" ? 16 : 128;
      const factor75 = format.source === "sonarr" ? 32 : 256;
      const factor25 = format.source === "sonarr" ? 64 : 512;
      if (value === factor75) return ok([compare("equal", download, literal(0.75))]);
      if (value === factor25) return ok([compare("equal", download, literal(0.25))]);
      if (value === internal || value === scene) return ok([compare("equal", ref("input", "release", "extra", "indexer_flags", variable("_")), literal(value === internal ? "internal" : "scene"))]);
      return fail("arr.indexer_flag.unmapped", `Indexer flag ${String(value)} has no documented Scryer mapping.`);
    }
    case "LanguageSpecification": {
      if (spec.fields.exceptLanguage !== undefined && typeof spec.fields.exceptLanguage !== "boolean") return fail("arr.language.invalid", "exceptLanguage must be a boolean.");
      const mapped = value === -2 ? null : LANGUAGES[value as number];
      if (mapped === undefined) return fail("arr.language.unmapped", `Language ${String(value)} has no distinct Scryer language mapping.`);
      const target = mapped === null ? ref("input", "context", "original_language") : literal(mapped);
      const language = variable("_arr_language");
      const match = call("scryer.lang_matches", language, target);
      const matches = comprehension(language, [
        compare("assign", language, ref("input", "release", "languages_audio", variable("_"))),
        { ...match, negated: spec.fields.exceptLanguage === true },
      ]);
      return ok([
        call("is_array", release("languages_audio")),
        compare(spec.negate ? "equal" : "gt", callTerm("count", matches), literal(0)),
      ], true);
    }
    case "YearSpecification":
      return fail("arr.year.unmapped", "Radarr year conditions require a movie-metadata fallback that is unavailable in the rule input.");
    case "SizeSpecification": {
      const { min, max } = spec.fields;
      if (!numeric(min) || !numeric(max) || min < 0 || max <= min) return fail("arr.range.invalid", "The condition needs a valid minimum and maximum range.");
      const lower = gibBytes(min);
      const upper = gibBytes(max);
      if (!Number.isSafeInteger(lower) || !Number.isSafeInteger(upper)) return fail("arr.range.unsafe", "Range exceeds the exact numeric precision supported by this translator.");
      const input = release("size_bytes");
      return ok([call("is_number", input), compare("gt", input, literal(lower)), compare("lte", input, literal(upper))]);
    }
    default: return fail("arr.implementation.unsupported", `Unsupported Arr implementation ${spec.implementation}.`);
  }
}
