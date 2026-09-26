//! Japanese romanization: the spellings a catalog and a release group can
//! each choose for one Japanese name written in Latin letters.

use super::LanguageRules;
use crate::title_spelling::TitleScript;

/// The locale reported for two spellings that agree only once romanization
/// variance is folded away.
pub const JAPANESE_ROMANIZATION_TAG: &str = "ja-latn";

pub(crate) struct JapaneseRomanization;

impl LanguageRules for JapaneseRomanization {
    fn equivalence_tag(&self) -> &'static str {
        JAPANESE_ROMANIZATION_TAG
    }

    /// Catalogs write these as `ja`, `jpn`, `ja-Latn`, or the AniDB/TVDB
    /// transliteration tag `x-jat`.
    fn applies_to(&self, language: &str) -> bool {
        let language = language.trim().to_ascii_lowercase().replace('_', "-");
        let root = language.split('-').next().unwrap_or("");
        matches!(root, "ja" | "jpn") || language.starts_with("x-jat")
    }

    fn sample_tags(&self) -> &'static [&'static str] {
        &["ja", "jpn", "ja-latn", "x-jat"]
    }

    fn script(&self) -> TitleScript {
        TitleScript::Latin
    }

    fn fold(&self, value: &str) -> String {
        fold_with(value, &Switches::ALL)
    }

    fn probes(&self) -> &'static [&'static str] {
        PROBES
    }
}

/// Probes run through the romanization fold for the index fingerprint.
/// Turning off any one rule of the fold, or any one onset or doubled
/// consonant of the romaji grammar, changes at least one probe's key; the
/// tests below prove that rule by rule.
pub(crate) const PROBES: &[&str] = &[
    "gasshou wo shimbun sampo",
    "tōkyō yuusha oosaka",
    "kāsan onīsan sūji onēsan",
    "kono uta kono oto yuu uta sem pai kaze wo miru",
    "wotaku",
    "good yuusha party",
    "hoshi kage 2",
    "kyuu ryuu chuu tsuu juu fuu gyuu nyuu hyuu byuu pyuu myuu",
    "shuu syuu tyuu zyuu jyuu dyuu",
    "kuu guu suu zuu tuu duu huu buu puu muu yuu ruu wuu",
    "akkuu agguu sshou attuu adduu ahhuu abbuu appuu arruu affuu ajjuu acchuu matchou ammuu",
];

/// Syllable onsets of the romaji grammar, each followed by one vowel. `n`
/// and `ny` are absent because a syllabic `n` followed by a vowel or a `y`
/// syllable already reads them.
const ONSETS: &[&str] = &[
    "sh", "ch", "ts", "ky", "gy", "hy", "by", "py", "my", "ry", "sy", "ty", "zy", "jy", "dy", "k",
    "g", "s", "z", "t", "d", "h", "b", "p", "m", "y", "r", "w", "f", "j",
];

/// Consonants that double before a syllable of their own row (`kk`, `ss`,
/// `cch`). A doubled `m` is the syllabic `m` before a labial, and `tch` has
/// its own rule.
const GEMINATES: &[u8] = b"kgstdhbprfjc";

/// Which rules [`fold_with`] applies. The fold always uses [`Switches::ALL`];
/// the tests turn off one rule at a time to prove the index fingerprint sees
/// every rule.
#[derive(Clone, Copy)]
pub(crate) struct Switches {
    macron: bool,
    long_ou: bool,
    long_oo: bool,
    long_uu: bool,
    wo_particle: bool,
    /// Off: drop every `w` before `o`, not only where a particle can sit.
    wo_placement: bool,
    labial_n: bool,
    romaji_gate: bool,
    word_join: bool,
    digit_boundary: bool,
    bare_vowel: bool,
    syllabic_n: bool,
    syllabic_m: bool,
    geminate_tch: bool,
    /// An onset the grammar leaves out; empty for none.
    dropped_onset: &'static str,
    /// A doubled consonant the grammar leaves out; `0` for none.
    dropped_geminate: u8,
}

