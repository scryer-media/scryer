import { TermType } from "rego-deparser";
import type { Body, Expr, Term } from "rego-deparser";

/** Constructors keep imported strings out of variable/operator positions. */
export function literal(value: string | number | boolean | null): Term {
  if (typeof value === "number" && !Number.isFinite(value)) throw new Error("Non-finite Rego number");
  return { type: value === null ? TermType.NULL : typeof value, value };
}

export function variable(name: string): Term {
  if (!/^[a-zA-Z_][a-zA-Z_0-9]*$/.test(name)) throw new Error("Invalid generated Rego variable");
  return { type: TermType.VAR, value: name };
}

export function ref(root: string, ...path: Array<string | Term>): Term {
  return { type: TermType.REF, value: [variable(root), ...path.map((part) => typeof part === "string" ? literal(part) : part)] };
}

export function callTerm(name: string, ...args: Term[]): Term {
  const [root, ...path] = name.split(".");
  return { type: TermType.CALL, value: [ref(root, ...path), ...args] };
}

export function call(name: string, ...args: Term[]): Expr {
  const [root, ...path] = name.split(".");
  return { terms: [ref(root, ...path), ...args] };
}

export function compare(operator: "equal" | "neq" | "lt" | "lte" | "gt" | "gte" | "assign", left: Term, right: Term): Expr {
  return { terms: [variable(operator), left, right] };
}

export function member(value: Term, choices: Array<string | number>): Expr {
  return { terms: { type: TermType.REF, value: [{ type: TermType.SET, value: choices.map(literal) }, value] } };
}

export function comprehension(value: Term, body: Body): Term {
  return { type: TermType.ARRAY_COMPREHENSION, value: { term: value, body } };
}
