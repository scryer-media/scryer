#!/bin/sh
# Run one GraphQL operation against Scryer.
#
#   gql.sh <operation-file.graphql> [variables-json-file] > out.json
#
# Reads SCRYER_URL (no trailing slash) and SCRYER_API_KEY from the environment.
# The key is handed to curl on stdin, so it never appears in argv, logs or output.
#
# stdout: the full JSON response body, whatever happened.
# stderr: one line per GraphQL error: "<code> [<reason>] <message>", plus any
#         rule diagnostics as "  line L col C <code>: <message>".
# Exit status: 0 success, 1 the response carried errors, 2 transport or
#              non-JSON response, 64 usage or environment problem.

set -eu

usage() {
  echo "usage: gql.sh <operation-file.graphql> [variables-json-file]" >&2
  exit 64
}

[ $# -ge 1 ] && [ $# -le 2 ] || usage
op_file=$1
vars_file=${2:-}

[ -r "$op_file" ] || { echo "gql.sh: cannot read operation file: $op_file" >&2; exit 64; }
[ -n "${SCRYER_URL:-}" ] || { echo "gql.sh: SCRYER_URL is not set" >&2; exit 64; }
[ -n "${SCRYER_API_KEY:-}" ] || { echo "gql.sh: SCRYER_API_KEY is not set" >&2; exit 64; }
command -v curl >/dev/null 2>&1 || { echo "gql.sh: curl is required" >&2; exit 64; }
command -v jq >/dev/null 2>&1 || { echo "gql.sh: jq is required" >&2; exit 64; }

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/scryer-gql.XXXXXX")
trap 'rm -rf "$tmp_dir"' EXIT HUP INT TERM
body=$tmp_dir/body.json
resp=$tmp_dir/response.json

if [ -n "$vars_file" ]; then
  [ -r "$vars_file" ] || { echo "gql.sh: cannot read variables file: $vars_file" >&2; exit 64; }
  jq -e 'type == "object"' "$vars_file" >/dev/null 2>&1 \
    || { echo "gql.sh: variables file must hold one JSON object: $vars_file" >&2; exit 64; }
  jq -n --rawfile query "$op_file" --slurpfile vars "$vars_file" \
    '{query: $query, variables: $vars[0]}' > "$body"
else
  jq -n --rawfile query "$op_file" '{query: $query, variables: {}}' > "$body"
fi

url=${SCRYER_URL%/}/graphql

# The Authorization header goes through a curl config read from stdin.
set +e
http_status=$(printf 'header = "Authorization: Bearer %s"\n' "$SCRYER_API_KEY" \
  | curl --silent --show-error --config - \
      --header 'Content-Type: application/json' \
      --header 'Accept: application/json' \
      --data-binary "@$body" \
      --output "$resp" \
      --write-out '%{http_code}' \
      "$url")
curl_status=$?
set -e

if [ "$curl_status" -ne 0 ]; then
  echo "gql.sh: request to $url failed (curl exit $curl_status)" >&2
  exit 2
fi

if ! jq -e 'type == "object"' "$resp" >/dev/null 2>&1; then
  echo "gql.sh: HTTP $http_status with a non-JSON body (first 200 bytes follow)" >&2
  head -c 200 "$resp" >&2 || true
  echo >&2
  exit 2
fi

cat "$resp"

if jq -e '(.errors // []) | length > 0' "$resp" >/dev/null; then
  jq -r '
    .errors[]
    | (.extensions // {}) as $x
    | "\($x.code // "NO_CODE")"
      + (if $x.reason then " [\($x.reason)]" else "" end)
      + (if $x.refusalCode then " [\($x.refusalCode)]" else "" end)
      + " \(.message)",
      ( ($x.diagnostics // [])[]
        | "  line \(.line // "-") col \(.column // "-") \(.code): \(.message)" )
  ' "$resp" >&2
  echo "gql.sh: HTTP $http_status" >&2
  exit 1
fi

if [ "$http_status" -ge 400 ]; then
  # Rate limiting and other HTTP-layer refusals answer {"error": "..."}.
  echo "gql.sh: HTTP $http_status $(jq -r '.error // "without GraphQL errors"' "$resp")" >&2
  exit 2
fi

exit 0
