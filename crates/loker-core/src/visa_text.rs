//! Visa sponsorship read from a job description. Only what the posting says
//! about sponsorship counts: "must be authorized to work in the US" alone,
//! a citizenship requirement or an equal opportunity statement says nothing
//! about it, and nothing is guessed. A wrong "not offered" turns a job away
//! from someone who could take it, so a sentence that doesn't read clearly is
//! left out. `eval/visa` measures it against labelled jobs.
use std::sync::LazyLock;

use regex::Regex;

use crate::model::{Sponsorship, Visa};

/// The sentence is about visas or permission to work, not an event sponsor
/// or employer-sponsored insurance.
static CONTEXT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bvisas?\b|work permits?|\bH-?1-?B\b|\bimmigration\b|(?:company|employer|employment|work)\s+sponsorship|(?:authori[sz]\w*|eligib\w*|permitted|able) to work\b|work authori[sz]ation").unwrap()
});

/// Sponsoring, or helping with the visa.
static SPONSOR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)sponsor|visa\s+(?:support|assistance|process\w*|help|application\w*)|(?:support|help|assist)\w*\s+(?:with\s+)?(?:your\s+|the\s+|a\s+)?(?:visa|immigration|work permit)").unwrap()
});

static NOT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:not|no|unable|cannot|without|never|nor)\b|n['’]t\b").unwrap()
});

static MAYBE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:may|might|case[- ]by[- ]case|certain|some|limited|eligible candidates|where possible|depending|considered)\b").unwrap()
});

/// An offer: "available", "we provide", "we'll cover your visa".
static OFFER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:yes|available|offer\w*|provid\w*|support\w*|cover\w*|includ\w*|assist\w*)\b|\b(?:we|will|can|able to|happy to)\s+sponsor|\bsponsor\s+(?:your|you|visas|work visas|H-?1-?B)").unwrap()
});

/// Not a statement of policy: "information was not provided", an
/// application question, or sponsorship as a duty of the role.
static SKIP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:information|details|policy)\b[^.]{0,30}\bnot\s+(?:provided|specified|stated|listed|mentioned)|\b(?:do|will|would) you\b|\b(?:manag|administer|coordinat|handl|oversee|track)\w*\b[^.]{0,40}\b(?:visa|immigration|sponsorship)").unwrap()
});

static NEED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(?:requir|need)\w*").unwrap());

/// "Health insurance, visa sponsorship, 401k": an item in a list of what
/// the job offers.
static LIST_ITEM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:^|[,;:]\s*)(?:full\s+)?visa sponsorship\s*(?:[,;.]|$)").unwrap()
});

static FIELD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?im)^([#\s]*visa sponsorship\s*:?)[ \t]*\n\s*").unwrap());

/// Visa sponsorship stated in `text` (a description), or `None`.
pub fn visa_from_text(text: &str) -> Option<Visa> {
    let text = text.replace("\\-", "-").replace(['*', '_'], "");
    // A heading and its answer on the next line: "### Visa sponsorship:" "No".
    let text = FIELD.replace_all(&text, "$1 ");
    let mut found: Vec<(Sponsorship, String)> = Vec::new();
    for sentence in sentences(&text) {
        if let Some(s) = read(sentence)
            && !found.iter().any(|(_, e)| e == sentence)
        {
            found.push((s, sentence.to_string()));
        }
    }
    let first = found.first()?.0;
    // Sentences that disagree ("no sponsorship outside the US", "visa
    // sponsorship for eligible candidates") mean some cases only.
    let sponsorship = if found.iter().all(|(s, _)| *s == first) {
        first
    } else {
        Sponsorship::Limited
    };
    let evidence: Vec<&str> = found.iter().map(|(_, e)| e.as_str()).collect();
    Some(Visa {
        sponsorship,
        evidence: evidence.join(" … ").chars().take(300).collect(),
    })
}

fn read(sentence: &str) -> Option<Sponsorship> {
    if sentence.chars().count() > 400 || !CONTEXT.is_match(sentence) || SKIP.is_match(sentence) {
        return None;
    }
    let cue = SPONSOR.find(sentence)?;
    // Negation and hedges near the cue, not anywhere in a long sentence.
    let near = window(sentence, cue.start(), cue.end(), 90, 60);
    let (not, maybe) = (NOT.is_match(near), MAYBE.is_match(near));
    // "may not be able to sponsor" is a hedge, not a no.
    if maybe {
        return Some(Sponsorship::Limited);
    }
    if not {
        return Some(Sponsorship::NotOffered);
    }
    // "Candidates requiring sponsorship ..." without a no is a question or a
    // condition, not an offer.
    if NEED.is_match(near) {
        return None;
    }
    if OFFER.is_match(near) || LIST_ITEM.is_match(sentence) {
        // "for candidates with an H-1B visa eligible for transfer": transfers only.
        return Some(if sentence.to_lowercase().contains("transfer") {
            Sponsorship::Limited
        } else {
            Sponsorship::Offered
        });
    }
    None
}

fn window(s: &str, start: usize, end: usize, before: usize, after: usize) -> &str {
    let mut a = start.saturating_sub(before);
    while !s.is_char_boundary(a) {
        a -= 1;
    }
    let mut b = (end + after).min(s.len());
    while !s.is_char_boundary(b) {
        b += 1;
    }
    &s[a..b]
}

/// Lines, split at sentence ends. An initialism ("U.S.") is not an end,
/// and a question is left out.
fn sentences(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    for line in text.lines() {
        let b = line.as_bytes();
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
    }
    out.into_iter()
        .filter(|s| !s.trim_end().ends_with('?'))
        .map(|s| {
            s.trim()
                .trim_start_matches(['#', '>', '-', '•', ' '])
                .trim()
        })
        .filter(|s| !s.is_empty())
        .collect()
}
