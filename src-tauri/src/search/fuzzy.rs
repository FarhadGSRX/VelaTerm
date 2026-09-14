//! Shared fuzzy text matching for the knowledge searches.
//!
//! Fuzzy tolerance targets ASCII words, where abbreviations and typos are common: a term matches a
//! word by ordered subsequence, by the initials of consecutive words, or within a bounded
//! Damerau-Levenshtein distance ("optimal string alignment" — insertions, deletions, substitutions
//! and adjacent transpositions each cost one). Chinese and other non-ASCII terms stay on exact
//! substring matching, where character-level distance would match almost anything.
//!
//! Callers run exact matching first and fall back to these helpers only when nothing matched, so
//! the strict path keeps its precision and `fuzzy` marks the loosened page.

/// Shortest term that may match by ordered subsequence.
const SUBSEQUENCE_MIN: usize = 3;
/// Shortest term that tolerates one edit.
const ONE_EDIT_MIN: usize = 4;
/// Shortest term that tolerates two edits.
const TWO_EDITS_MIN: usize = 8;

/// Whether a term may be matched fuzzily at all: ASCII alphanumeric, at least two characters.
pub fn is_fuzzy_term(term: &str) -> bool {
    term.len() >= 2 && term.chars().all(|c| c.is_ascii_alphanumeric())
}

/// Whether `word` matches `term` as an abbreviation or a typo. Both inputs are expected lowercased.
pub fn fuzzy_word(term: &str, word: &str) -> bool {
    if !is_fuzzy_term(term) {
        return false;
    }
    if term.len() >= SUBSEQUENCE_MIN && is_subsequence(term, word) {
        return true;
    }
    within_edits(term, word, allowed_edits(term.len()))
}

/// Whether any word of `text` matches `term` fuzzily, or the words' initials do (`kb` matches
/// `knowledge base`). Used for names, paths and titles, where words are short.
pub fn fuzzy_text(term: &str, text: &str) -> bool {
    if !is_fuzzy_term(term) {
        return false;
    }
    let lower = text.to_lowercase();
    lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|word| !word.is_empty() && fuzzy_word(term, word))
        || initials_match(term, &lower)
}

/// Ordered subsequence: every character of `needle` appears in `haystack` in order.
fn is_subsequence(needle: &str, haystack: &str) -> bool {
    let mut chars = haystack.chars();
    needle.chars().all(|want| chars.any(|have| have == want))
}

fn allowed_edits(len: usize) -> usize {
    if len >= TWO_EDITS_MIN {
        2
    } else if len >= ONE_EDIT_MIN {
        1
    } else {
        0
    }
}

/// Bounded optimal string alignment distance. `max` caps the computed value so an unrelated word
/// exits as soon as its whole row exceeds the budget instead of paying the full matrix.
fn within_edits(a: &str, b: &str, max: usize) -> bool {
    if max == 0 {
        return a == b;
    }
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len().abs_diff(b.len()) > max {
        return false;
    }
    // The swap chain keeps `prev2` = row i-2, `prev` = row i-1, `current` = the scratch row.
    let mut prev2 = vec![0usize; b.len() + 1];
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        current[0] = i;
        let mut row_min = current[0];
        for j in 1..=b.len() {
            let substitution = usize::from(a[i - 1] != b[j - 1]);
            let mut value = (prev[j] + 1)
                .min(current[j - 1] + 1)
                .min(prev[j - 1] + substitution);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                value = value.min(prev2[j - 2] + 1);
            }
            current[j] = value;
            row_min = row_min.min(value);
        }
        if row_min > max {
            return false;
        }
        std::mem::swap(&mut prev2, &mut prev);
        std::mem::swap(&mut prev, &mut current);
    }
    prev[b.len()] <= max
}

/// Word-initial abbreviation: `kb` matches `knowledge base`, `gtd` matches `getting things done`.
fn initials_match(term: &str, text: &str) -> bool {
    let mut wanted = term.chars();
    let mut next = wanted.next();
    for word in text
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
    {
        let Some(ch) = next else { break };
        if word
            .chars()
            .next()
            .is_some_and(|c| c.eq_ignore_ascii_case(&ch))
        {
            next = wanted.next();
        }
    }
    next.is_none()
}

/// The original word of `text` that matches `term` fuzzily, for snippet highlighting. Word-initial
/// abbreviations have no single literal to mark and return `None`.
pub fn fuzzy_word_in(term: &str, text: &str) -> Option<String> {
    if !is_fuzzy_term(term) {
        return None;
    }
    for (word, _) in words_with_spans(text) {
        if fuzzy_word(term, &word.to_lowercase()) {
            return Some(word.to_string());
        }
    }
    None
}

