import { ruleBuilderOperator as op, type RuleBuilderKind } from "./rule-builder.ts";

/// The comparisons every family builds its kinds from. No comparison is
/// satisfied by a field that is missing or `null` unless its label asks about
/// that ("has no value"): presence is asked with `!= null`, because a bare
/// `{path}` is true for `null` in Rego, and an ordering or a negative
/// comparison first checks the value has the type it compares, because Rego
/// orders `null` below every number and a negation is true of a missing
/// value.
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
      op("NOT_CONTAINS", "text", "is_array({path})\nnot {value} in {path}"),
      op("IS_EMPTY", "none", "count({path}) == 0"),
      op("IS_NOT_EMPTY", "none", "count({path}) > 0"),
    ],
  },
  /// A list the input leaves out when it is empty, so a missing list is an
  /// empty one: it contains nothing and is empty.
  omitted_when_empty_string_list: {
    operators: [
      op("CONTAINS", "text", "{value} in {path}"),
      op("NOT_CONTAINS", "text", "not {value} in {path}"),
      op("IS_EMPTY", "none", "not count({path}) > 0"),
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
