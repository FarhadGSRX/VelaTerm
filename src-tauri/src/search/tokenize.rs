//! Word segmentation for the word-level search index (`session_words`).
//!
//! FTS5's unicode61 tokenizer keeps any unbroken run of letters as one token, so Chinese prose would index
//! as a single token per sentence. This module runs jieba over each fragment and marks word boundaries with
//! U+200B (zero-width space), which unicode61 treats as a separator. The same segmentation turns a query
//! term into a phrase of words, so `搜索` matches the word `搜索` but not the run `总和服务`, which the
//! trigram index would also accept.
//!
//! Stripping U+200B from the stored column restores the original fragment text byte for byte, which is what
//! lets [`parse_highlight`] map FTS5 `highlight()` output straight onto the text for snippets and matched
//! literals.

use std::sync::LazyLock;

use jieba_rs::Jieba;

/// Word-boundary marker inserted into the indexed column; a separator for unicode61, invisible otherwise.
pub const BOUNDARY: char = '\u{200B}';
/// Markers passed to FTS5 `highlight()`; control characters cannot appear in transcript text.
pub const MARK_OPEN: &str = "\u{1}";
pub const MARK_CLOSE: &str = "\u{2}";

/// Shared segmenter, built on first use. Loading the bundled dictionary takes about 80 ms in a release build
/// and holds roughly 55 MB resident, so it happens once and only when search is used.
static JIEBA: LazyLock<Jieba> = LazyLock::new(Jieba::new);

/// Whether a token carries searchable content; pure punctuation and whitespace never become query words.
fn is_word(tok: &str) -> bool {
    tok.chars().any(char::is_alphanumeric)
}

/// Text with [`BOUNDARY`] inserted before every jieba token except the first. Characters jieba skips are
/// kept, so removing the markers yields the input unchanged.
pub fn boundary_marked(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let tokens = JIEBA.cut(text, true);
    let mut starts: Vec<usize> = tokens.iter().map(|t| t.start).collect();
    starts.sort_unstable();
    starts.dedup();
    let mut out = String::with_capacity(text.len() + starts.len() * 3);
    let mut next = starts.iter().copied().filter(|&s| s > 0).peekable();
    for (i, ch) in text.chars().enumerate() {
        if next.peek() == Some(&i) {
            out.push(BOUNDARY);
            next.next();
        }
        out.push(ch);
    }
    out
}

/// Turns one query term into an FTS5 phrase of its jieba words, or None when the term has no searchable
/// word at all (pure punctuation), in which case the caller falls back to the substring index.
pub fn query_phrase(term: &str) -> Option<String> {
    let words: Vec<&str> = JIEBA
        .cut(term, true)
        .into_iter()
        .map(|t| t.word)
        .filter(|w| is_word(w))
        .collect();
    if words.is_empty() {
        return None;
    }
    Some(format!("\"{}\"", words.join(" ").replace('"', "\"\"")))
}

/// Extract a small set of useful search terms from a natural-language question.
///
/// `vsearch` itself retains its exact AND semantics. This helper is for `vrefer --ask`, which searches
/// each term independently in the already resolved target session and uses those original snippets to
/// ground an answer made from a compressed transcript.
pub fn query_keywords(question: &str) -> Vec<String> {
    const STOP: &[&str] = &[
        "a",
        "an",
        "and",
        "are",
        "did",
        "do",
        "does",
        "for",
        "from",
        "how",
        "in",
        "is",
        "it",
        "of",
        "on",
        "or",
        "that",
        "the",
        "this",
        "to",
        "was",
        "were",
        "what",
        "when",
        "where",
        "which",
        "who",
        "why",
        "with",
        "了",
        "什么",
        "怎么",
        "怎样",
        "为什么",
        "如何",
        "是否",
        "是",
        "的",
        "在",
        "和",
        "与",
        "或",
        "从",
        "中",
        "里",
        "呢",
        "吗",
        "请",
    ];
    let mut out = Vec::new();
    for token in JIEBA.cut(question, true) {
        let word = token
            .word
            .trim_matches(|c: char| !c.is_alphanumeric() && !"-_./:@+#".contains(c));
        if !is_word(word) {
            continue;
        }
        let normalized = word.to_lowercase();
        let length = word.chars().count();
        let ascii = word.is_ascii();
        if STOP.contains(&normalized.as_str()) || (ascii && length < 2) || (!ascii && length < 2) {
            continue;
        }
        if !out.iter().any(|item| item == &normalized) {
            out.push(normalized);
            if out.len() == 8 {
                break;
            }
        }
    }
    out
}