/// First occurrence of any term in an already lowercased line.
pub fn first_hit(lowered: &str, terms: &[&str]) -> Option<usize> {
    terms.iter().filter_map(|term| lowered.find(*term)).min()
}

/// First line containing any term, clipped around the hit: one-based line number and snippet.
pub fn exact_line(content: &str, lowered: &str, terms: &[&str]) -> Option<(usize, String)> {
    content
        .lines()
        .zip(lowered.lines())
        .enumerate()
        .find_map(|(index, (raw, low))| {
            first_hit(low, terms).map(|offset| {
                (
                    index + 1,
                    clip(raw, low[..offset].chars().count(), 200),
                )
            })
        })
}

/// First fuzzy word hit in `content`: one-based line number, original word and byte offset in the
/// raw line. Callers use the offset to clip the snippet around the word.
pub fn word_hit(content: &str, term: &str) -> Option<(usize, String, usize)> {
    for (index, line) in content.lines().enumerate() {
        for (word, offset) in words_with_spans(line) {
            if fuzzy_word(term, &word.to_lowercase()) {
                return Some((index + 1, word.to_string(), offset));
            }
        }
    }
    None
}

/// ASCII word runs with their byte offsets in `line`.
pub fn words_with_spans(line: &str) -> Vec<(&str, usize)> {
    let mut words = Vec::new();
    let mut start: Option<usize> = None;
    for (i, c) in line.char_indices() {
        if c.is_ascii_alphanumeric() {
            if start.is_none() {
                start = Some(i);
            }
        } else if let Some(s) = start.take() {
            words.push((&line[s..i], s));
        }
    }
    if let Some(s) = start {
        words.push((&line[s..], s));
    }
    words
}

/// Window of at most `limit` characters around `hit` (a character index), marked with `…` where the
/// line was trimmed. Shared by every knowledge snippet so the clipping math stays in one place.
pub fn clip(line: &str, hit: usize, limit: usize) -> String {
    let chars: Vec<char> = line.chars().collect();
    if chars.len() <= limit {
        return line.trim().to_string();
    }
    let hit = hit.min(chars.len() - 1);
    let start = hit
        .saturating_sub(limit / 3)
        .min(chars.len().saturating_sub(limit));
    let end = start + limit;
    // Reserve one character per trimmed end so the snippet never exceeds `limit`.
    let room = limit.saturating_sub(usize::from(start > 0) + usize::from(end < chars.len()));
    let mut start = start;
    let mut end = end;
    if end - start > room {
        end = (start + room).max(hit + 1).min(chars.len());
        start = end.saturating_sub(room);
    }
    let mut text = String::new();
    if start > 0 {
        text.push('…');
    }
    text.extend(&chars[start..end]);
    if end < chars.len() {
        text.push('…');
    }
    text
}

/// First non-empty line, used when only a name, path or title matched.
pub fn leading_line(content: &str, limit: usize) -> String {
    content
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
        .chars()
        .take(limit)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abbreviations_match_by_subsequence_and_initials() {
        assert!(fuzzy_word("knwl", "knowledge"));
        assert!(fuzzy_text("kb", "knowledge-base.md"));
        assert!(fuzzy_text("gtd", "Getting Things Done"));
        assert!(!fuzzy_word("kb", "knowledge"));
        assert!(!fuzzy_text("xyz", "knowledge base"));
        // Short terms and non-ASCII stay exact: they cannot match by subsequence.
        assert!(!fuzzy_word("kb", "backtick"));
        assert!(!fuzzy_text("知识", "知 识"));
    }

    #[test]
    fn typos_match_within_the_bounded_distance() {
        assert!(fuzzy_word("knoledge", "knowledge"));
        assert!(fuzzy_word("recieve", "receive"));
        assert!(fuzzy_word("throttel", "throttle"));
        assert!(!fuzzy_word("kon", "knowledge"));
        assert!(!fuzzy_word("completely", "different"));
    }

    #[test]
    fn words_and_clipping_keep_offsets_and_bounds() {
        let words: Vec<_> = words_with_spans("Tea brewing, 90\u{b0}C!").into_iter().map(|(w, _)| w).collect();
        assert_eq!(words, vec!["Tea", "brewing", "90", "C"]);
        assert_eq!(words_with_spans("tea").len(), 1);
        let line = format!("{} needle {}", "x".repeat(300), "y".repeat(300));
        let clipped = clip(&line, 301, 200);
        assert!(clipped.starts_with('…') && clipped.ends_with('…'));
        assert!(clipped.contains("needle"));
        assert!(clipped.chars().count() <= 200);
        assert_eq!(clip("short line", 0, 200), "short line");
    }
}
