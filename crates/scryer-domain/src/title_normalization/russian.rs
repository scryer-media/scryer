//! Russian romanization: a catalog name written in Cyrillic and the Latin
//! transliteration a release group or a file name writes for it.

use super::LanguageRules;
use crate::title_spelling::TitleScript;

/// The locale reported when a Latin spelling agrees with a Russian name only
/// through its transliteration.
pub const RUSSIAN_ROMANIZATION_TAG: &str = "ru-latn";

pub(crate) struct RussianRomanization;

impl LanguageRules for RussianRomanization {
    fn equivalence_tag(&self) -> &'static str {
        RUSSIAN_ROMANIZATION_TAG
    }

    /// Catalogs write these as `ru`, `rus`, or either with subtags (`ru-RU`).
    fn applies_to(&self, language: &str) -> bool {
        let language = language.trim().to_ascii_lowercase().replace('_', "-");
        matches!(language.split('-').next().unwrap_or(""), "ru" | "rus")
    }

    fn sample_tags(&self) -> &'static [&'static str] {
        &["ru", "rus", "ru-ru"]
    }

    /// The catalog name is Cyrillic.
    fn script(&self) -> TitleScript {
        TitleScript::Cyrillic
    }

    fn fold(&self, value: &str) -> String {
        fold_with(value, &Switches::ALL)
    }

    fn probes(&self) -> &'static [&'static str] {
        PROBES
    }

    /// The release or file name is Latin.
    fn observed_script(&self) -> TitleScript {
        TitleScript::Latin
    }

    /// A Latin spelling is compared as written: no spelling variance is
    /// folded on this side.
    fn fold_observed(&self, value: &str) -> String {
        value.to_string()
    }
}

/// Probes run through the fold for the index fingerprint. Between them they
/// hold every letter of the Russian alphabet, a name with a letter outside
/// it, and a name mixing Cyrillic and Latin letters, so turning off any one
/// rule changes at least one probe's key; the tests below prove that rule by
/// rule.
pub(crate) const PROBES: &[&str] = &[
    "бавагадо ежёзик",
    "йолмун пирс 7",
    "тафух цочеш",
    "щубъем сырь эюя",
    "ґабур їмка",
    "бавага show",
];

/// One Latin spelling per Russian letter. The hard and soft signs are not
/// here: they are dropped.
const TABLE: &[(char, &str)] = &[
    ('а', "a"),
    ('б', "b"),
    ('в', "v"),
    ('г', "g"),
    ('д', "d"),
    ('е', "e"),
    ('ё', "yo"),
    ('ж', "zh"),
    ('з', "z"),
    ('и', "i"),
    ('й', "y"),
    ('к', "k"),
    ('л', "l"),
    ('м', "m"),
    ('н', "n"),
    ('о', "o"),
    ('п', "p"),
    ('р', "r"),
    ('с', "s"),
    ('т', "t"),
    ('у', "u"),
    ('ф', "f"),
    ('х', "kh"),
    ('ц', "ts"),
    ('ч', "ch"),
    ('ш', "sh"),
    ('щ', "shch"),
    ('ы', "y"),
    ('э', "e"),
    ('ю', "yu"),
    ('я', "ya"),
];

/// Which rules [`fold_with`] applies. The fold always uses [`Switches::ALL`];
/// the tests turn off one rule at a time to prove the index fingerprint sees
/// every rule.
#[derive(Clone, Copy)]
pub(crate) struct Switches {
    /// A table letter left as written; `None` transliterates them all.
    untransliterated: Option<char>,
    dropped_signs: bool,
    russian_letters_only: bool,
    one_script: bool,
}

impl Switches {
    const ALL: Self = Self {
        untransliterated: None,
        dropped_signs: true,
        russian_letters_only: true,
        one_script: true,
    };
}

fn is_cyrillic(character: char) -> bool {
    matches!(character as u32, 0x400..=0x52f)
}