impl Switches {
    const ALL: Self = Self {
        macron: true,
        long_ou: true,
        long_oo: true,
        long_uu: true,
        wo_particle: true,
        wo_placement: true,
        labial_n: true,
        romaji_gate: true,
        word_join: true,
        digit_boundary: true,
        bare_vowel: true,
        syllabic_n: true,
        syllabic_m: true,
        geminate_tch: true,
        dropped_onset: "",
        dropped_geminate: 0,
    };
}

#[derive(Clone, Copy, PartialEq)]
enum Segment {
    /// Adjacent words that each read as romaji, joined into one run.
    Romaji,
    /// A word that does not read as romaji, such as an English word.
    Other,
    /// A word holding a digit.
    Number,
}

/// Fold the romanization variance a catalog and a release group can each pick
/// for one Japanese name, for a name already in
/// [`title_lookup_form`](crate::title_spelling::title_lookup_form):
///
/// * One name written as one word or several is one name: `Hoshi Kage` /
///   `Hoshikage`, `Shite Mite` / `Shitemite`, `Kon'ya` (which the lookup form
///   spells `kon ya`) / `Konya`. Release groups and catalogs split compounds,
///   auxiliaries and particles differently, and an edit-distance lane cannot
///   bridge a different word count. So the words are joined, except that a
///   word holding a digit keeps its space on both sides: `ken 2` must not
///   become the `ken2` of a different name.
/// * Adjacent words that read as romaji (see [`reads_as_romaji`]) are joined
///   into one run. Vowels fold inside each source word only; a labial folds
///   across the joined run, so `Sem Pai` / `Sempai` / `Senpai` agree.
/// * Within one word, a long `o` or `u` written with a macron, doubled, or
///   bare is one vowel (`Gasshō` / `Gasshou` / `Gassho`), and an `o` absorbs
///   every `o` and `u` after it. A long `a`, `i` or `e` written with a macron
///   or bare is one vowel too; a doubled `aa`, `ii`, `ee` and a long `e`
///   written `ei` are left alone (release names measured against catalog
///   titles gave no case for them and several cases of distinct words they
///   would merge).
/// * A vowel is never absorbed across a word boundary, so a particle before
///   an `o`- or `u`-initial word keeps both: `Hana no Okami` /
///   `Hana no Ookami` / `Hana no Ōkami` agree and stay apart from
///   `Hana no Kami`, and `Hoshi no Otome` from `Hoshi no Tome`.
/// * The `wo` particle is `o`: a word `wo`, or a `w` before `o` inside a word
///   whose previous letter is a vowel or `n`, where a joined particle sits
///   (`Kaze wo Miru` / `Kazewo Miru` / `Kazewomiru` / `Kaze o Miru`). A word
///   that opens with `wo` keeps it: `Wotaku` is not `Otaku`.
/// * `m` before `b` or `p` is `n` (`Shimbun` / `Shinbun`).
/// * A word that does not read as romaji keeps its spelling, so most English
///   words in a romanized name stay unfolded: `good` does not fold onto `god`,
///   nor `room` onto `rom`.
///
/// Known collisions and misses, accepted for the cases they fix:
///
/// * A `w` before `o` after a vowel inside a word is read as the particle:
///   `Kaworu` keys as `Kaoru`, `Iwo` as `Io`.
/// * `m` before a romaji word that opens with `b` or `p` is `n` even when
///   the words are English that happen to read as romaji: `Him Before` keys
///   as `Hin Before`.
/// * A vowel run that straddles a boundary in a joined spelling collapses
///   inside that one word, while the split spelling keeps both vowels:
///   `KonoOto` misses `Kono Oto`, `Kiminouta` misses `Kimi no Uta`,
///   `Gasshouwo Utau` misses `Gasshou wo Utau`. Release names and catalogs
///   measured gave no case of these joined spellings.
///
/// This reduction is applied to both sides of one comparison and only to
/// Japanese-romanized names; the caller still has to prove no other library
/// identity answers to that key. The title search index persists this
/// output, so
/// [`title_collation_data_version`](crate::title_spelling::title_collation_data_version)
/// hashes [`PROBES`] through it: a rule change here moves the fingerprint and
/// rebuilds the index.
fn fold_with(value: &str, rules: &Switches) -> String {
    let words = value.split_whitespace().collect::<Vec<_>>();
    let mut segments: Vec<(Segment, Vec<&str>)> = Vec::with_capacity(words.len());
    for (index, word) in words.iter().enumerate() {
        let segment = if rules.digit_boundary && word.chars().any(|ch| ch.is_ascii_digit()) {
            Segment::Number
        } else if !rules.romaji_gate
            || reads_as_romaji_before(word, words.get(index + 1).copied(), rules)
        {
            Segment::Romaji
        } else {
            Segment::Other
        };
        match segments.last_mut() {
            Some((Segment::Romaji, run)) if rules.word_join && segment == Segment::Romaji => {
                run.push(word);
            }
            _ => segments.push((segment, vec![*word])),
        }
    }
    let mut folded = String::with_capacity(value.len());
    for (index, (segment, run)) in segments.iter().enumerate() {
        if index > 0
            && (!rules.word_join
                || *segment == Segment::Number
                || segments[index - 1].0 == Segment::Number)
        {
            folded.push(' ');
        }
        if *segment == Segment::Romaji {
            folded.push_str(&fold_romaji_run(run, rules));
        } else {
            folded.extend(run.iter().copied());
        }
    }
    folded
}

