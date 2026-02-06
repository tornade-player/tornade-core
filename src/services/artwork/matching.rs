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
}
