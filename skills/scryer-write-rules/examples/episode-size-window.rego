package scryer.rules.user.example
import rego.v1

# Applies to: series, anime (save with appliedFacets ["series", "anime"]).
# Prefers single 1080p episodes between 1 and 4 GiB and penalises single
# episodes over 8 GiB. Size is unknown for some listings, so every rule
# that uses it first checks that size_bytes is a number.

size_gib := scryer.size_gib(input.release.size_bytes) if {
	is_number(input.release.size_bytes)
}

score_entry["episode_size_sweet_spot"] := 100 if {
	input.release.quality == "1080P"
	not input.release.is_season_pack
	size_gib >= 1
	size_gib <= 4
}

score_entry["episode_oversized"] := -200 if {
	not input.release.is_season_pack
	size_gib > 8
}
