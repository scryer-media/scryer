package scryer.rules.user.example
import rego.v1

# Applies to: movie, series, anime.
# Small bonus for one trusted release group, small penalty for releases with no
# group, and a penalty for titles tagged "dubbed" that do not carry two audio
# languages. "ExampleGrp" is a placeholder: use the group the operator names.

score_entry["trusted_group"] := 150 if {
	group := input.release.release_group
	is_string(group)
	lower(group) == "examplegrp"
}

score_entry["no_release_group"] := -50 if {
	not input.release.has_release_group
}

score_entry["dubbed_without_dual_audio"] := -300 if {
	regex.match(`(?i)\bdubbed\b`, input.release.raw_title)
	not input.release.is_dual_audio
}
