package scryer.rules.user.example
import rego.v1

# Applies to: movie, series, anime.
# Strong penalty for hardcoded subtitles and AI-upscaled releases. The block
# score is recoverable: a release that also earns large bonuses elsewhere can
# still climb back above the rejection threshold, so keep other bonuses small.

score_entry["hardcoded_subs"] := scryer.block_score() if {
	input.release.is_hardcoded_subs
}

score_entry["ai_enhanced"] := scryer.block_score() if {
	input.release.is_ai_enhanced
}
