//! A job description parsed once, for the readers that take facts from it
//! (`experience_text`, `visa_text`, ...). Markdown escapes and emphasis are
//! removed, blank lines dropped, and each line knows whether it is a
//! heading, a list item or text, and what kind of section it sits in, so
//! "years under Nice to have" or a sentence split is decided in one place.
//! A reader's rules stay in its own module; this holds only what reading
//! any posting needs.
use std::sync::LazyLock;

use regex::Regex;

pub struct Posting {
    pub lines: Vec<Line>,
}

pub struct Line {
    /// The line with emphasis (`*`, `_`) removed and trimmed; list markers
    /// other than `*` stay ("- 5+ years").
    pub text: String,
    pub kind: LineKind,
    /// What the heading above says this part of the posting is.
    pub section: Section,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// A markdown heading, a bold line alone, or a short line ending in a
    /// colon. A reader may still read one ("Experience: 5+ years:").
    Heading,
    /// A bullet or numbered item.
    Item,
    Text,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// Under "Preferred", "Nice to have", "Bonus", ...: not required.
    Preferred,
    /// Under any other heading, or none.
    Other,
}

/// Not required: preferred, a bonus, ideally. For headings, and for readers
/// that look before a phrase ("Ideally, you have").
pub static PREFERRED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bprefer(?:red|ably|able)?\b|\bideally\b|\bnice[- ]to[- ]have\b|\bbonus\b|\ba plus\b|\bplus\s*:?\s*$|\bdesir(?:ed|able)\b|\badvantage(?:ous)?\b|\bgood to have\b|\bwould be great\b|\bstand out\b|\bextra credit\b|\bpluses\b").unwrap()
});

/// "Minimum and preferred qualifications": a heading over both, read as required.
static HEADING_REQUIRED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(?:minimum|required|requirements|must)\b").unwrap());

static ESCAPE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\([[:punct:]])").unwrap());

impl Posting {
    pub fn parse(text: &str) -> Posting {
        let text = ESCAPE.replace_all(text, "$1");
        let mut section = Section::Other;
        let mut lines = Vec::new();
        for raw in text.lines() {
            let bare = raw.replace(['*', '_'], "");
            let bare = bare.trim();
            if bare.is_empty() {
                continue;
            }
            let kind = kind(raw, bare);
            if kind == LineKind::Heading {
                // Any other heading ("Requirements", "About us") ends a preferred list.
                section = if PREFERRED.is_match(bare) && !HEADING_REQUIRED.is_match(bare) {
                    Section::Preferred
                } else {
                    Section::Other
                };
            }
            lines.push(Line {
                text: bare.to_string(),
                kind,
                section,
            });
        }
        Posting { lines }
    }
}

fn kind(raw: &str, bare: &str) -> LineKind {
    let t = raw.trim();
    let item = t.starts_with(['-', '•', '+', '*']) && !t.starts_with("**")
        || t.chars().next().is_some_and(|c| c.is_ascii_digit()) && t.contains(". ");
    if item {
        return LineKind::Item;
    }
    let heading = bare.chars().count() <= 70
        && (t.starts_with('#')
            || (t.starts_with("**") && t.trim_end_matches(':').ends_with("**"))
            || bare.ends_with(':'));
    if heading {
        LineKind::Heading
    } else {
        LineKind::Text
    }
}

/// A line split at sentence ends, each trimmed of heading and list marks.
/// An initialism ("U.S.") is not an end.
pub fn sentences(line: &str) -> Vec<&str> {
    let b = line.as_bytes();
    let mut out = Vec::new();
    let mut from = 0;
    for i in 0..b.len() {
        let end = matches!(b[i], b'.' | b'!' | b'?')
            && b.get(i + 1).is_none_or(|c| c.is_ascii_whitespace())
            && !(b[i] == b'.'
                && i >= 1
                && b[i - 1].is_ascii_uppercase()
                && (i < 2 || !b[i - 2].is_ascii_alphabetic()));
        if end {
            out.push(&line[from..=i]);
            from = i + 1;
        }
    }
    out.push(&line[from..]);
    out.into_iter()
        .map(|s| {
            s.trim()
                .trim_start_matches(['#', '>', '-', '•', '+', ' '])
                .trim()
        })
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_know_their_kind_and_section() {
        let p = Posting::parse(
            "## About us\nWe build tools\\.\n\n**Nice to have:**\n*   Go\n### Requirements\n- 3+ years",
        );
        let got: Vec<(&str, LineKind, Section)> = p
            .lines
            .iter()
            .map(|l| (l.text.as_str(), l.kind, l.section))
            .collect();
        assert_eq!(
            got,
            [
                ("## About us", LineKind::Heading, Section::Other),
                ("We build tools.", LineKind::Text, Section::Other),
                ("Nice to have:", LineKind::Heading, Section::Preferred),
                ("Go", LineKind::Item, Section::Preferred),
                ("### Requirements", LineKind::Heading, Section::Other),
                ("- 3+ years", LineKind::Item, Section::Other),
            ]
        );
    }

    #[test]
    fn sentences_keep_initialisms() {
        assert_eq!(
            sentences("Based in the U.S. only. Apply now!"),
            ["Based in the U.S. only.", "Apply now!"]
        );
    }
}
