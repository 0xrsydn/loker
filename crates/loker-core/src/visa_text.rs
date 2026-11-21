//! Visa sponsorship read from a job description. Only what the posting says
//! about sponsorship counts: "must be authorized to work in the US" alone,
//! a citizenship requirement or an equal opportunity statement says nothing
//! about it, and nothing is guessed. A wrong "not offered" turns a job away
//! from someone who could take it, so a sentence that doesn't read clearly is
//! left out. `eval/visa` measures it against labelled jobs.
use std::sync::LazyLock;

use regex::Regex;

use crate::model::{Sponsorship, Visa};
use crate::posting::{Posting, sentences};

/// The sentence is about visas or permission to work, not an event sponsor
/// or employer-sponsored insurance.
static CONTEXT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bvisas?\b|work permits?|\bH-?1-?B\b|\bimmigration\b|(?:company|employer|employment|work)\s+sponsorship|(?:authori[sz]\w*|eligib\w*|permitted|able) to work\b|right to work\b|work authori[sz]ation").unwrap()
});

/// Sponsoring, or helping with the visa.
static SPONSOR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)sponsor|visa\s+(?:support|assistance|process\w*|help|application\w*)|(?:support|help|assist)\w*\s+(?:with\s+)?(?:your\s+|the\s+|a\s+)?(?:visa|immigration|work permit)|(?:support|help|assist)\w*\s+(?:with|for)\s+[^.]{0,40}?\b(?:visas?|immigration|work permits?)\b|\bvisas?\s*(?:&|and|/)\s*relocation").unwrap()
});

static NOT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:not|no|unable|cannot|without|never|nor)\b|n['’]t\b").unwrap()
});

static MAYBE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:may|might|case[- ]by[- ]case|certain|some|limited|eligible candidates|where possible|depending|considered)\b").unwrap()
});

/// An offer: "available", "we provide", "we'll cover your visa".
static OFFER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:yes|available|offer\w*|provid\w*|support\w*|cover\w*|includ\w*|assist\w*)\b|\b(?:we|will|can|able to|happy to)\s+sponsor|\bsponsor\s+(?:your|you|visas|work visas|H-?1-?B)|\bsponsorship\s+for\s+(?:you|your)\b").unwrap()
});

/// Not a statement of policy: "information was not provided", an
/// application question, or sponsorship as a duty of the role.
static SKIP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:information|details|policy)\b[^.]{0,30}\bnot\s+(?:provided|specified|stated|listed|mentioned)|\b(?:do|will|would) you\b|\b(?:manag|administer|coordinat|handl|oversee|track)\w*\b[^.]{0,40}\b(?:visa|immigration|sponsorship)|\b(?:experience|knowledge)\b[^.]{0,60}\b(?:visa|immigration|sponsorship)").unwrap()
});

static NEED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(?:requir|need)\w*").unwrap());

/// "for those who need it": who the offer is for, not a condition.
static WHO_NEED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bwho\s+(?:need|require)s?\s+(?:it|one|them|this)\b").unwrap()
});

/// "can support visa transfers but will not sponsor ...": transfers only.
static TRANSFER_OK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:can|will|able to|happy to)\s+(?:support|sponsor|accept|assist with)\s+(?:\S+\s+){0,2}?transfers?\b").unwrap()
});

/// "(not Spain)": a place left out of an offer.
static PAREN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\([^)]*\)").unwrap());

/// "Health insurance, visa sponsorship, 401k": an item in a list of what
/// the job offers.
static LIST_ITEM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:^|[,;:]\s*)(?:full\s+)?visa sponsorship\s*(?:[,;.]|$)").unwrap()
});

/// "### Visa sponsorship:" alone on a line, its answer on the next.
static FIELD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^[#\s]*visa sponsorship\s*:?$").unwrap());

/// Visa sponsorship stated in `text` (a description), or `None`.
pub fn visa_from_text(text: &str) -> Option<Visa> {
    read(&Posting::parse(text))
}

/// Visa sponsorship a parsed posting states, or `None`.
pub fn read(posting: &Posting) -> Option<Visa> {
    let mut found: Vec<(Sponsorship, String)> = Vec::new();
    let mut lines = posting.lines.iter().peekable();
    while let Some(line) = lines.next() {
        // A heading and its answer on the next line: "### Visa sponsorship:" "No".
        let text = match lines.peek() {
            Some(next) if FIELD.is_match(&line.text) => {
                let joined = format!("{} {}", line.text, next.text);
                lines.next();
                joined
            }
            _ => line.text.clone(),
        };
        // A question ("Do you need sponsorship?") states nothing.
        for sentence in sentences(&text).into_iter().filter(|s| !s.ends_with('?')) {
            if let Some(s) = read_sentence(sentence)
                && !found.iter().any(|(_, e)| e == sentence)
            {
                found.push((s, sentence.to_string()));
            }
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

fn read_sentence(sentence: &str) -> Option<Sponsorship> {
    if sentence.chars().count() > 400 || !CONTEXT.is_match(sentence) || SKIP.is_match(sentence) {
        return None;
    }
    let cue = SPONSOR.find(sentence)?;
    if TRANSFER_OK.is_match(sentence) {
        return Some(Sponsorship::Limited);
    }
    // "We can sponsor visas for the UK (not Spain)": an offer with places left out.
    let bare = PAREN.replace_all(sentence, "");
    if PAREN.find_iter(sentence).any(|p| NOT.is_match(p.as_str()))
        && let Some(c) = SPONSOR.find(&bare)
    {
        let near = window(&bare, c.start(), c.end(), 90, 60);
        if !NOT.is_match(near) && OFFER.is_match(near) {
            return Some(Sponsorship::Limited);
        }
    }
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
    if NEED.is_match(near) && !WHO_NEED.is_match(near) {
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
