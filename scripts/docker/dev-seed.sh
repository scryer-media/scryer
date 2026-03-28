#!/usr/bin/env sh
set -eu

SCRYER_URL="${SCRYER_URL:-http://localhost:8080}"
SEED_FILE="${1:-/seed.json}"
GRAPHQL_URL="$SCRYER_URL/graphql"

if [ ! -f "$SEED_FILE" ]; then
  echo "seed: no seed file found at $SEED_FILE — skipping"
  exit 0
fi

echo "seed: waiting for scryer at $SCRYER_URL ..."
attempts=0
max_attempts=60
while [ "$attempts" -lt "$max_attempts" ]; do
  HEALTH=$(curl -sf "$SCRYER_URL/health" 2>/dev/null) || true
  if printf '%s' "$HEALTH" | jq -er '.status == "ok"' >/dev/null 2>&1; then
    echo "seed: scryer is ready"
    break
  fi
  attempts=$((attempts + 1))
  sleep 2
done

if [ "$attempts" -ge "$max_attempts" ]; then
  echo "seed: scryer did not become healthy after ${max_attempts} attempts" >&2
  exit 1
fi

CLIENT_ALIAS_FILE=$(mktemp)
ENTRIES_FILE=$(mktemp)
trap 'rm -f "$CLIENT_ALIAS_FILE" "$ENTRIES_FILE"' EXIT
printf '{}' > "$CLIENT_ALIAS_FILE"

slugify() {
  printf '%s' "$1" \
    | tr '[:upper:]' '[:lower:]' \
    | sed -e 's/[^a-z0-9][^a-z0-9]*/-/g' -e 's/^-//' -e 's/-$//'
}

graphql_request() {
  query="$1"
  variables_json="${2-}"
  if [ -z "$variables_json" ]; then
    variables_json='{}'
  fi
  payload=$(jq -nc \
    --arg query "$query" \
    --argjson variables "$variables_json" \
    '{query: $query, variables: $variables}')

  response=$(curl -sf -X POST "$GRAPHQL_URL" \
    -H "Content-Type: application/json" \
    -d "$payload" 2>&1) || {
      echo "seed: GraphQL request failed" >&2
      echo "$response" >&2
      return 1
    }

  errors=$(printf '%s' "$response" | jq -r '.errors // [] | length')
  if [ "$errors" -gt 0 ]; then
    echo "seed: GraphQL request returned $errors errors" >&2
    printf '%s' "$response" | jq -r '.errors[] | "  - \(.message)"' >&2
    return 1
  fi

  printf '%s' "$response"
}

batch_reset() {
  BATCH_VAR_DEFS=""
  BATCH_FIELDS=""
  BATCH_VARIABLES='{}'
  BATCH_COUNT=0
}

batch_add() {
  input_type="$1"
  field_name="$2"
  selection="$3"
  input_json="$4"

  alias_name="op$BATCH_COUNT"
  variable_name="input$BATCH_COUNT"

  if [ -n "$BATCH_VAR_DEFS" ]; then
    BATCH_VAR_DEFS="$BATCH_VAR_DEFS, "
    BATCH_FIELDS="$BATCH_FIELDS "
  fi

  BATCH_VAR_DEFS="${BATCH_VAR_DEFS}\$$variable_name: $input_type!"
  BATCH_FIELDS="${BATCH_FIELDS}${alias_name}: ${field_name}(input: \$$variable_name) ${selection}"
  BATCH_VARIABLES=$(printf '%s' "$BATCH_VARIABLES" | jq -c \
    --arg key "$variable_name" \
    --argjson value "$input_json" \
    '. + {($key): $value}')
  BATCH_COUNT=$((BATCH_COUNT + 1))
}

batch_execute() {
  batch_label="$1"

  if [ "$BATCH_COUNT" -eq 0 ]; then
    printf '%s' '{"data":{}}'
    return 0
  fi

  echo "seed: sending batched $batch_label request ($BATCH_COUNT operations)" >&2
  graphql_request \
    "mutation SeedBatch($BATCH_VAR_DEFS) { $BATCH_FIELDS }" \
    "$BATCH_VARIABLES"
}

add_client_alias() {
  alias_key="$1"
  client_id="$2"

  if [ -z "$alias_key" ]; then
    return 0
  fi

  tmp=$(mktemp)
  jq --arg key "$alias_key" --arg value "$client_id" \
    '. + {($key): $value}' "$CLIENT_ALIAS_FILE" > "$tmp"
  mv "$tmp" "$CLIENT_ALIAS_FILE"
}

