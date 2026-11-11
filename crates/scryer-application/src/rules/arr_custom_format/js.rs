//! JavaScript value semantics the Arr export format was written against.
//!
//! Sonarr and Radarr exports, and the rules already saved from them, were
//! produced with these exact string and number conventions, so the
//! translator keeps them rather than Rust's defaults.

/// `String(number)` for a finite or non-finite JavaScript number.
pub(super) fn js_number_to_string(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    if value == 0.0 {
        return "0".to_string();
    }
    let magnitude = value.abs();
    if (1e-6..1e21).contains(&magnitude) {
        return format!("{value}");
    }
    let formatted = format!("{value:e}");
    match formatted.split_once('e') {
        Some((mantissa, exponent)) if !exponent.starts_with('-') => {
            format!("{mantissa}e+{exponent}")
        }
        _ => formatted,
    }
}

pub(super) const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

/// `Number.isSafeInteger`.
pub(super) fn is_safe_integer(value: f64) -> bool {
    value.is_finite() && value.fract() == 0.0 && value.abs() <= MAX_SAFE_INTEGER
}

/// Whitespace removed by `String.prototype.trim` and matched by `\s`.
pub(super) fn is_js_whitespace(c: char) -> bool {
    matches!(
        c,
        '\u{0009}'
            | '\u{000A}'
            | '\u{000B}'
            | '\u{000C}'
            | '\u{000D}'
            | '\u{0020}'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

pub(super) fn js_trim(value: &str) -> &str {
    value.trim_matches(is_js_whitespace)
}

/// `string.length`: UTF-16 code units.
pub(super) fn utf16_len(value: &str) -> usize {
    value.chars().map(char::len_utf16).sum()
}

/// Property names every plain JavaScript object inherits. A pasted field
/// named after one of them was always refused as a duplicate, because the
/// duplicate check walked the prototype chain.
pub(super) const OBJECT_PROTOTYPE_NAMES: &[&str] = &[
    "constructor",
    "__defineGetter__",
    "__defineSetter__",
    "hasOwnProperty",
    "__lookupGetter__",
    "__lookupSetter__",
    "isPrototypeOf",
    "propertyIsEnumerable",
    "toString",
    "valueOf",
    "__proto__",
    "toLocaleString",
];

/// Whether a key is an array index, which plain objects enumerate first in
/// ascending numeric order.
fn array_index(key: &str) -> Option<u32> {
    if key.is_empty() || (key.len() > 1 && key.starts_with('0')) {
        return None;
    }
    if !key.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    key.parse::<u32>().ok().filter(|index| *index != u32::MAX)
}

/// Reorders object entries the way `Object.entries` enumerates them.
pub(super) fn object_order<T>(entries: Vec<(String, T)>) -> Vec<(String, T)> {
    let (mut indexed, named): (Vec<_>, Vec<_>) = entries
        .into_iter()
        .partition(|(key, _)| array_index(key).is_some());
    indexed.sort_by_key(|(key, _)| array_index(key));
    indexed.extend(named);
    indexed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_print_like_javascript() {
        let cases = [
            (0.0, "0"),
            (-0.0, "0"),
            (100.0, "100"),
            (-1.0, "-1"),
            (0.5, "0.5"),
            (0.75, "0.75"),
            (1_073_741_824.0, "1073741824"),
            (1e21, "1e+21"),
            (1.5e-7, "1.5e-7"),
            (0.000001, "0.000001"),
            (123_456_789_012_345_680_000.0, "123456789012345680000"),
            (f64::INFINITY, "Infinity"),
        ];
        for (value, expected) in cases {
            assert_eq!(js_number_to_string(value), expected, "{value}");
        }
    }

    #[test]
    fn trim_matches_javascript_whitespace() {
        assert_eq!(js_trim("\u{FEFF}\u{3000} name \n"), "name");
        assert_eq!(js_trim("\u{0085}name"), "\u{0085}name");
    }

    #[test]
    fn object_order_puts_array_indexes_first() {
        let ordered = object_order(vec![
            ("b".to_string(), 1),
            ("10".to_string(), 2),
            ("2".to_string(), 3),
            ("01".to_string(), 4),
        ]);
        let keys: Vec<_> = ordered.iter().map(|(key, _)| key.as_str()).collect();
        assert_eq!(keys, ["2", "10", "b", "01"]);
    }
}
