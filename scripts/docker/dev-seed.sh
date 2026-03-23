#!/usr/bin/env sh
set -eu

SCRYER_URL="${SCRYER_URL:-http://localhost:8080}"
SEED_FILE="${1:-/seed.json}"

if [ ! -f "$SEED_FILE" ]; then
  echo "seed: no seed file found at $SEED_FILE — skipping"
  exit 0
fi

# Wait for scryer to be healthy
echo "seed: waiting for scryer at $SCRYER_URL ..."
attempts=0
max_attempts=60
while [ "$attempts" -lt "$max_attempts" ]; do
  if curl -sf "$SCRYER_URL/health" >/dev/null 2>&1; then
    echo "seed: scryer is healthy"
    break
  fi
  attempts=$((attempts + 1))
  sleep 2
done

if [ "$attempts" -ge "$max_attempts" ]; then
  echo "seed: scryer did not become healthy after ${max_attempts} attempts" >&2
  exit 1
fi

# Build a single batched GraphQL mutation from the seed file and write
# the complete JSON payload to a temp file. This avoids shell variable
# size limits and quoting issues with large mutation strings.
PAYLOAD_FILE=$(mktemp)
trap 'rm -f "$PAYLOAD_FILE"' EXIT

jq '{
  "query": (
    def escape: gsub("\\\\"; "\\\\\\\\") | gsub("\""; "\\\"") | gsub("\n"; "\\n");

    # Indexers
    ([.indexers // [] | to_entries[] | .key as $i | .value |
      "idx\($i): createIndexerConfig(input: { name: \"\(.name | escape)\", providerType: \"\(.providerType)\", baseUrl: \"\(.baseUrl | escape)\", apiKey: \"\(.apiKey | escape)\", isEnabled: \(.enabled) }) { id name }"
    ] | join("\n")) as $indexers |

    # Download clients
    ([.downloadClients // [] | to_entries[] | .key as $i | .value |
      (.config | tojson | escape) as $configJson |
      "dc\($i): createDownloadClientConfig(input: { name: \"\(.name | escape)\", clientType: \"\(.clientType)\", baseUrl: \"\(.baseUrl | escape)\", configJson: \"\($configJson)\" }) { id name }"
    ] | join("\n")) as $clients |

    # Settings
    ([.settings // [] | to_entries[] | .key as $i | .value |
      (if .scopeId then ", scopeId: \"\(.scopeId)\"" else "" end) as $scopeId |
      "s\($i): saveAdminSettings(input: { scope: \"\(.scope)\"\($scopeId), items: [{ keyName: \"\(.key)\", value: \"\(.value | escape)\" }] }) { scope }"
    ] | join("\n")) as $settings |

    # Titles — movies
    ([.titles.movies // [] | to_entries[] | .key as $i | .value |
      "m\($i): addTitle(input: { name: \"\(.name | escape)\", facet: \"movie\", monitored: false, tags: [], externalIds: [{ source: \"tvdb\", value: \"\(.tvdbId)\" }] }) { title { id } }"
    ] | join("\n")) as $movies |

    # Titles — series
    ([.titles.series // [] | to_entries[] | .key as $i | .value |
      "se\($i): addTitle(input: { name: \"\(.name | escape)\", facet: \"series\", monitored: false, tags: [], externalIds: [{ source: \"tvdb\", value: \"\(.tvdbId)\" }] }) { title { id } }"
    ] | join("\n")) as $series |

    # Titles — anime
    ([.titles.anime // [] | to_entries[] | .key as $i | .value |
      "a\($i): addTitle(input: { name: \"\(.name | escape)\", facet: \"anime\", monitored: false, tags: [], externalIds: [{ source: \"tvdb\", value: \"\(.tvdbId)\" }] }) { title { id } }"
    ] | join("\n")) as $anime |

    "mutation Seed {\n\($indexers)\n\($clients)\n\($settings)\n\($movies)\n\($series)\n\($anime)\n}"
  )
}' "$SEED_FILE" > "$PAYLOAD_FILE"

# Count entities for logging
n_idx=$(jq '.indexers // [] | length' "$SEED_FILE")
n_dc=$(jq '.downloadClients // [] | length' "$SEED_FILE")
n_settings=$(jq '.settings // [] | length' "$SEED_FILE")
n_movies=$(jq '.titles.movies // [] | length' "$SEED_FILE")
n_series=$(jq '.titles.series // [] | length' "$SEED_FILE")
n_anime=$(jq '.titles.anime // [] | length' "$SEED_FILE")
n_total=$((n_idx + n_dc + n_settings + n_movies + n_series + n_anime))

echo "seed: sending batched mutation ($n_total operations: ${n_idx} indexers, ${n_dc} clients, ${n_settings} settings, ${n_movies} movies, ${n_series} series, ${n_anime} anime)"

RESPONSE=$(curl -sf -X POST "$SCRYER_URL/graphql" \
  -H "Content-Type: application/json" \
  -d @"$PAYLOAD_FILE" 2>&1) || {
  echo "seed: GraphQL request failed" >&2
  echo "$RESPONSE" >&2
  exit 1
}

# Check for errors in response
ERRORS=$(echo "$RESPONSE" | jq -r '.errors // [] | length')
if [ "$ERRORS" -gt 0 ]; then
  echo "seed: completed with $ERRORS errors:"
  echo "$RESPONSE" | jq -r '.errors[] | "  - \(.message)"'
else
  echo "seed: completed successfully ($n_total entities seeded)"
fi
