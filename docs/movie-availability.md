# Movie Availability

Movie availability controls when automatic acquisition may grab a monitored movie. It does not replace monitoring, quality profiles, source restrictions, or other acquisition requirements.

## Thresholds

- **Announced**: available immediately.
- **In Cinemas**: available on the earliest theatrical release date for the configured market.
- **Released**: available on the earliest digital or physical home-release date for the configured market.

For **Released**, Scryer estimates theatrical date plus 90 days only when both digital and physical date lists are absent. If a home-release list exists but has no usable date, or no applicable date is known, availability stays unknown and automatic acquisition remains blocked. The signed day offset applies to the selected date; zero includes the effective date, negative values allow earlier acquisition, and positive values delay it.

The release market is an ISO-3166-1 alpha-2 country code. A market change refreshes movie metadata before dates from the new market can satisfy a threshold. Eligibility uses the current title metadata and acquisition settings, so revised dates and setting changes are reconsidered without removing and re-adding a movie.

## Existing Movies

The migration adds storage for regional release-date metadata without changing existing movie settings or file associations. Existing `min_availability = NULL` values are interpreted as **Announced**, preserving their prior immediate-acquisition behavior. Scryer does not backfill or rewrite existing monitored movies. The global default applies only to newly added movies; an individual movie can have its own minimum-availability setting.

Interactive search keeps blocked releases visible and explains the availability reason and effective date when known. Queueing one early requires a separate confirmation for that release; it does not change the movie's setting or allow automatic acquisition to bypass the gate.