/// Whether a word reads as romaji where it stands: on its own, or as a stem
/// whose final `m` is a syllabic `n` before a following romaji word that
/// opens with `b` or `p` (`sem pai`, which joins to `sempai`). The second
/// case exists only so that `m` meets the labial it folds before; an English
/// word such as `room` before `mate`, or `doom` before `patrol`, stays as
/// written.
fn reads_as_romaji_before(word: &str, next: Option<&str>, rules: &Switches) -> bool {
    if reads_as_romaji(word, rules) {
        return true;
    }
    rules.syllabic_m
        && next.is_some_and(|next| {
            next.bytes()
                .next()
                .is_some_and(|first| matches!(first.to_ascii_lowercase(), b'b' | b'p'))
                && reads_as_romaji(next, rules)
        })
        && word
            .strip_suffix(['m', 'M'])
            .is_some_and(|stem| !stem.is_empty() && reads_as_romaji(stem, rules))
}

/// Whether a word can be read as Japanese romanization, which is where the
/// long-vowel, `wo` and labial rules apply: it carries a macron, or it splits
/// entirely into romaji syllables (an onset and a vowel, a bare vowel, a
/// syllabic `n`, a doubled consonant or `tch`, or `m` before `b`, `p`, `m`).
///
/// This is a spelling test, not a dictionary. `good`, `four`, `room`, `lamp`
/// and `the` do not split and are never folded, but a common English word
/// that happens to split into syllables is read as romaji and folds like one:
/// `moon`, `soon`, `too`, `you`, `tempo`, `jumbo`, `samba`.
///
/// Some spellings the fold used to reach no longer pass and are left as
/// written: a long `o` spelled `oh` (`ohkouchi`), a word mixing letters and
/// digits, `zz`, `kw`, `dz`, and a word ending in a consonant other than `n`.
fn reads_as_romaji(word: &str, rules: &Switches) -> bool {
    if rules.macron
        && word
            .chars()
            .any(|ch| matches!(ch, 'ā' | 'ī' | 'ū' | 'ē' | 'ō'))
    {
        return true;
    }
    let word = word.to_ascii_lowercase();
    let bytes = word.as_bytes();
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_lowercase) {
        return false;
    }
    let is_vowel = |byte: u8| matches!(byte, b'a' | b'i' | b'u' | b'e' | b'o');
    let doubles = |byte: u8| GEMINATES.contains(&byte) && byte != rules.dropped_geminate;
    // reachable[i]: the first i letters split into whole syllables.
    let mut reachable = vec![false; bytes.len() + 1];
    reachable[0] = true;
    for start in 0..bytes.len() {
        if !reachable[start] {
            continue;
        }
        let rest = &bytes[start..];
        if (rules.bare_vowel && is_vowel(rest[0])) || (rules.syllabic_n && rest[0] == b'n') {
            reachable[start + 1] = true;
        }
        if rest.len() > 1
            && ((doubles(rest[0]) && rest[1] == rest[0])
                || (rules.geminate_tch && rest[0] == b't' && rest[1] == b'c')
                || (rules.syllabic_m && rest[0] == b'm' && matches!(rest[1], b'b' | b'p' | b'm')))
        {
            reachable[start + 1] = true;
        }
        for onset in ONSETS {
            if *onset == rules.dropped_onset {
                continue;
            }
            let onset = onset.as_bytes();
            if rest.len() > onset.len() && rest.starts_with(onset) && is_vowel(rest[onset.len()]) {
                reachable[start + onset.len() + 1] = true;
            }
        }
    }
    reachable[bytes.len()]
}

