# Discovering a provider's config fields

Indexers and download clients are plugins. Their settings are declared by
the plugin, not fixed by Scryer. Never guess field keys; read them.

## 1. List the providers

```sh
sh "$G" "$S"/operations/IndexerProviderTypes.graphql > idx-types.json
jq -r '.data.indexerProviderTypes[] | "\(.providerType)\t\(.name)\ttest=\(.supportsTest)\tdynamic=\(.dynamicConfigTriggerKeys | length)"' idx-types.json
```

Use `DownloadClientProviderTypes.graphql` and
`.data.downloadClientProviderTypes` for clients.

## 2. Read one provider's fields

```sh
jq --arg t newznab '.data.indexerProviderTypes[] | select(.providerType == $t)
  | {defaultBaseUrl, dynamicConfigTriggerKeys,
     fields: [.configFields[] | {key, fieldType, required, defaultValue, role, valueSource, advanced,
       options: [.options[].value], visibleWhen, requiredWhen}]}' idx-types.json
```

(`newznab` is an example provider type; use the one from step 1.)

| field property | meaning |
|---|---|
| `key` | the key to send in `ProviderConfigValueInput.key` |
| `fieldType` | STRING, MULTILINE, PATH, TAG, SELECT, FILTERED_SELECT → `stringValue`; PASSWORD → `secretValue`; BOOL → `boolValue`; NUMBER → `intValue` or `floatValue`; INFO is display text, never send it |
| `required` | must have a value |
| `defaultValue` | used when the operator gives none; send it explicitly on create |
| `valueSource` | USER: the operator supplies it. HOST_BINDING: Scryer fills it from `hostBinding`; do not ask for it |
| `role` | CONNECTION_URL marks the base URL field (at most one per provider). LOGIN_CHALLENGE_ANSWER marks the field that takes a challenge answer |
| `options[]` | allowed values for SELECT; an option's `configOverrides` sets other keys when that option is chosen, apply them too |
| `visibleWhen` / `requiredWhen` | `{key, op, values}` with op EQ, NE, IN, NOT_IN, NON_EMPTY: the field applies, or becomes required, only when that other key matches |
| `advanced` | hidden by default in the UI; leave at the default unless the operator asks |
| `helpText` | show it to the operator when asking for the value |

Ask only for USER fields that are required, or required by `requiredWhen`
given the other answers, plus any optional field the operator mentions.

## 3. Dynamic fields

When `dynamicConfigTriggerKeys` is non-empty, the field list depends on
the values of those keys (for example a site or account type). After the
operator sets a trigger key, call:

```sh
jq -n --arg t "$TYPE" --slurpfile v config.json \
  '{providerType: $t, values: $v[0], indexerId: null, proxyConfigId: null}' > fields-vars.json
sh "$G" "$S"/operations/PluginConfigFields.graphql fields-vars.json > fields.json
jq '{hidden: .data.pluginConfigFields.hiddenFieldKeys,
     challenge: (.data.pluginConfigFields.loginChallenge | if . then {answerFieldKey, contentType} else null end),
     fields: [.data.pluginConfigFields.fields[] | {key, fieldType, required, defaultValue}]}' fields.json
```

Some providers contact their service to build the list; say so before
calling it with real credentials. Drop keys listed in `hiddenFieldKeys`.

A `loginChallenge` means the service wants a human answer (for example an
image). The image is in `imageData`; do not read it into the conversation.
Tell the operator a challenge is needed and that the UI under
**Integrations > Indexers** shows it; this step is manual.

## 4. Build the config array

`config.json` is a `[ProviderConfigValueInput]` array. Each entry is
`{"key": "...", "<one of>": value}` with exactly one of `stringValue`,
`boolValue`, `intValue`, `floatValue`, `secretValue`, or `"clearSecret": true`.
An entry with no value is ignored; one with two values is refused with
`INTERFACE_CONFIG_VALUE_AMBIGUOUS`.

Non-secret values can be written with jq directly:

```sh
jq -n '[{key: "base_url", stringValue: "https://indexer.example"}, {key: "use_ssl", boolValue: true}]' > config.json
```

Secrets never go through the command line or the conversation:

```sh
sh "$S"/scripts/add-secret.sh config.json api_key config.json.new --file ./indexer-key.txt
```

The operator writes the secret file (or exports a variable and uses
`--env NAME`). Keep secret-bearing files in a private directory, and tell
the operator where they are when done.
