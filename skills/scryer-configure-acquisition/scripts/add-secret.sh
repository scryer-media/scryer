#!/bin/sh
# Add or replace one secret value in a [ProviderConfigValueInput] array
# without the secret passing through the command line or stdout.
#
# Usage:
#   add-secret.sh <config-in.json> <key> <config-out.json> --file <path>
#   add-secret.sh <config-in.json> <key> <config-out.json> --env <VARIABLE>
#
#   --file reads the secret from a file the operator wrote (one trailing
#          newline is removed).
#   --env  reads it from an environment variable the operator exported.
#
# <config-out.json> is written with mode 600, replacing any existing file and
# whatever mode it had. It contains the secret: keep it in a private directory
# and tell the operator where it is. Prints only the key name and the output
# path.
set -eu

in=${1:?usage: add-secret.sh <config-in.json> <key> <config-out.json> --file <path> | --env <VARIABLE>}
key=${2:?missing key}
out=${3:?missing output path}
mode=${4:?missing --file or --env}
src=${5:?missing secret source}

umask 077
# Written to a new private file and renamed into place, so an existing output
# file never keeps a wider mode and is never left half written.
tmp=$(mktemp "$out.XXXXXX")
trap 'rm -f "$tmp"' EXIT
case "$mode" in
  --file)
    [ -s "$src" ] || { echo "secret file is missing or empty" >&2; exit 2; }
    jq --arg k "$key" --rawfile s "$src" \
      '[.[] | select(.key != $k)] + [{key: $k, secretValue: ($s | sub("\r?\n$"; ""))}]' \
      "$in" > "$tmp"
    ;;
  --env)
    printenv "$src" >/dev/null 2>&1 || { echo "environment variable $src is not set" >&2; exit 2; }
    jq --arg k "$key" --arg n "$src" \
      'if ($ENV[$n] // "") == "" then error("environment variable is empty") else . end
       | [.[] | select(.key != $k)] + [{key: $k, secretValue: $ENV[$n]}]' \
      "$in" > "$tmp"
    ;;
  *)
    echo "fourth argument must be --file or --env" >&2
    exit 2
    ;;
esac
chmod 600 "$tmp"
mv -f "$tmp" "$out"
trap - EXIT

echo "secret for key '$key' written to $out"