/// Result of decoding FTS5 `highlight()` output over the boundary-marked column.
pub struct Highlighted {
    /// Original fragment text with boundary and highlight markers removed.
    pub text: String,
    /// Byte ranges in `text` that FTS5 marked as matching, in document order.
    pub spans: Vec<(usize, usize)>,
}

/// Whether the text between two marked words is part of one identifier, so the two are one literal.
/// Empty means adjacent CJK words of a phrase; a short run of identifier punctuation joins `vlx-term`,
/// `a.b`, `x/y`. Anything with whitespace or other punctuation keeps the words separate.
fn joins_words(gap: &str) -> bool {
    gap.chars().count() <= 2 && gap.chars().all(|c| "-_./:@+#".contains(c))
}

/// Strips [`BOUNDARY`] and the highlight markers, recording each marked range as a byte span in the
/// restored text. FTS5 marks every word separately; adjacent words and words joined by identifier
/// punctuation are merged, so a phrase such as `终端搜索` or an identifier such as `vlx-term` comes back as
/// one literal.
pub fn parse_highlight(marked: &str) -> Highlighted {
    let mut text = String::with_capacity(marked.len());
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let mut open_at: Option<usize> = None;
    for ch in marked.chars() {
        match ch {
            BOUNDARY => {}
            '\u{1}' => open_at = Some(text.len()),
            '\u{2}' => {
                if let Some(start) = open_at.take() {
                    if text.len() > start {
                        spans.push((start, text.len()));
                    }
                }
            }
            _ => text.push(ch),
        }
    }
    let mut merged: Vec<(usize, usize)> = Vec::with_capacity(spans.len());
    for (a, b) in spans {
        match merged.last_mut() {
            Some(last) if joins_words(&text[last.1..a]) => last.1 = b,
            _ => merged.push((a, b)),
        }
    }
    Highlighted {
        text,
        spans: merged,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundary_marks_split_cjk_words_and_restore_input() {
        let marked = boundary_marked("打开终端搜索功能 vlx-term");
        let restored: String = marked.chars().filter(|&c| c != BOUNDARY).collect();
        assert_eq!(restored, "打开终端搜索功能 vlx-term");
        let words: Vec<&str> = marked.split(BOUNDARY).collect();
        assert!(words.contains(&"终端"), "{words:?}");
        assert!(words.contains(&"搜索"), "{words:?}");
    }

    #[test]
    fn query_phrase_segments_terms_and_rejects_punctuation() {
        assert_eq!(query_phrase("终端搜索").as_deref(), Some("\"终端 搜索\""));
        assert_eq!(query_phrase("vlx-term").as_deref(), Some("\"vlx term\""));
        assert_eq!(query_phrase("\"quoted\"").as_deref(), Some("\"quoted\""));
        assert_eq!(query_phrase("..."), None);
    }

    #[test]
    fn question_keywords_drop_filler_and_keep_searchable_terms() {
        assert_eq!(
            query_keywords("vrefer 的总结机制是怎么工作的？"),
            vec!["vrefer", "总结", "机制", "工作"]
        );
        assert_eq!(
            query_keywords("How did output scheduler throttling work?"),
            vec!["output", "scheduler", "throttling", "work"]
        );
    }

    #[test]
    fn parse_highlight_restores_text_and_merges_adjacent_spans() {
        let h = parse_highlight("打开\u{200B}\u{1}终端\u{2}\u{200B}\u{1}搜索\u{2}\u{200B}功能");
        assert_eq!(h.text, "打开终端搜索功能");
        assert_eq!(h.spans, vec![(6, 18)]);
        assert_eq!(&h.text[6..18], "终端搜索");

        let h =
            parse_highlight("open \u{1}vlx\u{2}\u{200B}-\u{200B}\u{1}term\u{2} and \u{1}vlx\u{2}");
        assert_eq!(h.text, "open vlx-term and vlx");
        assert_eq!(h.spans, vec![(5, 13), (18, 21)]);

        // Words separated by whitespace or sentence punctuation stay separate literals.
        let h = parse_highlight("\u{1}会话\u{2}）**：\u{1}会话\u{2} \u{1}搜索\u{2}");
        assert_eq!(h.text, "会话）**：会话 搜索");
        assert_eq!(h.spans.len(), 3);
    }
}