/// Whether a vowel can end a syllable before a joined particle `wo`.
fn precedes_joined_particle(character: char) -> bool {
    matches!(
        character,
        'a' | 'i' | 'u' | 'e' | 'o' | 'ā' | 'ī' | 'ū' | 'ē' | 'ō' | 'n'
    )
}

/// Whether a long vowel absorbs the letter after it: an `o` takes `o` and
/// `u`, a `u` takes `u`.
fn absorbs(vowel: char, following: char, rules: &Switches) -> bool {
    match (vowel, following) {
        ('o', 'u') => rules.long_ou,
        ('o', 'o') => rules.long_oo,
        ('u', 'u') => rules.long_uu,
        _ => false,
    }
}

/// Fold one word of a romaji run: macrons become bare vowels, the particle
/// `w` is dropped, and every `o`/`u` run collapses to its first vowel.
fn fold_romaji_word(word: &str, rules: &Switches) -> Vec<char> {
    let source = word.chars().collect::<Vec<_>>();
    let whole_particle = word == "wo";
    let mut characters = Vec::with_capacity(source.len());
    for (index, &character) in source.iter().enumerate() {
        let is_particle_w = character == 'w'
            && matches!(source.get(index + 1), Some('o' | 'ō'))
            && (whole_particle
                || !rules.wo_placement
                || (index > 0 && precedes_joined_particle(source[index - 1])));
        if rules.wo_particle && is_particle_w {
            continue;
        }
        characters.push(match character {
            'ā' if rules.macron => 'a',
            'ī' if rules.macron => 'i',
            'ū' if rules.macron => 'u',
            'ē' if rules.macron => 'e',
            'ō' if rules.macron => 'o',
            other => other,
        });
    }
    let mut folded = Vec::with_capacity(characters.len());
    for character in characters {
        if folded
            .last()
            .is_some_and(|&last| absorbs(last, character, rules))
        {
            continue;
        }
        folded.push(character);
    }
    folded
}

/// Fold one romaji run: each word on its own, so a vowel never folds across
/// a word boundary, then `m` before `b` or `p` becomes `n` across the whole
/// joined run.
fn fold_romaji_run(words: &[&str], rules: &Switches) -> String {
    let mut run: Vec<char> = Vec::new();
    for word in words {
        run.extend(fold_romaji_word(word, rules));
    }
    let mut folded = String::with_capacity(run.len());
    for (index, &character) in run.iter().enumerate() {
        if rules.labial_n && character == 'm' && matches!(run.get(index + 1), Some('b' | 'p')) {
            folded.push('n');
        } else {
            folded.push(character);
        }
    }
    folded
}

