/** The plugin's stable item key retains the provider library, including anime movies. */
export function simklItemUrl(itemKey: string): string | null {
  const match = /^simkl:(movie|show|anime):([1-9][0-9]*)$/.exec(itemKey);
  if (!match) return null;
  const section =
    match[1] === "movie" ? "movies" : match[1] === "show" ? "tv" : "anime";
  return `https://simkl.com/${section}/${match[2]}/`;
}
