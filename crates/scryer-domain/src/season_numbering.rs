//! The season ordinal a community entry's own name states.
//!
//! Release groups number an anime by the community's seasons, and the bridge
//! records those as entries in air order. An entry's position is not its
//! season number: a split cour, a movie, or a placeholder entry each take a
//! position without starting a season. The entry's name, when it says
//! `Season 3`, `3rd Season`, `III` or `第3期`, is the evidence that does.

use std::sync::LazyLock;

use regex::Regex;
use unicode_normalization::UnicodeNormalization;

/// The highest season a name is believed to state. A larger number in a
/// season position is a year or a part of the name, not a season.
const MAX_SEASON_ORDINAL: i32 = 50;

const ENGLISH_ORDINAL_WORDS: [&str; 10] = [
    "first", "second", "third", "fourth", "fifth", "sixth", "seventh", "eighth", "ninth", "tenth",
];
const ENGLISH_CARDINAL_WORDS: [&str; 10] = [
    "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
];

/// The season ordinal one name states, or `None` when it states none or
/// states two different ones.
///
/// Read forms:
///
/// * `Season 3`, `Season Three`, `Season III`;
/// * `3rd Season`, `Third Season`, and a bare `2nd` that ends the name or
///   comes right before a colon or parenthesis;
/// * `S3` and `s3p1` as a token of their own;
/// * an upper-case Roman numeral `II`–`X` that ends the name or comes right
///   before a colon, parenthesis or spaced dash. `V` and `X` alone count only
///   after `Season`, since a single trailing letter is as often part of the
///   name. A numeral after `Part`, `Vol`, `Chapter` or `Cour` numbers that,
///   not a season;
/// * `第2期`, `2期`, `第二季`, `第2シーズン`.
///
/// `Part 2` alone is not a season. A name is folded with NFKC first, so
/// full-width digits and the Unicode Roman numeral characters read as their
/// ASCII spellings.
pub fn season_ordinal_from_title(title: &str) -> Option<i32> {
    static SEASON_NUMBER: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)\bseason\s+(\d{1,3})\b").expect("valid pattern"));
    static SEASON_WORD: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)\bseason\s+(one|two|three|four|five|six|seven|eight|nine|ten)\b")
            .expect("valid pattern")
    });
    static SEASON_ROMAN: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\b(?i:season)\s+(I|II|III|IV|V|VI|VII|VIII|IX|X)\b").expect("valid pattern")
    });
    static NUMBERED_ORDINAL_SEASON: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)\b(\d{1,3})(?:st|nd|rd|th)\s+season\b").expect("valid pattern")
    });
    static NUMBERED_ORDINAL_TAIL: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)\b(\d{1,3})(?:st|nd|rd|th)(?:\s*$|\s*[:(])").expect("valid pattern")
    });
    static WORD_ORDINAL_SEASON: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"(?i)\b(first|second|third|fourth|fifth|sixth|seventh|eighth|ninth|tenth)\s+season\b",
        )
        .expect("valid pattern")
    });
    static SEASON_TOKEN: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)\bs(\d{1,2})(?:p\d{1,2})?\b").expect("valid pattern"));
    static ROMAN_TAIL: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"(?:\b((?i:part|vol|volume|chapter|cour|season))\.?\s+)?\b(II|III|IV|VI|VII|VIII|IX)(?:\s*$|\s*[:(]|\s+[-–]\s)",
        )
        .expect("valid pattern")
    });
    static JAPANESE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"第\s*([0-9]{1,3}|[一二三四五六七八九十]{1,3})\s*(?:期|季|シーズン)|([0-9]{1,3})\s*期",
        )
        .expect("valid pattern")
    });

    let folded = title.nfkc().collect::<String>();
    let mut ordinals = Vec::new();

    for captures in SEASON_NUMBER.captures_iter(&folded) {
        ordinals.push(captures[1].parse::<i32>().ok());
    }
    for captures in SEASON_WORD.captures_iter(&folded) {
        ordinals.push(word_position(&ENGLISH_CARDINAL_WORDS, &captures[1]));
    }
    for captures in SEASON_ROMAN.captures_iter(&folded) {
        ordinals.push(roman_value(&captures[1]));
    }
    for captures in NUMBERED_ORDINAL_SEASON.captures_iter(&folded) {
        ordinals.push(captures[1].parse::<i32>().ok());
    }
    for captures in NUMBERED_ORDINAL_TAIL.captures_iter(&folded) {
        ordinals.push(captures[1].parse::<i32>().ok());
    }
    for captures in WORD_ORDINAL_SEASON.captures_iter(&folded) {
        ordinals.push(word_position(&ENGLISH_ORDINAL_WORDS, &captures[1]));
    }
    for captures in SEASON_TOKEN.captures_iter(&folded) {
        ordinals.push(captures[1].parse::<i32>().ok());
    }
    for captures in ROMAN_TAIL.captures_iter(&folded) {
        // `Season III` is read above; any other leading word numbers
        // something other than a season.
        if captures.get(1).is_some() {
            continue;
        }
        ordinals.push(roman_value(&captures[2]));
    }
    for captures in JAPANESE.captures_iter(&folded) {
        let number = captures.get(1).or_else(|| captures.get(2))?;
        ordinals.push(
            number
                .as_str()
                .parse::<i32>()
                .ok()
                .or_else(|| kanji_value(number.as_str())),
        );
    }

    let mut agreed: Option<i32> = None;
    for ordinal in ordinals {
        // A season-shaped token that does not read as a usable number makes
        // the name ambiguous rather than silently ignored.
        let ordinal = ordinal.filter(|value| (1..=MAX_SEASON_ORDINAL).contains(value))?;
        match agreed {
            None => agreed = Some(ordinal),
            Some(previous) if previous == ordinal => {}
            Some(_) => return None,
        }
    }
    agreed
}

