import { isDeepStrictEqual } from "node:util";
import type { Body, Expr, Module, Term } from "rego-deparser";

type Env = Map<string, unknown>;
type Result = { value: unknown; env: Env };
const UNBOUND = Symbol("unbound");

/** Test interpreter for the emitted AST subset, not a Rego validator/runtime.
 * Evaluate dependencies completely before negation; throw on unknown constructs.
 */
export function evaluate(module: Module, input: unknown, ruleName: string): unknown {
  const cache = new Map<string, unknown>();
  const active = new Set<string>();
  const rules = module.rules ?? [];
  const bound = (value: unknown): boolean => value !== UNBOUND && (!Array.isArray(value) || value.every(bound));

  function query(name: string): unknown {
    if (cache.has(name)) return cache.get(name);
    if (active.has(name)) throw new Error("Recursive test rule: " + name);
    const selected = rules.filter((rule) => rule.head?.name === name);
    if (!selected.length) throw new Error("Unknown test rule: " + name);
    active.add(name);
    let result: unknown = selected.some((rule) => rule.head?.key) ? new Map<string, unknown>() : undefined;
    for (const rule of selected) for (const env of body(rule.body ?? [], new Map())) {
      const value = rule.head?.value ? values(rule.head.value, env)[0]?.value : true;
      if (rule.head?.key) {
        for (const key of values(rule.head.key, env)) {
          if (!bound(key.value)) throw new Error("Unbound generated head key");
          (result as Map<string, unknown>).set(JSON.stringify(key.value), value);
        }
      } else result = value;
    }
    active.delete(name);
    cache.set(name, result);
    return result;
  }

  function bind(pattern: Term, value: unknown, env: Env): Env[] {
    if (pattern.type === "var") {
      const name = String(pattern.value);
      if (name === "_") return [env];
      if (env.has(name)) return isDeepStrictEqual(env.get(name), value) ? [env] : [];
      return [new Map(env).set(name, value)];
    }
    if (pattern.type === "array") {
      const parts = pattern.value as Term[];
      if (!Array.isArray(value) || parts.length !== value.length) return [];
      return parts.reduce((states, part, i) => states.flatMap((state) => bind(part, value[i], state)), [env]);
    }
    return values(pattern, env).filter((item) => isDeepStrictEqual(item.value, value)).map((item) => item.env);
  }

  function values(term: Term, env: Env): Result[] {
    if (["string", "number", "boolean", "null"].includes(term.type ?? "")) return [{ value: term.value, env }];
    if (term.type === "var") {
      const name = String(term.value);
      const value = name === "input" ? input : env.has(name) ? env.get(name) : rules.some((rule) => rule.head?.name === name) ? query(name) : UNBOUND;
      return value === undefined ? [] : [{ value, env }];
    }
    if (term.type === "array" || term.type === "set") {
      const items = (term.value as Term[]).reduce<Result[]>((states, part) => states.flatMap((state) => values(part, state.env).map((next) => ({ value: [...state.value as unknown[], next.value], env: next.env }))), [{ value: [], env }]);
      return term.type === "set" ? items.map((item) => ({ ...item, value: new Set(item.value as unknown[]) })) : items;
    }
    if (term.type === "arraycomprehension") {
      const comp = term.value as { term: Term; body: Body };
      return [{ value: body(comp.body, new Map(env)).flatMap((state) => values(comp.term, state).map((item) => item.value)), env }];
    }
    if (term.type === "ref") {
      const parts = term.value as Term[];
      return parts.slice(1).reduce<Result[]>((states, index) => states.flatMap((state) => {
        const object = state.value;
        const entries: Array<[unknown, unknown]> = object instanceof Map
          ? [...object].map(([key, value]) => [JSON.parse(key), value])
          : object instanceof Set ? [...object].map((value) => [value, value])
          : Array.isArray(object) ? object.map((value, key) => [key, value])
          : object && typeof object === "object" ? Object.entries(object) : [];
        return entries.flatMap(([key, value]) => bind(index, key, state.env).map((env) => ({ value, env })));
      }), values(parts[0]!, env));
    }
    if (term.type === "call") return invoke(term.value as Term[], env);
    throw new Error("Unsupported test term: " + term.type);
  }

  function invoke(parts: Term[], env: Env): Result[] {
    const name = parts[0]?.type === "var" ? String(parts[0].value) : (parts[0]!.value as Term[]).map((part) => String(part.value)).join(".");
    const combinations = parts.slice(1).reduce<Result[]>((states, part) => states.flatMap((state) => values(part, state.env).map((item) => ({ value: [...state.value as unknown[], item.value], env: item.env }))), [{ value: [], env }]);
    return combinations.flatMap(({ value, env }) => {
      const args = value as unknown[];
      if (!args.every(bound)) throw new Error("Unbound test builtin: " + name);
      let result: unknown;
      switch (name) {
        case "count": result = typeof args[0] === "string" ? [...args[0]].length : Array.isArray(args[0]) ? args[0].length : undefined; break;
        case "minus": result = Number(args[0]) - Number(args[1]); break;
        case "plus": result = Number(args[0]) + Number(args[1]); break;
        case "numbers.range": {
          const start = Number(args[0]); const end = Number(args[1]);
          result = Array.from({ length: Math.abs(end - start) + 1 }, (_, i) => start + i * (start <= end ? 1 : -1)); break;
        }
        case "substring": result = typeof args[0] === "string" ? [...args[0]].slice(Number(args[1]), Number(args[2]) < 0 ? undefined : Number(args[1]) + Number(args[2])).join("") : undefined; break;
        case "is_array": result = Array.isArray(args[0]); break;
        case "is_string": result = typeof args[0] === "string"; break;
        case "is_number": result = typeof args[0] === "number"; break;
        case "lower": result = typeof args[0] === "string" ? args[0].toLowerCase() : undefined; break;
        case "object.get": result = args[0] && typeof args[0] === "object" ? (args[0] as Record<string, unknown>)[String(args[1])] ?? args[2] : undefined; break;
        case "regex.match": {
          if (typeof args[0] !== "string" || typeof args[1] !== "string") break;
          let pattern = args[0]; let flags = "u";
          if (pattern.startsWith("(?i)")) { flags += "i"; pattern = pattern.slice(4); }
          // Test fixtures use JS-compatible syntax; preserve absolute endpoints.
          pattern = pattern.replaceAll("\\A", "^").replaceAll("\\z", "$(?![\\s\\S])");
          result = new RegExp(pattern, flags).test(args[1]); break;
        }
        default: throw new Error("Unsupported test builtin: " + name);
      }
      return result === undefined ? [] : [{ value: result, env }];
    });
  }

  function expression(expr: Expr, env: Env): Env[] {
    if (expr.negated) return expression({ ...expr, negated: false }, new Map(env)).length ? [] : [env];
    if (!Array.isArray(expr.terms)) return values(expr.terms!, env).filter((item) => item.value !== false && item.value !== UNBOUND).map((item) => item.env);
    const [operator, left, right] = expr.terms;
    if (operator?.type === "var") {
      const name = String(operator.value);
      if (["assign", "eq", "equal"].includes(name)) return values(right!, env).flatMap((item) => bind(left!, item.value, item.env));
      if (["gt", "gte", "lt", "lte", "neq"].includes(name)) return values(left!, env).flatMap((l) => values(right!, l.env).filter((r) => {
        if (name === "neq") return !isDeepStrictEqual(l.value, r.value);
        if (typeof l.value !== "number" || typeof r.value !== "number") throw new Error("Non-numeric test comparison");
        return name === "gt" ? l.value > r.value : name === "gte" ? l.value >= r.value : name === "lt" ? l.value < r.value : l.value <= r.value;
      }).map((r) => r.env));
    }
    return invoke(expr.terms, env).filter((item) => item.value === true).map((item) => item.env);
  }

  function body(expressions: Body, env: Env): Env[] {
    return expressions.reduce((states, expr) => states.flatMap((state) => expression(expr, state)), [env]);
  }
  return query(ruleName) ?? false;
}
