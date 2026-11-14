import { ruleBuilderOperator as op, type RuleBuilderKind } from "./rule-builder.ts";

/// Kinds for rule families whose input sends an unknown value as `null`
/// rather than leaving the field out. A bare `{path}` is true for `null` in
/// Rego, so presence is asked with `!= null`, and an ordering or a negative
/// comparison first checks the value has the type it compares, so an unknown
/// value never satisfies one.
export const NULL_AWARE_RULE_BUILDER_KINDS: Readonly<
  Record<string, RuleBuilderKind>
> = {
  bool: {
    operators: [
      op("IS_TRUE", "none", "{path} == true"),
      op("IS_FALSE", "none", "{path} == false"),
    ],
  },
  number: {
    operators: [
      op("EQUALS", "number", "{path} == {value}"),
      op("NOT_EQUALS", "number", "{path} != {value}"),
      op("GREATER_THAN", "number", "{path} > {value}"),
      op("GREATER_OR_EQUAL", "number", "{path} >= {value}"),
      op("LESS_THAN", "number", "{path} < {value}"),
      op("LESS_OR_EQUAL", "number", "{path} <= {value}"),
    ],
  },
  nullable_number: {
    operators: [
      op("EQUALS", "number", "{path} == {value}"),
      op("NOT_EQUALS", "number", "is_number({path})\n{path} != {value}"),
      op("GREATER_THAN", "number", "is_number({path})\n{path} > {value}"),
      op("GREATER_OR_EQUAL", "number", "is_number({path})\n{path} >= {value}"),
      op("LESS_THAN", "number", "is_number({path})\n{path} < {value}"),
      op("LESS_OR_EQUAL", "number", "is_number({path})\n{path} <= {value}"),
      op("HAS_VALUE", "none", "{path} != null"),
      op("HAS_NO_VALUE", "none", "not {path} != null"),
    ],
  },
  string: {
    operators: [
      op("EQUALS", "text", "{path} == {value}"),
      op("NOT_EQUALS", "text", "{path} != {value}"),
    ],
  },
  nullable_string: {
    operators: [
      op("EQUALS", "text", "{path} == {value}"),
      op("NOT_EQUALS", "text", "is_string({path})\n{path} != {value}"),
      op("HAS_VALUE", "none", "{path} != null"),
      op("HAS_NO_VALUE", "none", "not {path} != null"),
    ],
  },
  choice: {
    operators: [
      op("EQUALS", "choice", "{path} == {value}"),
      op("NOT_EQUALS", "choice", "{path} != {value}"),
    ],
  },
  string_list: {
    operators: [
      op("CONTAINS", "text", "{value} in {path}"),
      op("NOT_CONTAINS", "text", "not {value} in {path}"),
      op("IS_EMPTY", "none", "count({path}) == 0"),
      op("IS_NOT_EMPTY", "none", "count({path}) > 0"),
    ],
  },
  nullable_string_list: {
    operators: [
      op("CONTAINS", "text", "is_array({path})\n{value} in {path}"),
      op("NOT_CONTAINS", "text", "is_array({path})\nnot {value} in {path}"),
      op("IS_EMPTY", "none", "{path} == []"),
      op("IS_NOT_EMPTY", "none", "is_array({path})\ncount({path}) > 0"),
      op("HAS_VALUE", "none", "{path} != null"),
      op("HAS_NO_VALUE", "none", "not {path} != null"),
    ],
  },
};