/// Transliterate a Russian name already in
/// [`title_lookup_form`](crate::title_spelling::title_lookup_form) into the
/// Latin spelling release groups write for it:
///
/// * Each letter has one Latin spelling: `а` a, `б` b, `в` v, `г` g, `д` d,
///   `е` e, `ё` yo, `ж` zh, `з` z, `и` i, `й` y, `к` k, `л` l, `м` m, `н` n,
///   `о` o, `п` p, `р` r, `с` s, `т` t, `у` u, `ф` f, `х` kh, `ц` ts, `ч` ch,
///   `ш` sh, `щ` shch, `ы` y, `э` e, `ю` yu, `я` ya.
/// * The hard sign `ъ` and the soft sign `ь` are dropped.
/// * Words stay words and a doubled letter stays doubled; digits pass
///   through.
/// * A name holding a Cyrillic letter outside the Russian alphabet (a
///   Ukrainian, Belarusian or Serbian letter, for example) is refused, and so
///   is a name mixing Cyrillic and Latin letters: a partial transliteration
///   would be a lossy key.
///
/// This is one transliteration, not a phonetic match. The Latin side is
/// compared as written, so a release that spells `х` as `h`, `ю` as `ju`,
/// `щ` as `sch`, `ц` as `c`, `ё` as `e`, or a final `ий` as `ii` or `j` does
/// not agree. An apostrophe written for a sign is not a variant: the lookup
/// form drops apostrophes before any rule runs, as this fold drops the sign.
/// Spelling variants are added only from measured release names.
///
/// Known collisions, accepted because the table has one Latin spelling per
/// letter:
///
/// * `е` and `э` are both `e`; `й` and `ы` are both `y`.
/// * A dropped sign leaves no trace: `мел` and `мель` are one key.
/// * A letter spelled with two Latin letters meets the two letters that
///   spell them one by one: `ц` and `тс`, `щ` and `шч`, `я` and `йа` or
///   `ыа`, `ю` and `йу` or `ыу`, `ё` and `йо` or `ыо`.
///
/// The caller still has to prove no other library identity answers to the
/// key. The title search index persists this output, so
/// [`title_collation_data_version`](crate::title_spelling::title_collation_data_version)
/// hashes [`PROBES`] through it: a rule change here moves the fingerprint and
/// rebuilds the index.
fn fold_with(value: &str, rules: &Switches) -> String {
    let mixed = value.chars().any(is_cyrillic)
        && value
            .chars()
            .any(|character| character.is_alphabetic() && !is_cyrillic(character));
    if rules.one_script && mixed {
        return String::new();
    }
    let mut folded = String::with_capacity(value.len() * 2);
    for character in value.chars() {
        if Some(character) == rules.untransliterated {
            folded.push(character);
        } else if let Some((_, latin)) = TABLE.iter().find(|(letter, _)| *letter == character) {
            folded.push_str(latin);
        } else if matches!(character, 'ъ' | 'ь') {
            if !rules.dropped_signs {
                folded.push(character);
            }
        } else if is_cyrillic(character) && character.is_alphabetic() {
            if rules.russian_letters_only {
                return String::new();
            }
            folded.push(character);
        } else {
            folded.push(character);
        }
    }
    folded
}

#[cfg(test)]
impl Switches {
    /// Every rule of the fold, each paired with the switches that turn only
    /// that rule off: every table letter and every flag.
    pub(crate) fn one_rule_off() -> Vec<(String, Self)> {
        let all = Self::ALL;
        // Listing every field without `..` makes a new switch fail to compile
        // here until it has a counterfactual too.
        let Self {
            untransliterated: _,
            dropped_signs: _,
            russian_letters_only: _,
            one_script: _,
        } = all;
        let mut cases = TABLE
            .iter()
            .map(|(letter, _)| {
                (
                    format!("letter {letter}"),
                    Self {
                        untransliterated: Some(*letter),
                        ..all
                    },
                )
            })
            .collect::<Vec<_>>();
        cases.push((
            "dropped signs".to_string(),
            Self {
                dropped_signs: false,
                ..all
            },
        ));
        cases.push((
            "russian letters only".to_string(),
            Self {
                russian_letters_only: false,
                ..all
            },
        ));
        cases.push((
            "one script".to_string(),
            Self {
                one_script: false,
                ..all
            },
        ));
        cases
    }
}

