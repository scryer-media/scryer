# Discovering a provider's config fields

Indexers and download clients are plugins; the plugin declares their
settings. Never guess field keys; read them.

## 1. List the providers

```sh
sh "$G" "$S"/operations/IndexerProviderTypes.graphql > idx-types.json
jq -r '.data.indexerProviderTypes[] | "\(.providerType)\t\(.name)\ttest=\(.supportsTest)\tdynamic=\(.dynamicConfigTriggerKeys | length)"' idx-types.json
```

Clients: `DownloadClientProviderTypes.graphql`,
`.data.downloadClientProviderTypes`.

## 2. Read one provider's fields

```sh
jq --arg t newznab '.data.indexerProviderTypes[] | select(.providerType == $t)
  | {defaultBaseUrl, dynamicConfigTriggerKeys,
     fields: [.configFields[] | {key, fieldType, required, defaultValue, role, valueSource, advanced,
       options: [.options[].value], visibleWhen, requiredWhen}]}' idx-types.json
```

(`newznab` is an example; use the type from step 1.)

- `key`: key to send in `ProviderConfigValueInput.key`.
- `fieldType`: STRING, MULTILINE, PATH, TAG, SELECT, FILTERED_SELECT →
  `stringValue`; PASSWORD → `secretValue`; BOOL → `boolValue`; NUMBER →
  `intValue` or `floatValue`; INFO is display text, never send it.
- `required`: must have a value.
- `defaultValue`: used when the operator gives none; send it explicitly on
  create.
- `valueSource`: USER = operator supplies it. HOST_BINDING = Scryer fills it
  from `hostBinding`; do not ask for it.
- `role`: CONNECTION_URL marks the base URL field (at most one per provider).
  LOGIN_CHALLENGE_ANSWER marks the field taking a challenge answer.
- `options[]`: allowed values for SELECT; an option's `configOverrides` sets
  other keys when chosen; apply them too.
- `visibleWhen` / `requiredWhen`: `{key, op, values}`, op EQ, NE, IN, NOT_IN,
  NON_EMPTY: field applies, or becomes required, only when that other key
  matches.
- `advanced`: hidden by default in the UI; leave at default unless the
  operator asks.
- `helpText`: show it to the operator when asking for the value.

Ask only for USER fields that are required, or required by `requiredWhen`
given the other answers, plus any optional field the operator mentions.

## 3. Dynamic fields

Non-empty `dynamicConfigTriggerKeys`: field list depends on those keys'
values (e.g. site or account type). After the operator sets a trigger key:

```sh
jq -n --arg t "$TYPE" --slurpfile v config.json \
  '{providerType: $t, values: $v[0], indexerId: null, proxyConfigId: null}' > fields-vars.json
sh "$G" "$S"/operations/PluginConfigFields.graphql fields-vars.json > fields.json
jq '{hidden: .data.pluginConfigFields.hiddenFieldKeys,
     challenge: (.data.pluginConfigFields.loginChallenge | if . then {answerFieldKey, contentType} else null end),
     fields: [.data.pluginConfigFields.fields[] | {key, fieldType, required, defaultValue}]}' fields.json
```

Some providers contact their service to build the list; say so before
calling with real credentials. Drop keys in `hiddenFieldKeys`.

`loginChallenge` = service wants a human answer (e.g. an image). Image is in
`imageData`; do not read it into the conversation. Tell the operator a
challenge is needed and the UI under **Integrations > Indexers** shows it;
manual step.

## 4. Build the config array

`config.json` is a `[ProviderConfigValueInput]` array. Each entry
`{"key": "...", "<one of>": value}` with exactly one of `stringValue`,
`boolValue`, `intValue`, `floatValue`, `secretValue`, or `"clearSecret": true`.
No value: entry ignored. Two values: refused with
`INTERFACE_CONFIG_VALUE_AMBIGUOUS`.

Non-secret values directly with jq:

```sh
jq -n '[{key: "base_url", stringValue: "https://indexer.example"}, {key: "use_ssl", boolValue: true}]' > config.json
```

Secrets never via command line or conversation:

```sh
sh "$S"/scripts/add-secret.sh config.json api_key config.json.new --file ./indexer-key.txt
```

Operator writes the secret file (or exports a variable and uses
`--env NAME`). Keep secret-bearing files in a private directory; tell the
operator where they are when done.
