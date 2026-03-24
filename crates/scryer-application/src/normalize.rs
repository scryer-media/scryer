/// Normalize an IMDb ID to the canonical `tt{digits}` format.
///
/// Accepts any of: "tt1234567", "1234567", "tt1234567abc".
/// Returns `None` for empty strings or strings with no digits.
pub(crate) fn normalize_imdb_id(raw: &str) -> Option<String> {
    let value = raw.trim();
    if value.is_empty() {
        return None;
    }

    if let Some(tt_index) = value.to_ascii_lowercase().find("tt") {
        let digits: String = value[tt_index + 2..]
            .chars()
            .take_while(|ch| ch.is_ascii_digit())
            .collect();
        if !digits.is_empty() {
            return Some(format!("tt{digits}"));
        }
    }

    if value.chars().all(|ch| ch.is_ascii_digit()) {
        Some(format!("tt{value}"))
    } else {
        None
    }
}

/// Normalize a numeric external ID (TVDB, AniDB, etc.) by extracting digits.
pub(crate) fn normalize_numeric_id(raw: &str) -> Option<String> {
    let value = raw.trim();
    if value.is_empty() {
        return None;
    }
    let digits: String = value.chars().filter(|ch| ch.is_ascii_digit()).collect();
    if digits.is_empty() {
        None
    } else {
        Some(digits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imdb_id_with_prefix() {
        assert_eq!(normalize_imdb_id("tt1234567"), Some("tt1234567".to_string()));
    }

    #[test]
    fn imdb_id_digits_only() {
        assert_eq!(normalize_imdb_id("1234567"), Some("tt1234567".to_string()));
    }

    #[test]
    fn imdb_id_with_trailing_chars() {
        assert_eq!(normalize_imdb_id("tt0123456abc"), Some("tt0123456".to_string()));
    }

    #[test]
    fn imdb_id_empty() {
        assert_eq!(normalize_imdb_id(""), None);
    }

    #[test]
    fn imdb_id_whitespace_only() {
        assert_eq!(normalize_imdb_id("  "), None);
    }

    #[test]
    fn imdb_id_no_digits() {
        assert_eq!(normalize_imdb_id("abcdef"), None);
    }

    #[test]
    fn imdb_id_trimmed() {
        assert_eq!(normalize_imdb_id("  tt1234567  "), Some("tt1234567".to_string()));
    }

    #[test]
    fn numeric_id_simple() {
        assert_eq!(normalize_numeric_id("12345"), Some("12345".to_string()));
    }

    #[test]
    fn numeric_id_with_prefix() {
        assert_eq!(normalize_numeric_id("aid12345"), Some("12345".to_string()));
    }

    #[test]
    fn numeric_id_empty() {
        assert_eq!(normalize_numeric_id(""), None);
    }

    #[test]
    fn numeric_id_no_digits() {
        assert_eq!(normalize_numeric_id("abc"), None);
    }
}