resolve_setting_value() {
  entry_json="$1"
  printf '%s' "$entry_json" | jq -r --slurpfile aliases "$CLIENT_ALIAS_FILE" '
    def serialize_setting_value:
      if has("valueJson") then
        .valueJson | tojson
      elif (.value | type) == "string" then
        .value
      else
        .value | tojson
      end;

    if .key == "download_client.routing" then
      (
        if has("valueJson") then
          .valueJson
        else
          .value | fromjson
        end
      )
      | with_entries(.key = ($aliases[0][.key] // .key))
      | tojson
    else
      serialize_setting_value
    end
  ' | tr -d '\n'
}

count_array() {
  jq "$1 | length" "$SEED_FILE"
}

seed_indexers() {
  jq -c '.indexers // [] | .[]' "$SEED_FILE" > "$ENTRIES_FILE"
  batch_reset

  while IFS= read -r entry_json; do
    [ -n "$entry_json" ] || continue

    name=$(printf '%s' "$entry_json" | jq -r '.name')
    input_json=$(printf '%s' "$entry_json" | jq -c '
      {
        name: .name,
        providerType: .providerType,
        baseUrl: .baseUrl,
        apiKey: (.apiKey // null),
        rateLimitSeconds: (if has("rateLimitSeconds") then .rateLimitSeconds else null end),
        rateLimitBurst: (if has("rateLimitBurst") then .rateLimitBurst else null end),
        isEnabled: (
          if has("enabled") then .enabled
          elif has("isEnabled") then .isEnabled
          else null
          end
        ),
        enableInteractiveSearch: (
          if has("enableInteractiveSearch") then .enableInteractiveSearch
          else null
          end
        ),
        enableAutoSearch: (
          if has("enableAutoSearch") then .enableAutoSearch
          else null
          end
        ),
        configJson: (
          if has("config") then .config | tojson
          elif has("configJson") then .configJson
          else null
          end
        )
      }')

    echo "seed: creating indexer '$name'"
    batch_add \
      'CreateIndexerConfigInput' \
      'createIndexerConfig' \
      '{ id name }' \
      "$input_json"
  done < "$ENTRIES_FILE"

  batch_execute 'indexer create' >/dev/null
}

seed_download_clients() {
  jq -c '.downloadClients // [] | .[]' "$SEED_FILE" > "$ENTRIES_FILE"
  batch_reset

  while IFS= read -r entry_json; do
    [ -n "$entry_json" ] || continue

    name=$(printf '%s' "$entry_json" | jq -r '.name')
    input_json=$(printf '%s' "$entry_json" | jq -c '
      {
        name: .name,
        clientType: .clientType,
        configJson: (
          if has("config") then .config | tojson
          elif has("configJson") then .configJson
          else "{}"
          end
        ),
        isEnabled: (
          if has("enabled") then .enabled
          elif has("isEnabled") then .isEnabled
          else null
          end
        )
      }')

    echo "seed: creating download client '$name'"
    batch_add \
      'CreateDownloadClientConfigInput' \
      'createDownloadClientConfig' \
      '{ id name clientType }' \
      "$input_json"
  done < "$ENTRIES_FILE"

  response=$(batch_execute 'download client create')

  batch_index=0
  while IFS= read -r entry_json; do
    [ -n "$entry_json" ] || continue

    name=$(printf '%s' "$entry_json" | jq -r '.name')
    client_id=$(printf '%s' "$response" | jq -r --arg alias "op$batch_index" '.data[$alias].id')
    seed_id=$(printf '%s' "$entry_json" | jq -r '.seedId // ""')
    name_slug=$(slugify "$name")

    add_client_alias "$client_id" "$client_id"
    add_client_alias "$name" "$client_id"
    add_client_alias "$name_slug" "$client_id"
    add_client_alias "$seed_id" "$client_id"

    batch_index=$((batch_index + 1))
  done < "$ENTRIES_FILE"
}

seed_settings() {
  movie_path=""
  series_path=""
  anime_path=""

  jq -c '.settings // [] | .[]' "$SEED_FILE" > "$ENTRIES_FILE"
  batch_reset

  while IFS= read -r entry_json; do
    [ -n "$entry_json" ] || continue

    key=$(printf '%s' "$entry_json" | jq -r '.key')
    scope_id=$(printf '%s' "$entry_json" | jq -r '.scopeId // ""')

    case "$key" in
      movies.path)
        movie_path=$(printf '%s' "$entry_json" | jq -r '
          (.value // .valueJson // "") | if type == "string" then . else tostring end
        ')
        ;;
      series.path)
        series_path=$(printf '%s' "$entry_json" | jq -r '
          (.value // .valueJson // "") | if type == "string" then . else tostring end
        ')
        ;;
      anime.path)
        anime_path=$(printf '%s' "$entry_json" | jq -r '
          (.value // .valueJson // "") | if type == "string" then . else tostring end
        ')
        ;;
      download_client.routing)
        if [ -z "$scope_id" ]; then
          echo "seed: download_client.routing requires scopeId" >&2
          exit 1
        fi
        entries=$(printf '%s' "$entry_json" | jq -c --slurpfile aliases "$CLIENT_ALIAS_FILE" '
          (
            if has("valueJson") then
              .valueJson
            else
              .value | fromjson
            end
          )
          | with_entries(.key = ($aliases[0][.key] // .key))
          | to_entries
          | map({
              clientId: .key,
              enabled: (
                if .value | has("enabled") then .value.enabled
                elif .value | has("is_enabled") then .value.is_enabled
                elif .value | has("isEnabled") then .value.isEnabled
                else true
                end
              ),
              category: (.value.category // null),
              recentQueuePriority: (
                .value.recentQueuePriority
                // .value.recentPriority
                // .value.recent_priority
                // null
              ),
              olderQueuePriority: (
                .value.olderQueuePriority
                // .value.olderPriority
                // .value.older_priority
                // null
              ),
              removeCompleted: (
                .value.removeCompleted
                // .value.remove_completed
                // .value.removeComplete
                // false
              ),
              removeFailed: (
                .value.removeFailed
                // .value.remove_failed
                // .value.removeFailure
                // false
              )
            })
        ')
        variables=$(jq -nc \
          --arg scope "$scope_id" \
          --argjson entries "$entries" '
          { scope: $scope, entries: $entries }
        ')

        echo "seed: saving setting '$key' (system/$scope_id)"
        batch_add \
          'UpdateDownloadClientRoutingInput' \
          'updateDownloadClientRouting' \
          '{ clientId }' \
          "$variables"
        ;;
      *)
        echo "seed: unsupported typed setting '$key'" >&2
        exit 1
        ;;
    esac
  done < "$ENTRIES_FILE"

  if [ -n "$movie_path" ] || [ -n "$series_path" ] || [ -n "$anime_path" ]; then
    [ -n "$movie_path" ] || movie_path="/media/movies"
    [ -n "$series_path" ] || series_path="/media/series"

    variables=$(jq -nc \
      --arg moviePath "$movie_path" \
      --arg seriesPath "$series_path" \
      --arg animePath "$anime_path" '
      {
        moviePath: $moviePath,
        seriesPath: $seriesPath,
        animePath: (if $animePath == "" then null else $animePath end)
      }')

    echo "seed: saving media library paths"
    batch_add \
      'UpdateLibraryPathsInput' \
      'updateLibraryPaths' \
      '{ moviePath seriesPath animePath }' \
      "$variables"
  fi

  batch_execute 'settings update' >/dev/null
}

seed_titles_for_facet() {
  collection_path="$1"
  facet="$2"
  label="$3"

  jq -c "$collection_path // [] | .[]" "$SEED_FILE" > "$ENTRIES_FILE"
  batch_reset

  while IFS= read -r entry_json; do
    [ -n "$entry_json" ] || continue

    name=$(printf '%s' "$entry_json" | jq -r '.name')
    input_json=$(printf '%s' "$entry_json" | jq -c --arg facet "$facet" '
      {
        name: .name,
        facet: $facet,
        monitored: (if has("monitored") then .monitored else false end),
        tags: (.tags // []),
        options: (if has("options") then .options else null end),
        externalIds: (
          (
            if has("externalIds") then .externalIds
            else []
            end
          ) + (
            if has("tvdbId") then
              [{ source: "tvdb", value: (.tvdbId | tostring) }]
            else
              []
            end
          )
          | unique_by(.source + ":" + .value)
        ),
        sourceHint: (if has("sourceHint") then .sourceHint else null end),
        sourceKind: (if has("sourceKind") then .sourceKind else null end),
        sourceTitle: (if has("sourceTitle") then .sourceTitle else null end),
        minAvailability: (if has("minAvailability") then .minAvailability else null end),
        posterUrl: (if has("posterUrl") then .posterUrl else null end),
        year: (if has("year") then .year else null end),
        overview: (if has("overview") then .overview else null end),
        sortTitle: (if has("sortTitle") then .sortTitle else null end),
        slug: (if has("slug") then .slug else null end),
        runtimeMinutes: (if has("runtimeMinutes") then .runtimeMinutes else null end),
        language: (if has("language") then .language else null end),
        contentStatus: (if has("contentStatus") then .contentStatus else null end)
      }')

    echo "seed: adding $label title '$name'"
    batch_add \
      'AddTitleInput' \
      'addTitle' \
      '{ title { id name facet } }' \
      "$input_json"
  done < "$ENTRIES_FILE"

  batch_execute "$label title add" >/dev/null
}

n_idx=$(count_array '.indexers // []')
n_dc=$(count_array '.downloadClients // []')
n_settings=$(count_array '.settings // []')
n_movies=$(count_array '.titles.movies // []')
n_series=$(count_array '.titles.series // []')
n_anime=$(count_array '.titles.anime // []')
n_total=$((n_idx + n_dc + n_settings + n_movies + n_series + n_anime))

echo "seed: applying $n_total operations from $(basename "$SEED_FILE")"

seed_indexers
seed_download_clients
seed_settings
seed_titles_for_facet '.titles.movies' 'movie' 'movie'
seed_titles_for_facet '.titles.series' 'tv' 'series'
seed_titles_for_facet '.titles.anime' 'anime' 'anime'

echo "seed: completed successfully ($n_total entities seeded)"
