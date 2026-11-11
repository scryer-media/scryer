#!/bin/sh
# Turn the stored config of one indexer or download client into a
# [ProviderConfigValueInput] array, ready to edit and send back.
#
# Usage: config-to-input.sh <list-response.json> <id> > config.json
#   <list-response.json> is the saved output of ListIndexers.graphql or
#   ListDownloadClients.graphql.
#
# Secret keys are left out on purpose: an update keeps stored secrets for
# omitted secret keys. Non-secret keys are all included, because an update's
# config replaces every non-secret key. Keys whose stored value the API cannot
# express (lists or objects) are named on stderr; sending the array would
# drop them, so stop and tell the operator.
set -eu

resp=${1:?usage: config-to-input.sh <list-response.json> <id>}
id=${2:?usage: config-to-input.sh <list-response.json> <id>}

item=$(jq -c --arg id "$id" '
  ((.data.indexers // .data.downloadClientConfigs // [])[] | select(.id == $id))
' "$resp")
if [ -z "$item" ]; then
  echo "no indexer or download client with id $id in $resp" >&2
  exit 1
fi

lost=$(printf '%s' "$item" | jq -r '[.config[] | select(.value == null) | .key] | join(", ")')
if [ -n "$lost" ]; then
  echo "warning: keys with no representable value (would be dropped): $lost" >&2
fi

printf '%s' "$item" | jq '[
  .config[]
  | select(.value != null and .value.__typename != "SecretConfigValuePayload")
  | {key} + (.value | del(.__typename))
]'