/// The Russian rule set folding with the given switches.
#[cfg(test)]
pub(crate) struct FoldedWith(pub(crate) Switches);

#[cfg(test)]
impl LanguageRules for FoldedWith {
    fn equivalence_tag(&self) -> &'static str {
        RussianRomanization.equivalence_tag()
    }
    fn applies_to(&self, language: &str) -> bool {
        RussianRomanization.applies_to(language)
    }
    fn sample_tags(&self) -> &'static [&'static str] {
        RussianRomanization.sample_tags()
    }
    fn script(&self) -> TitleScript {
        RussianRomanization.script()
    }
    fn fold(&self, value: &str) -> String {
        fold_with(value, &self.0)
    }
    fn probes(&self) -> &'static [&'static str] {
        RussianRomanization.probes()
    }
    fn observed_script(&self) -> TitleScript {
        RussianRomanization.observed_script()
    }
    fn fold_observed(&self, value: &str) -> String {
        RussianRomanization.fold_observed(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::title_spelling::{title_lookup_form, title_spelling_fingerprint};

    fn key(value: &str) -> String {
        RussianRomanization.fold(&title_lookup_form(value))
    }

    fn observed(value: &str) -> String {
        RussianRomanization.fold_observed(&title_lookup_form(value))
    }

    #[test]
    fn the_fingerprint_notices_every_rule() {
        let current = title_spelling_fingerprint(&[&RussianRomanization]);
        assert_eq!(
            current,
            title_spelling_fingerprint(&[&FoldedWith(Switches::ALL)])
        );
        let cases = Switches::one_rule_off();
        assert_eq!(cases.len(), TABLE.len() + 3);
        for (rule, switches) in cases {
            assert_ne!(
                current,
                title_spelling_fingerprint(&[&FoldedWith(switches)]),
                "turning off {rule} leaves every probe's key unchanged"
            );
        }
    }

    #[test]
    fn the_rule_set_reads_cyrillic_catalog_names_and_latin_observed_names() {
        assert_eq!(RussianRomanization.script(), TitleScript::Cyrillic);
        assert_eq!(RussianRomanization.observed_script(), TitleScript::Latin);
        for tag in ["ru", "RU", "rus", "ru-RU", "ru_RU", "rus-Cyrl"] {
            assert!(RussianRomanization.applies_to(tag), "{tag}");
        }
        for tag in [
            "uk", "ukr", "be", "sr", "bg", "en", "ja", "x-jat", "run", "",
        ] {
            assert!(!RussianRomanization.applies_to(tag), "{tag}");
        }
    }

    #[test]
    fn every_letter_has_its_one_latin_spelling() {
        assert_eq!(TABLE.len(), 31);
        for (letter, latin) in TABLE {
            // Between two plain letters, so the spelling is read in place.
            assert_eq!(
                key(&format!("ба{letter}ам")),
                format!("ba{latin}am"),
                "{letter}"
            );
            // And it is that letter's spelling, not a neighbour's left as is.
            assert_ne!(key(&format!("ба{letter}ам")), "baam", "{letter}");
        }
        assert_eq!(key("Бавагадо Ежёзик"), "bavagado ezhyozik");
        assert_eq!(key("Йолмун Пирс"), "yolmun pirs");
        assert_eq!(key("Тафух Цочеш"), "tafukh tsochesh");
        assert_eq!(key("Щубем Сыр Эюя"), "shchubem syr eyuya");
    }

    #[test]
    fn a_latin_spelling_is_compared_as_written() {
        for (catalog, written) in [
            ("ИИскра", "IIskra"),
            ("Жухарь Дожжа", "Zhukhar Dozhzha"),
            ("Тихий Чащук", "Tikhiy Chashchuk"),
            ("Юрная Цыпь", "Yurnaya Tsyp"),
        ] {
            assert_eq!(key(catalog), observed(written), "{catalog} / {written}");
        }
        // No spelling variance is folded: these are other spellings of the
        // same letters, and none of them agrees.
        for (catalog, variant) in [
            ("Жухарь", "Zhuhar"),
            ("Жухарь", "Zhuxar"),
            ("Тихий", "Tikhii"),
            ("Тихий", "Tikhij"),
            ("Юрная", "Jurnaja"),
            ("Чащук", "Chaschuk"),
            ("Цыпь", "Cyp"),
            ("Ежёзик", "Ezhezik"),
        ] {
            assert_ne!(key(catalog), observed(variant), "{catalog} / {variant}");
        }
        // The lookup form drops an apostrophe, as the fold drops the sign.
        assert_eq!(key("Цыпь"), observed("Tsyp'"));
    }

    #[test]
    fn words_and_doubled_letters_are_kept() {
        assert_eq!(key("ИИскра в Ночи"), "iiskra v nochi");
        assert_ne!(key("ИИскра в Ночи"), key("ИИскравночи"));
        assert_ne!(key("ИИскра"), key("Искра"));
        assert_ne!(key("Дожжа"), key("Дожа"));
    }

    #[test]
    fn the_signs_are_dropped_and_nothing_else_is() {
        assert_eq!(key("Объём"), "obyom");
        assert_eq!(key("Сырь"), "syr");
        // Documented collision: a dropped sign leaves no trace.
        assert_eq!(key("Мел"), key("Мель"));
        // A sign is not a vowel: the name with a letter there stays apart.
        assert_ne!(key("Мель"), key("Мели"));
        assert_ne!(key("Объём"), key("Обиём"));
    }

    #[test]
    fn the_table_merges_only_the_letters_it_documents() {
        for (left, right) in [
            ("Эхор", "Ехор"),
            ("Бой", "Боы"),
            ("Цап", "Тсап"),
            ("Щук", "Шчук"),
            ("Яр", "Йар"),
            ("Юр", "Йур"),
            ("Ёр", "Йор"),
        ] {
            assert_eq!(key(left), key(right), "{left} / {right}");
        }
        for (left, right) in [
            ("Жар", "Зар"),
            ("Хор", "Кор"),
            ("Чар", "Цар"),
            ("Шар", "Сар"),
            ("Щар", "Шар"),
            ("Юла", "Ула"),
            ("Яма", "Ама"),
            ("Ёлка", "Елка"),
            ("Иск", "Йск"),
        ] {
            assert_ne!(key(left), key(right), "{left} / {right}");
        }
    }

    #[test]
    fn a_letter_outside_the_russian_alphabet_refuses_the_name() {
        // Ukrainian, Belarusian and Serbian letters.
        for name in ["Ґабур", "Їмка", "Сіній", "Єнот", "Ўзор", "Љубав", "Ђак"]
        {
            assert_eq!(key(name), "", "{name}");
        }
        // One such letter refuses the whole name, not only its word.
        assert_eq!(key("Бавага Їмка"), "");
        // The same name without it is transliterated.
        assert_eq!(key("Бавага Имка"), "bavaga imka");
        assert_eq!(key("Синий"), "siniy");
    }

    #[test]
    fn a_name_mixing_cyrillic_and_latin_letters_is_refused() {
        assert_eq!(key("Бавага Show"), "");
        assert_eq!(key("Bавага"), "");
        // Digits are not letters of another script.
        assert_eq!(key("Бавага 7"), "bavaga 7");
        assert_eq!(key("Йолмун 2049"), "yolmun 2049");
    }
}