fn word_position(words: &[&str], word: &str) -> Option<i32> {
    words
        .iter()
        .position(|candidate| candidate.eq_ignore_ascii_case(word))
        .and_then(|position| i32::try_from(position + 1).ok())
}

fn roman_value(numeral: &str) -> Option<i32> {
    const NUMERALS: [&str; 10] = ["I", "II", "III", "IV", "V", "VI", "VII", "VIII", "IX", "X"];
    word_position(&NUMERALS, numeral)
}

/// `一`–`九`, `十`, `十一`–`十九`, `二十`–`九十九`.
fn kanji_value(numeral: &str) -> Option<i32> {
    fn digit(ch: char) -> Option<i32> {
        "一二三四五六七八九"
            .chars()
            .position(|candidate| candidate == ch)
            .and_then(|position| i32::try_from(position + 1).ok())
    }
    let chars = numeral.chars().collect::<Vec<_>>();
    match chars.as_slice() {
        [single] if *single == '十' => Some(10),
        [single] => digit(*single),
        ['十', ones] => digit(*ones).map(|ones| 10 + ones),
        [tens, '十'] => digit(*tens).map(|tens| tens * 10),
        [tens, '十', ones] => Some(digit(*tens)? * 10 + digit(*ones)?),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::season_ordinal_from_title;
    use crate::{
        AnimeCommunitySeason, AnimeCommunitySeasonRange, AnimeNumberingBridge,
        NumberingBridgeSource,
    };

    fn entry(index: i32, titles: &[&str], range: Option<(i32, i32, i32)>) -> AnimeCommunitySeason {
        AnimeCommunitySeason {
            index,
            titles: titles.iter().map(|title| (*title).to_string()).collect(),
            ranges: range
                .map(|(tvdb_season, start, end)| AnimeCommunitySeasonRange {
                    community_episode_start: 1,
                    community_episode_end: Some(end - start + 1),
                    tvdb_season,
                    tvdb_episode_start: start,
                    tvdb_episode_end: Some(end),
                })
                .into_iter()
                .collect(),
            ..AnimeCommunitySeason::default()
        }
    }

    fn bridge_of(seasons: Vec<AnimeCommunitySeason>) -> AnimeNumberingBridge {
        AnimeNumberingBridge {
            generated_on: "2026-10-01".to_string(),
            seasons,
            ..AnimeNumberingBridge::default()
        }
    }

    fn derived(bridge: &AnimeNumberingBridge) -> Vec<Option<i32>> {
        bridge
            .seasons
            .iter()
            .map(|season| bridge.community_season_number(season))
            .collect()
    }

    /// Five entries over three TVDB seasons, each season split in two parts
    /// except the last. Positions run 1–5; seasons run 1–3.
    fn split_season_bridge() -> AnimeNumberingBridge {
        bridge_of(vec![
            entry(1, &["Ashen Ledger"], Some((1, 1, 11))),
            entry(2, &["Ashen Ledger Part 2"], Some((1, 12, 23))),
            entry(3, &["Ashen Ledger II"], Some((2, 1, 12))),
            entry(4, &["Ashen Ledger Season 2 Part 2"], Some((2, 13, 24))),
            entry(5, &["Ashen Ledger III"], Some((3, 1, 14))),
        ])
    }

    #[test]
    fn a_split_season_entry_is_numbered_by_its_name_not_its_position() {
        assert_eq!(
            derived(&split_season_bridge()),
            vec![Some(1), Some(2), Some(2), Some(2), Some(3)]
        );
    }

    #[test]
    fn an_unnamed_entry_falls_back_to_its_tvdb_season_then_its_place_in_it() {
        let mut bridge = split_season_bridge();
        for season in &mut bridge.seasons {
            season.titles.clear();
        }
        // Entries 1, 3 and 5 start a TVDB season; 2 and 4 are the second entry
        // inside theirs.
        assert_eq!(
            derived(&bridge),
            vec![Some(1), Some(2), Some(2), Some(2), Some(3)]
        );
    }

    #[test]
    fn a_nameless_stub_after_an_entry_without_ranges_takes_its_tvdb_season() {
        let bridge = bridge_of(vec![
            entry(1, &["Bramble Hollow"], Some((1, 1, 12))),
            entry(2, &["Bramble Hollow 2nd Season"], Some((2, 1, 12))),
            entry(3, &["Bramble Hollow Recap"], None),
            entry(4, &[], Some((3, 1, 12))),
        ]);
        assert_eq!(derived(&bridge), vec![Some(1), Some(2), None, Some(3)]);
    }

    #[test]
    fn cours_of_one_long_tvdb_season_are_numbered_by_their_place_in_it() {
        let bridge = bridge_of(vec![
            entry(1, &["Lantern Verge"], Some((1, 1, 14))),
            entry(2, &["Lantern Verge: Ember Circuit"], Some((1, 15, 26))),
            entry(3, &["Lantern Verge: Glass Meridian"], Some((1, 27, 36))),
            entry(4, &["Lantern Verge: Final Chorus"], Some((1, 37, 60))),
        ]);
        assert_eq!(derived(&bridge), vec![Some(1), Some(2), Some(3), Some(4)]);
    }

    #[test]
    fn a_lone_entry_starting_mid_season_settles_no_season() {
        let bridge = bridge_of(vec![entry(
            1,
            &["Lantern Verge: Ember Circuit"],
            Some((2, 13, 24)),
        )]);
        assert_eq!(derived(&bridge), vec![None]);
    }

    #[test]
    fn entry_names_that_disagree_settle_no_ordinal() {
        let mut season = entry(
            4,
            &["Ashen Ledger II", "Ashen Ledger 3rd Season"],
            Some((2, 13, 24)),
        );
        assert_eq!(season.title_season_ordinal(), None);
        season.titles = vec![
            "Ashen Ledger II".to_string(),
            "Ashen Ledger".to_string(),
            "Ashen Ledger 2nd Season".to_string(),
        ];
        assert_eq!(season.title_season_ordinal(), Some(2));
    }

    #[test]
    fn a_tvdb_order_bridge_keeps_the_order_season_as_its_number() {
        let mut bridge = split_season_bridge();
        bridge.source = NumberingBridgeSource::TvdbAlternate;
        assert_eq!(
            derived(&bridge),
            vec![Some(1), Some(2), Some(3), Some(4), Some(5)]
        );
    }

    #[test]
    fn season_ordinals_are_read_from_the_forms_entry_names_use() {
        let cases: &[(&str, Option<i32>)] = &[
            // Spelled-out season numbers.
            ("Lantern Verge Season 3", Some(3)),
            ("Lantern Verge Season 2 Part 2", Some(2)),
            ("Lantern Verge season 12", Some(12)),
            ("Lantern Verge Season Two", Some(2)),
            ("Lantern Verge Season III", Some(3)),
            ("Lantern Verge Season X", Some(10)),
            // Ordinal words and numbers before `Season`.
            ("Lantern Verge 3rd Season", Some(3)),
            ("Lantern Verge 2nd Season: Ember Circuit", Some(2)),
            ("Lantern Verge Third Season", Some(3)),
            ("Lantern Verge 2nd", Some(2)),
            ("Lantern Verge 4th: Glass Meridian", Some(4)),
            // Season tokens.
            ("Lantern Verge S3", Some(3)),
            ("Lantern Verge s3p1", Some(3)),
            // Roman numerals that end the name or open a subtitle.
            ("Lantern Verge II", Some(2)),
            ("Lantern Verge III: Glass Meridian", Some(3)),
            ("Lantern Verge IV (Final Chorus)", Some(4)),
            ("Lantern Verge VII - Final Chorus", Some(7)),
            ("Lantern Verge Ⅲ", Some(3)),
            // Japanese season markers.
            ("灯火の境界 第2期", Some(2)),
            ("灯火の境界 2期", Some(2)),
            ("灯火の境界 第二季", Some(2)),
            ("灯火の境界 第十一期", Some(11)),
            ("灯火の境界 第３シーズン", Some(3)),
            // Agreeing forms of one season.
            ("Lantern Verge II: 2nd Season", Some(2)),
            // Two different seasons: the name settles nothing.
            ("Lantern Verge II: Season 3", None),
            ("Lantern Verge 2nd Season 第3期", None),
            // No season stated.
            ("Lantern Verge", None),
            ("Lantern Verge Part 2", None),
            ("Lantern Verge Part II", None),
            ("Lantern Verge Vol. III", None),
            ("Lantern Verge: Ember Circuit", None),
            ("Lantern Verge 2", None),
            ("Lantern Verge II Ember Circuit", None),
            ("Lantern Verge X", None),
            ("Lantern Verge V", None),
            ("Lantern Verge ii", None),
            ("Lantern Verge Mix", None),
            ("Lantern Verge Season 2026", None),
            ("Lantern Verge Final Season", None),
            ("Lantern Verge Seasons", None),
            ("", None),
        ];

        for (title, expected) in cases {
            assert_eq!(
                season_ordinal_from_title(title),
                *expected,
                "season ordinal of {title:?}"
            );
        }
    }
}
