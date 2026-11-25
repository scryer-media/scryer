# Token discipline

Libraries hold thousands of titles, tens of thousands of episodes. Keep bulk
data on disk; read only what a decision needs.

## Rules

1. **Every response to a file.** `gql.sh op.graphql vars.json > out.json`,
   then summarise with `jq`. Never `cat` a response.
2. **Counts first.** `jq '.data.titles.totalCount' out.json`,
   `jq '.data.X | length'`. Look at items only when a count looks wrong.
3. **Sample, don't list.** A handful: `jq -c '.data.titles.items[:5][] | {id, name}'`.
4. **Only mismatches reach the conversation.** In a verification loop, write
   every result to a TSV or JSON file; print only failing rows and a total.
   Cap at 20 rows and say how many more.
5. **Use the shipped operations.** Each selects only the fields its step
   needs. No fields "just in case"; never fetch or read the GraphQL schema.
   Missing a field: add that one field to a copy of the file.
6. **Let the server do the work.** Use its preview, validate, translate and
   test operations; do not reimplement their logic locally.
7. **Poll sparingly.** Long jobs (import warm-up, scans) report progress.
   Poll every 5 to 15 seconds; print one short line per poll, or nothing until
   done.
8. **Page with limits.** List queries take `limit` and `offset`. Page into
   files (e.g. 200 items per page), merge with `jq -s`.
9. **Keep secrets out of files you read back.** Arr snapshots can hold host
   names and user names. Keep them in a mode-700 directory. Never print them
   whole.

## jq patterns

```sh
jq -r '.errors[]?.extensions.code' out.json             # error codes only
jq -r '.data.titles.totalCount' out.json                # one number
jq -r '[.data.libraries[] | {id, facet, name}]' out.json  # small table
jq -s 'map(.data.titles.items) | add | length' page-*.json  # merge pages
```
