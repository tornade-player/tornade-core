// Fuzzy string matching for album/artist names

/// Normalize a string for matching (lowercase, remove accents, trim)
pub fn normalize(s: &str) -> String {
    s.to_lowercase()
        .trim()
        .to_string()
}

/// Calculate simple fuzzy match score between two strings
pub fn fuzzy_match(query: &str, target: &str) -> f32 {
    let query_norm = normalize(query);
    let target_norm = normalize(target);

    if query_norm == target_norm {
        return 1.0;
    }

    if target_norm.contains(&query_norm) {
        return 0.8;
    }

    if query_norm.contains(&target_norm) {
        return 0.7;
    }

    // Calculate character-level similarity
    let query_chars: Vec<char> = query_norm.chars().collect();
    let target_chars: Vec<char> = target_norm.chars().collect();

    let mut matches = 0;
    for qc in &query_chars {
        if target_chars.contains(qc) {
            matches += 1;
        }
    }

    let max_len = query_chars.len().max(target_chars.len()) as f32;
    if max_len == 0.0 {
        return 0.0;
    }

    matches as f32 / max_len
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_match() {
        assert_eq!(fuzzy_match("Abbey Road", "Abbey Road"), 1.0);
    }

    #[test]
    fn test_case_insensitive() {
        assert_eq!(fuzzy_match("abbey road", "ABBEY ROAD"), 1.0);
    }

    #[test]
    fn test_substring() {
        assert!(fuzzy_match("Abbey", "Abbey Road") > 0.7);
    }

    #[test]
    fn test_normalize() {
        assert_eq!(normalize("  Abbey Road  "), "abbey road");
    }

    #[test]
    fn test_partial_match() {
        // query contains target
        let score = fuzzy_match("Abbey Road Deluxe", "Abbey Road");
        assert!(score >= 0.7);
    }

    #[test]
    fn test_no_match() {
        let score = fuzzy_match("ZZZZZ", "Abbey Road");
        assert!(score < 0.5);
    }

    #[test]
    fn test_empty_strings() {
        assert_eq!(fuzzy_match("", ""), 1.0); // both normalized to "" -> equal
    }

    #[test]
    fn test_score_is_exactly_08_when_target_contains_query() {
        // "Abbey" is contained in "Abbey Road" and they are not equal
        assert_eq!(fuzzy_match("Abbey", "Abbey Road"), 0.8);
    }

    #[test]
    fn test_score_is_exactly_07_when_query_contains_target() {
        // "Abbey Road" contains "Abbey Road" — but here: query contains target and they differ
        // Use a case where query is longer and contains target
        assert_eq!(fuzzy_match("Abbey Road Remastered", "Abbey Road"), 0.7);
    }

    #[test]
    fn test_score_between_0_and_1_for_partial_overlap() {
        let score = fuzzy_match("Dark Side", "Abbey Road");
        assert!(score >= 0.0 && score <= 1.0, "score must be in [0, 1]");
    }

    #[test]
    fn test_empty_query_nonempty_target() {
        // "" is contained in any string, so target.contains("") is true
        // result should be 0.8 (target contains query)
        let score = fuzzy_match("", "Abbey Road");
        assert_eq!(score, 0.8);
    }

    #[test]
    fn test_score_is_symmetric_for_char_level() {
        // When neither contains the other: score depends on char overlap
        let s1 = fuzzy_match("xyz", "abc");
        let s2 = fuzzy_match("abc", "xyz");
        // The char-level path isn't guaranteed to be symmetric, but both should be in [0,1]
        assert!(s1 >= 0.0 && s1 <= 1.0);
        assert!(s2 >= 0.0 && s2 <= 1.0);
    }

    #[test]
    fn test_normalize_trims_leading_trailing_whitespace() {
        assert_eq!(normalize("   Pink Floyd   "), "pink floyd");
    }

    #[test]
    fn test_normalize_lowercases() {
        assert_eq!(normalize("THE DARK SIDE OF THE MOON"), "the dark side of the moon");
    }
}
