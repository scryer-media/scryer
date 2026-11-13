#!/bin/sh
# Print only what a write would change, for the operator to approve.
#
# Usage: show-change.sh <before.json> <after.json>
#   <before.json>  the current state, extracted from a read: one indexer,
#                  one client, one delay profile,
#                  the quality profile settings, or a routing list.
#   <after.json>   the variables file for the write ({"input": {...}}), or
#                  the bare input object.
#
# Output lines: "path: before -> after". Config lists are compared by key and
# routing lists by indexer or client id. Because config and routing entries
# are full replacements, entries present before but missing after are shown
# as "(removed)"; omitted secret keys keep their stored value and are not
# listed. Other fields left out of the input are unchanged and not
# shown. Secret values are never printed: stored secrets show as
# <secret stored>, new ones as <new secret>.
set -eu

before=${1:?usage: show-change.sh <before.json> <after.json>}
after=${2:?usage: show-change.sh <before.json> <after.json>}

jq -n -r --slurpfile b "$before" --slurpfile a "$after" '
  def first_value: [.stringValue, .boolValue, .intValue, .floatValue] | map(select(. != null)) | .[0];
  def cfgval:
    if (.value | type) == "object" then
      (.value | if .__typename == "SecretConfigValuePayload"
                then (if .stored then "<secret stored>" else "<no secret>" end)
                else first_value end)
    elif .secretValue != null then "<new secret>"
    elif .clearSecret == true then "<clear secret>"
    else first_value end;
  def idkey: (.id // .indexerId // .clientId // .key // .scope) | tostring;
  def n:
    if type == "object" then
      (if (.config | type) == "array"
       then .config |= (map({key: .key, value: cfgval}) | from_entries)
       else . end)
      | with_entries(.value |= n)
    elif type == "array" then
      if length > 0 and all(.[]; type == "object")
      then (map({key: idkey, value: (. | n)}) | from_entries)
      else "\u0001" + tojson end
    else . end;
  def secretish: test("password|secret|token|api_?key|username|user_name"; "i");
  def show($p; $v):
    if ($v | type) == "string" and ($v | test("^<(secret stored|no secret|new secret|clear secret)>$")) then $v
    elif any($p[]; type == "string" and secretish) and $v != null then "<redacted>"
    elif ($v | type) == "string" and ($v | startswith("\u0001")) then ($v | ltrimstr("\u0001"))
    else ($v | tojson) end;
  def pathname($p): $p | map(tostring) | join(".");

  ($b[0] | if type == "array" then {entries: .} else . end | n) as $B
  | ($a[0] | (.input // .) | n) as $A
  | ([$A | paths(type != "object")] | map(select(. as $p | ($B | getpath($p)) != ($A | getpath($p))))) as $changed
  | (["config", "entries"] | map(. as $r
      | select(($A[$r] | type) == "object" and ($B[$r] | type) == "object")
      | ($B[$r] | keys[]) as $k
      | select(($A[$r] | has($k)) | not)
      | select($B[$r][$k] != "<secret stored>" and $B[$r][$k] != "<no secret>")
      | [$r, $k])) as $removed
  | if ($changed | length) == 0 and ($removed | length) == 0 then "no changes"
    else
      ($changed[] as $p | "\(pathname($p)): \(show($p; $B | getpath($p))) -> \(show($p; $A | getpath($p)))"),
      ($removed[] as $p | "\(pathname($p)): (removed)")
    end
'
