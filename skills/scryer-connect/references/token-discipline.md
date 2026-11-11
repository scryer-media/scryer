# Token discipline

A media library holds thousands of titles and tens of thousands of episodes.
Reading that data into the conversation costs far more than the work itself.
Keep bulk data on disk and read only what a decision needs.

## Rules

1. **Every response goes to a file.** Run `gql.sh op.graphql vars.json > out.json`,
   then read a summary with `jq`. Never `cat` a response.
2. **Read counts first.** `jq '.data.titles.totalCount' out.json`,
   `jq '.data.X | length'`. Look at items only when a count looks wrong.
3. **Sample, don't list.** When you must look at items, take a handful:
   `jq -c '.data.titles.items[:5][] | {id, name}'`.
4. **Only mismatches reach the conversation.** In a verification loop, write
   every result to a TSV or JSON file and print only the failing rows and a
   total. Cap what you print at 20 rows and say how many more there are.
5. **Use the shipped operations.** Each one selects only the fields its step
   needs. Do not add fields "just in case", and never fetch or read the
   GraphQL schema. If an operation is missing a field you need, add that one
   field to a copy of the file.
6. **Let the server do the work.** Use the server's preview, validate,
   translate and test operations rather than reimplementing their logic
   locally.
7. **Poll sparingly.** Long jobs (import warm-up, scans) report progress.
   Poll every 5 to 15 seconds and print one short line per poll, or nothing
   until the job finishes.
8. **Page with limits.** List queries take `limit` and `offset`. Page through
   into files, for example 200 items per page, and merge them with
   `jq -s`.
9. **Keep secrets out of files you read back.** Arr snapshots can hold host
   names and user names. Keep them in a mode-700 directory. Do not print them
   whole.

## Handy jq patterns

```sh
jq -r '.errors[]?.extensions.code' out.json             # error codes only
jq -r '.data.titles.totalCount' out.json                # one number
jq -r '[.data.libraries[] | {id, facet, name}]' out.json  # small table
jq -s 'map(.data.titles.items) | add | length' page-*.json  # merge pages
```