#[cfg(test)]
impl Switches {
    /// Every rule of the fold, each paired with the switches that turn only
    /// that rule off: every flag, every onset and every doubled consonant.
    pub(crate) fn one_rule_off() -> Vec<(String, Self)> {
        let all = Self::ALL;
        // Listing every field without `..` makes a new switch fail to compile
        // here until it has a counterfactual too.
        let Self {
            macron: _,
            long_ou: _,
            long_oo: _,
            long_uu: _,
            wo_particle: _,
            wo_placement: _,
            labial_n: _,
            romaji_gate: _,
            word_join: _,
            digit_boundary: _,
            bare_vowel: _,
            syllabic_n: _,
            syllabic_m: _,
            geminate_tch: _,
            dropped_onset: _,
            dropped_geminate: _,
        } = all;
        let mut cases = vec![
            (
                "macron",
                Self {
                    macron: false,
                    ..all
                },
            ),
            (
                "long ou",
                Self {
                    long_ou: false,
                    ..all
                },
            ),
            (
                "long oo",
                Self {
                    long_oo: false,
                    ..all
                },
            ),
            (
                "long uu",
                Self {
                    long_uu: false,
                    ..all
                },
            ),
            (
                "wo",
                Self {
                    wo_particle: false,
                    ..all
                },
            ),
            (
                "wo placement",
                Self {
                    wo_placement: false,
                    ..all
                },
            ),
            (
                "m before a labial",
                Self {
                    labial_n: false,
                    ..all
                },
            ),
            (
                "romaji gate",
                Self {
                    romaji_gate: false,
                    ..all
                },
            ),
            (
                "word join",
                Self {
                    word_join: false,
                    ..all
                },
            ),
            (
                "digit boundary",
                Self {
                    digit_boundary: false,
                    ..all
                },
            ),
            (
                "bare vowel",
                Self {
                    bare_vowel: false,
                    ..all
                },
            ),
            (
                "syllabic n",
                Self {
                    syllabic_n: false,
                    ..all
                },
            ),
            (
                "syllabic m",
                Self {
                    syllabic_m: false,
                    ..all
                },
            ),
            (
                "tch",
                Self {
                    geminate_tch: false,
                    ..all
                },
            ),
        ]
        .into_iter()
        .map(|(rule, switches)| (rule.to_string(), switches))
        .collect::<Vec<_>>();
        for onset in ONSETS {
            cases.push((
                format!("onset {onset}"),
                Self {
                    dropped_onset: onset,
                    ..all
                },
            ));
        }
        for &letter in GEMINATES {
            cases.push((
                format!("doubled {}", letter as char),
                Self {
                    dropped_geminate: letter,
                    ..all
                },
            ));
        }
        cases
    }
}

/// The Japanese rule set folding with the given switches.
#[cfg(test)]
pub(crate) struct FoldedWith(pub(crate) Switches);

