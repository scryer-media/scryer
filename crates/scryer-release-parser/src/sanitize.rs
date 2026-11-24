use html_escape::decode_html_entities;
use unicode_normalization::UnicodeNormalization;

pub(crate) const MAX_INPUT_BYTES: usize = 4096;

/// The text an HTML named entity stands for, by its name without `&` and
/// `;` (`eacute`, `nbsp`); `None` for a name HTML does not define.
pub fn html_named_entity(name: &str) -> Option<&'static str> {
    html_escape::NAMED_ENTITIES
        .binary_search_by(|(known, _)| (*known).cmp(name.as_bytes()))
        .ok()
        .map(|index| html_escape::NAMED_ENTITIES[index].1)
}

#[derive(Debug, Clone)]
pub(crate) struct SanitizedInput {
    pub(crate) value: String,
    pub(crate) hints: Vec<String>,
}

pub(crate) fn sanitize_input(raw: &str) -> SanitizedInput {
    let mut hints = Vec::new();
    let truncated = truncate_utf8(raw, MAX_INPUT_BYTES, &mut hints);
    let html_decoded = decode_common_entities(truncated, &mut hints);
    let normalized = html_decoded.nfkc().collect::<String>();
    let cleaned = normalized
        .chars()
        .filter_map(|ch| sanitize_char(ch, &mut hints))
        .collect::<String>();
    let punctuation_normalized = cleaned.replace('&', " and ").replace(':', " ");

    SanitizedInput {
        value: punctuation_normalized,
        hints,
    }
}

fn decode_common_entities(raw: &str, hints: &mut Vec<String>) -> String {
    let mut decoded = raw.to_string();
    let mut changed = false;

    // Real-world feeds occasionally arrive double-encoded (for example
    // `&amp;amp;`). Decode a bounded number of times so we normalize common
    // garbage without turning sanitize into an open-ended rewrite loop.
    for _ in 0..2 {
        let next = decode_html_entities(&decoded).into_owned();
        if next == decoded {
            break;
        }
        decoded = next;
        changed = true;
    }

    if changed {
        hints.push("html_entity_decoded".to_string());
    }

    decoded
}

fn truncate_utf8<'a>(raw: &'a str, max_bytes: usize, hints: &mut Vec<String>) -> &'a str {
    if raw.len() <= max_bytes {
        return raw;
    }

    let mut boundary = max_bytes;
    while boundary > 0 && !raw.is_char_boundary(boundary) {
        boundary -= 1;
    }
    hints.push("input_truncated".to_string());
    &raw[..boundary]
}

fn sanitize_char(ch: char, hints: &mut Vec<String>) -> Option<char> {
    if is_zero_width(ch) {
        hints.push("zero_width_stripped".to_string());
        return None;
    }
    if is_bidi_control(ch) {
        hints.push("bidi_control_stripped".to_string());
        return None;
    }
    Some(ch)
}

fn is_zero_width(ch: char) -> bool {
    matches!(
        ch,
        '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{2060}' | '\u{FEFF}'
    )
}

fn is_bidi_control(ch: char) -> bool {
    matches!(
        ch,
        '\u{202A}'
            | '\u{202B}'
            | '\u{202C}'
            | '\u{202D}'
            | '\u{202E}'
            | '\u{2066}'
            | '\u{2067}'
            | '\u{2068}'
            | '\u{2069}'
    )
}