#[cfg(test)]
impl LanguageRules for FoldedWith {
    fn equivalence_tag(&self) -> &'static str {
        JapaneseRomanization.equivalence_tag()
    }
    fn applies_to(&self, language: &str) -> bool {
        JapaneseRomanization.applies_to(language)
    }
    fn sample_tags(&self) -> &'static [&'static str] {
        JapaneseRomanization.sample_tags()
    }
    fn script(&self) -> TitleScript {
        JapaneseRomanization.script()
    }
    fn fold(&self, value: &str) -> String {
        fold_with(value, &self.0)
    }
    fn probes(&self) -> &'static [&'static str] {
        JapaneseRomanization.probes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::title_spelling::{title_lookup_form, title_spelling_fingerprint};

    fn fold(value: &str) -> String {
        JapaneseRomanization.fold(value)
    }

    fn key(value: &str) -> String {
        fold(&title_lookup_form(value))
    }

    fn romaji(word: &str) -> bool {
        reads_as_romaji(word, &Switches::ALL)
    }

    #[test]
    fn the_fingerprint_notices_every_rule() {
        let current = title_spelling_fingerprint(&[&JapaneseRomanization]);
        assert_eq!(
            current,
            title_spelling_fingerprint(&[&FoldedWith(Switches::ALL)])
        );
        let cases = Switches::one_rule_off();
        assert_eq!(cases.len(), 14 + ONSETS.len() + GEMINATES.len());
        for (rule, switches) in cases {
            assert_ne!(
                current,
                title_spelling_fingerprint(&[&FoldedWith(switches)]),
                "turning off {rule} leaves every probe's key unchanged"
            );
        }
    }

    #[test]
    fn long_vowels_fold_only_in_words_that_read_as_romaji() {
        // Romaji words still fold.
        assert_eq!(fold("kaijuu no mori"), "kaijunomori");
        assert_eq!(fold("gakkou sempai"), "gakkosenpai");
        assert_eq!(fold("tōrimichi"), "torimichi");
        // English words beside them keep their vowels and their `m`.
        assert_eq!(fold("good yuusha"), "goodyusha");
        assert_eq!(fold("four room"), "fourroom");
        assert_eq!(fold("young soul"), "youngsoul");
        assert_eq!(fold("lamp camp"), "lampcamp");
        for (english, other) in [
            ("good", "god"),
            ("four", "for"),
            ("room", "rom"),
            ("our", "or"),
        ] {
            assert_ne!(
                fold(&format!("hoshi no {english}")),
                fold(&format!("hoshi no {other}")),
                "{english} / {other}"
            );
        }
    }

    #[test]
    fn romaji_reading_splits_whole_words_into_syllables() {
        for word in [
            "gasshou", "shimbun", "sempai", "kyouto", "tsukimi", "matcha", "konya", "n", "o",
            "rokubō",
        ] {
            assert!(romaji(word), "{word}");
        }
        for word in [
            "good", "four", "room", "lamp", "the", "x", "kaiju2", "café", "", "ohkouchi", "pizza",
            "sem",
        ] {
            assert!(!romaji(word), "{word}");
        }
        // A spelling test, not a dictionary: English words that split into
        // syllables read as romaji.
        for word in ["moon", "soon", "too", "you", "tempo", "jumbo", "samba"] {
            assert!(romaji(word), "{word}");
        }
    }

    #[test]
    fn split_and_joined_spellings_agree_unless_a_vowel_straddles_the_boundary() {
        for spellings in [
            // `m|b` and `m|p`: the labial folds across the boundary.
            &["Tom Bo no Mori", "Tombo no Mori", "Tonbo no Mori"][..],
            &["Sem Pai", "Sempai", "Senpai"],
            // `wo` as its own word, joined, or written `o`.
            &["Kaze wo Miru", "Kazewo Miru", "Kazewomiru", "Kaze o Miru"],
            // A long vowel before the particle, however each is spelled.
            &[
                "Gasshou wo Utau",
                "Gassho o Utau",
                "Gasshoo o Utau",
                "Gasshō wo Utau",
            ],
        ] {
            for spelling in spellings {
                assert_eq!(key(spellings[0]), key(spelling), "{spellings:?}");
            }
        }
        assert_eq!(key("Sem Pai"), "senpai");
        assert_eq!(key("Kaze wo Miru"), "kazeomiru");
        // Documented misses: a vowel run that straddles a joined boundary
        // collapses inside the one word, and the split spelling keeps both
        // vowels. Release names and catalogs measured gave no case of these
        // joined spellings.
        for (joined, split) in [
            ("KonoOto", "Kono Oto"),
            ("Kiminouta", "Kimi no Uta"),
            ("Gasshouwo Utau", "Gasshou wo Utau"),
        ] {
            assert_ne!(key(joined), key(split), "{joined} / {split}");
        }
    }

    #[test]
    fn a_long_vowel_folds_inside_its_own_word_only() {
        for spelling in ["Hana no Okami", "Hana no Ookami", "Hana no Ōkami"] {
            assert_eq!(key(spelling), "hananookami", "{spelling}");
        }
        // A vowel that opens the next word is never absorbed into a particle,
        // so the common `no` + long-vowel title shape stays apart from the
        // name without that vowel.
        for (long, other) in [
            ("Hana no Okami", "Hana no Kami"),
            ("Hana no Ookami", "Hana no Kami"),
            ("Hoshi no Otome", "Hoshi no Tome"),
            ("Kimi no Uta", "Kimi no Ta"),
            ("Hana no Ouji", "Hana no Ji"),
            ("Mori no Ousama", "Mori no Sama"),
        ] {
            assert_ne!(key(long), key(other), "{long} / {other}");
        }
    }

    #[test]
    fn only_a_particle_w_is_dropped() {
        for (left, right) in [
            ("Wotaku", "Otaku"),
            ("Woman", "Oman"),
            ("Women", "Omen"),
            ("Won", "On"),
            ("Wore no Hoshi", "Ore no Hoshi"),
        ] {
            assert_ne!(key(left), key(right), "{left} / {right}");
        }
        assert_eq!(key("Wotaku"), "wotaku");
        // Where a joined particle can sit, after a vowel or `n`, the `w` goes:
        // documented cost, `Kaworu` keys as `Kaoru`.
        assert_eq!(key("Kaworu"), key("Kaoru"));
        assert_eq!(key("Kazewomiru"), "kazeomiru");
        assert_eq!(key("Kaze wo Miru"), "kazeomiru");
    }

    #[test]
    fn a_word_that_is_not_romaji_stops_the_fold_at_its_boundary() {
        // The romaji word beside an English word still folds on its own.
        assert_eq!(key("Good Yuusha"), key("Good Yusha"));
        assert_eq!(key("Yuusha Party"), key("Yusha Party"));
        assert_eq!(key("Yuusha Party"), "yushaparty");
        // The English word does not, and it does not fold through its
        // neighbour either.
        assert_ne!(key("Good Hoshi"), key("God Hoshi"));
        assert_ne!(key("Room Oto"), key("Rom Oto"));
        // A final `m` is a syllabic `n` only before a romaji word that opens
        // with a labial.
        assert_eq!(key("Sem Kage"), "semkage");
        assert_ne!(key("Room Mate"), key("Rom Mate"));
        assert_ne!(key("Doom Patrol"), key("Dom Patrol"));
        assert_eq!(key("Sem Pai"), "senpai");
        assert_eq!(key("Sempai"), "senpai");
        assert_eq!(key("Senpai"), "senpai");
        // One word that fuses an English word with a romaji one cannot be
        // told apart from a romaji word: `GoodYuusha` splits as `go-o-dyu-u-
        // sha` and folds whole, so it does not meet `Good Yuusha`.
    }

    #[test]
    fn a_name_split_into_words_or_joined_is_one_key() {
        for (left, right) in [
            ("Hoshi Kage no Machi", "Hoshikage no Machi"),
            ("Hoshi Kage no Machi", "HOSHIKAGE NO MACHI"),
            ("Kirameki ni Shite Mite", "Kirameki ni Shitemite"),
            ("Tsuki Mori Banashi", "Tsukimoribanashi"),
            ("Kumo no Kane de wa", "Kumono Kane dewa"),
            ("Yuusha-sama no Tabi", "Yusha sama no Tabi"),
        ] {
            assert_eq!(key(left), key(right), "{left} / {right}");
        }
        assert_eq!(key("Hoshi Kage no Machi"), "hoshikagenomachi");
    }

    #[test]
    fn a_number_keeps_its_word_boundary() {
        assert_eq!(key("Hoshi Kage 2"), "hoshikage 2");
        assert_eq!(key("Hoshi Kage 2 Tabi"), "hoshikage 2 tabi");
        for (left, right) in [
            ("Tsurugi Ken 2", "Tsurugi Ken2"),
            ("Mori 2 Kage", "Mori2 Kage"),
            ("Kage 20", "Kage 2 0"),
        ] {
            assert_ne!(key(left), key(right), "{left} / {right}");
        }
    }

    #[test]
    fn a_syllabic_n_apostrophe_folds_only_through_the_word_join() {
        let marked = title_lookup_form("Kon'ya no Hoshi");
        let bare = title_lookup_form("Konya no Hoshi");
        // Without the join the two spellings stay apart: the apostrophe is a
        // word break in the lookup form and no other rule touches it.
        let unjoined = Switches {
            word_join: false,
            ..Switches::ALL
        };
        assert_ne!(fold_with(&marked, &unjoined), fold_with(&bare, &unjoined));
        assert_eq!(fold(&marked), fold(&bare));
    }

    #[test]
    fn joining_words_keeps_different_names_apart() {
        for (left, right) in [
            ("Kage no Machi", "Kageno Mori"),
            ("Hoshi Kage", "Hoshi Kaze"),
            ("Mizu no Tane", "Mizuno Tanei"),
            ("good hoshi", "god hoshi"),
            ("time hoshi", "chime hoshi"),
        ] {
            assert_ne!(key(left), key(right), "{left} / {right}");
        }
    }
}
