use std::sync::LazyLock;

use regex::Regex;

use crate::model::{Experience, ExperienceMethod};
use crate::posting::{Gate, LineKind, PREFERRED, Posting, Section, sentences};

static YEAR_GATE: LazyLock<Gate> = LazyLock::new(|| Gate::new(&["year", "yr", "yoe"]));
static LINE_GATE: LazyLock<Gate> =
    LazyLock::new(|| Gate::new(&["year", "yr", "yoe", "experience", "grad"]));
static NONE_NEEDED_GATE: LazyLock<Gate> =
    LazyLock::new(|| Gate::new(&["year", "experience", "grad"]));
static NUMBER_GATE: LazyLock<Gate> = LazyLock::new(|| {
    Gate::new(&[
        "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven",
        "twelve", "fifteen",
    ])
});

fn may_state_years(sentence: &str) -> bool {
    YEAR_GATE.may_match(sentence)
        && (sentence
            .chars()
            .any(|c| c.is_ascii_digit() || (!c.is_ascii() && c.is_numeric()))
            || NUMBER_GATE.may_match(sentence))
}

const NUM: &str = r"\d{1,2}|one|two|three|four|five|six|seven|eight|nine|ten|eleven|twelve|fifteen";

static YEARS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?i)\b(?P<min>{NUM})(?:\s*\(\d{{1,2}}\))?\s*(?:\+|plus\b)?\s*(?:(?:-|–|—|~|to)\s*(?P<max>{NUM})\s*\+?\s*)?(?:or\s+more\s+)?(?:full\s+)?(?P<hy>-)?(?P<unit>years?|yrs?|yoe)\b'?"
    ))
    .unwrap()
});

static FIELD_AFTER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^\s*(?:of\s+)?(?:in|as|at|across|within|on|with|[a-z]+ing|leading|working|building|developing|managing|selling|running|owning|supporting|designing|delivering|programming|coding|writing|teaching|hands[- ]on|professional|relevant|industry|post[- ]\w+|progressive|full[- ]time)\b").unwrap()
});

static EXPERIENCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bexperience|\bexperienced\b|\bbackground\b|\btrack record\b|\b(?:working|worked|building|developing|leading|managing|selling|designing|writing|operating|running)\b|\bin\s+(?:a|an|the|similar|comparable|relevant|professional|industry|software|engineering|sales|product|data|tech|technical|b2b|saas|fintech|customer|marketing|finance|accounting|recruiting|it)\b|\bas\s+an?\b|\bof\s+(?:professional|relevant|related|hands[- ]on|industry|progressive|post|full[- ]time|practical|commercial|direct|proven|recent|demonstrated|prior|work|production)").unwrap()
});

static NOT_HIRE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?i)\b(?:for|since|over the (?:past|last)|in the (?:past|last)|within|after|every|first|next|ago)\s+(?:(?:over|more than|nearly|almost|about|the|a|up to)\s+)?(?:{NUM})\+?\s*years?\b|\bin\s+(?:less than|under|just|only)\s+(?:{NUM})\s+years?\b|\byears?\s+(?:old|of age|ago|in business|in a row|running|straight|of history|of operation|of growth|of innovation|of excellence|of service|of combined|combined|of research|of study|of school|of college|of university|of education|of schooling)\b|\bcombined\s+(?:experience|years)\b|\bdrawing on\b|\b(?:kept|retain(?:ed)?|stored|held)\b|\b(?:he|she|his|her)\b|\b(?:we|we've|we have|our|company|founded|established)\b[^.]{{0,40}}\b(?:{NUM})\+?\s*years\b(?:\s+of\s+(?:experience|expertise))?(?:\s+(?:in|of)\s+(?:business|the industry|the market))?\s*(?:,|we|our)"
    ))
    .unwrap()
});

/// Not required, said after the years: "5+ years in fintech preferred",
/// "is an asset". "Ideally" or "preferably" there qualifies the setting
/// ("8+ years, ideally at a fintech"), not the years.
static PREFERRED_AFTER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bpreferred\b|\bnice[- ]to[- ]have\b|\bbonus\b|\ba (?:big |huge |strong )?plus\b|\bdesir(?:ed|able)\b|\b(?:an?|big|real|great)\s+(?:advantage|asset)\b|\badvantageous\b|\bgood to have\b|\bwould be great\b|\bhelpful\b|\bbeneficial\b").unwrap()
});

static NONE_NEEDED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bno\s+(?:prior\s+|previous\s+|professional\s+|work\s+|formal\s+)*experience\s+(?:is\s+)?(?:required|needed|necessary)\b|\b(?:new|recent)\s+grad(?:uate)?s?\s+(?:are\s+)?(?:welcome|encouraged)|\b0\s*\+\s*years?\b|\b(?:do not|don't|doesn't|does not)\s+need\s+(?:any\s+)?(?:prior\s+|previous\s+|years\s+of\s+)?(?:\w+\s+)?experience\s+to\b").unwrap()
});

static OR_BETWEEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bor\b[^;]{0,60}\b(?:bachelor|master|ph\.?d|doctorate|degree|b\.?s\.?c?|b\.?a|m\.?s\.?c?|mba|diploma)\b|[,;]\s*or\s+[^,]{0,30}$").unwrap()
});

static SOMEONE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b[A-Z][a-z]+\s+(?:has|had)\s+(?:spent|been|worked|led)\b").unwrap()
});

static LEVEL_PREFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^for\s+[^:]{1,40}:").unwrap());

static OR_START: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^or\b").unwrap());

static A_YEAR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(at least|a minimum of|minimum of|more than|over)\s+a\s+year\b").unwrap()
});

static NEGATED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:\bnot\b|n['’]t\b|\bno\b|\bnever\b)\s+(?:\w+\s+){0,3}$").unwrap()
});

static AT_MOST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:less than|fewer than|no more than|not more than|under|up to|maximum(?: of)?|max\.?|at most)\s*$").unwrap()
});

static MONTHS_TO: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bmonths?\s*(?:to|-|–)\s*$").unwrap());

pub fn experience_from_text(text: &str) -> Option<Experience> {
    read(&Posting::parse(text))
}

/// Years of experience a parsed posting states the job requires, or `None`.
pub fn read(posting: &Posting) -> Option<Experience> {
    let mut required: Vec<Claim> = Vec::new();
    let mut none_needed: Option<String> = None;
    let mut last_had_claim = false;
    let mut levels = false;
    let mut level_block = false;
    for line in &posting.lines {
        let line_may_matter = LINE_GATE.may_match(&line.text);
        let bare = if line_may_matter {
            A_YEAR.replace_all(&line.text, "$1 1 year")
        } else {
            std::borrow::Cow::Borrowed(line.text.as_str())
        };
        let bare = bare.as_ref();
        if LEVEL_PREFIX.is_match(bare.trim_start_matches(['-', '•', '+', ' ']))
            && bare.trim_end().ends_with(':')
        {
            level_block = true;
            continue;
        }
        // A heading that states years ("Experience: 5+ years:") is read.
        if line.kind == LineKind::Heading && !(line_may_matter && YEARS.is_match(bare)) {
            level_block = false;
            continue;
        }
        let preferred_section = line.section == Section::Preferred;
        if !line_may_matter {
            if !preferred_section && !sentences(bare).is_empty() {
                last_had_claim = false;
            }
            continue;
        }
        for sentence in sentences(bare) {
            if none_needed.is_none()
                && NONE_NEEDED_GATE.may_match(sentence)
                && NONE_NEEDED.is_match(sentence)
            {
                none_needed = Some(sentence.to_string());
            }
            if preferred_section {
                continue;
            }
            let found = if !may_state_years(sentence) || SOMEONE.is_match(sentence) {
                Vec::new()
            } else {
                claims(sentence)
            };
            levels |= !found.is_empty() && (level_block || LEVEL_PREFIX.is_match(sentence));
            if !found.is_empty()
                && OR_START.is_match(sentence)
                && last_had_claim
                && let (Some(last), Some(first)) = (required.last_mut(), found.first())
            {
                if first.min < last.min {
                    *last = Claim {
                        min: first.min,
                        max: first.max,
                        evidence: first.evidence.clone(),
                    };
                }
                required.extend(found.into_iter().skip(1));
                last_had_claim = true;
                continue;
            }
            last_had_claim = !found.is_empty();
            required.extend(found);
        }
    }
    let top = if levels {
        required.iter().min_by_key(|c| c.min)
    } else {
        required.iter().max_by_key(|c| c.min)
    };
    if let Some(top) = top {
        return Some(Experience {
            min_years: top.min,
            max_years: top.max,
            method: ExperienceMethod::Description,
            evidence: top.evidence.clone(),
        });
    }
    let evidence = none_needed?;
    Some(Experience {
        min_years: 0,
        max_years: None,
        method: ExperienceMethod::Description,
        evidence: clip(&evidence),
    })
}

#[derive(Debug)]
struct Claim {
    min: u8,
    max: Option<u8>,
    evidence: String,
}

fn claims(sentence: &str) -> Vec<Claim> {
    if sentence.chars().count() > 500 {
        return Vec::new();
    }
    let found: Vec<_> = YEARS.captures_iter(sentence).collect();
    let mut out: Vec<Claim> = Vec::new();
    let mut prev_end = 0;
    let mut prev_kept = false;
    for (i, c) in found.iter().enumerate() {
        let m = c.get(0).unwrap();
        let next_start = found
            .get(i + 1)
            .map_or(sentence.len(), |n| n.get(0).unwrap().start());
        let after = &sentence[m.end()..next_start];
        let before = &sentence[prev_end..m.start()];
        let around = &sentence[floor(sentence, m.start().saturating_sub(60))
            ..ceil(sentence, (m.end() + 90).min(sentence.len()))];
        let gap_or = OR_BETWEEN.is_match(before);
        prev_end = m.end();
        let (Some(min), max) = (
            number(&c["min"]),
            c.name("max").and_then(|x| number(x.as_str())),
        ) else {
            prev_kept = false;
            continue;
        };
        let singular = !c["unit"].to_lowercase().ends_with('s');
        if c.name("hy").is_some()
            && singular
            && !after.trim_start().to_lowercase().starts_with("experience")
        {
            prev_kept = false;
            continue;
        }
        let after_clause = &after[..after.find([';', '.']).unwrap_or(after.len())];
        let near_after = &after_clause[..ceil(after_clause, after_clause.len().min(80))];
        let near_before = &before[floor(before, before.len().saturating_sub(40))..];
        let says_experience = EXPERIENCE.is_match(near_after)
            || FIELD_AFTER.is_match(after)
            || (after.trim_start().to_lowercase().starts_with("of ")
                && (m.start() <= 12 || EXPERIENCE.is_match(after_clause)))
            || near_before.to_lowercase().contains("experience");
        if !says_experience
            || min >= 20
            || NOT_HIRE.is_match(around)
            || NEGATED.is_match(near_before)
        {
            prev_kept = false;
            continue;
        }
        let preferred = preferred_after(after_clause, REQUIRED_WORD.is_match(near_before))
            || (PREFERRED.is_match(near_before) && !before.to_lowercase().contains("requir"));
        if preferred {
            // "12+ years with a bachelor's, or 8 with a master's, is preferred".
            if gap_or && prev_kept {
                out.pop();
            }
            prev_kept = false;
            continue;
        }
        let max = max.filter(|&x| x > min);
        let (min, max) = if MONTHS_TO.is_match(near_before) || AT_MOST.is_match(near_before) {
            (0, Some(min))
        } else {
            (min, max)
        };
        if gap_or
            && prev_kept
            && let Some(last) = out.last_mut()
        {
            if min < last.min {
                last.min = min;
                last.max = max;
            }
        } else {
            out.push(Claim {
                min,
                max,
                evidence: clip(sentence),
            });
        }
        prev_kept = true;
    }
    out
}

/// "5+ years of Go preferred", "..., or healthcare setting preferred": the
/// years are preferred. "8+ years in sales, tech client experience
/// preferred": a later part about experience of its own is preferred, not
/// the years.
fn preferred_after(clause: &str, required_before: bool) -> bool {
    for (i, part) in clause.split([',', '—', '–', '(']).enumerate() {
        // "Minimum 1–3 years of sales (retail preferred)": only the first
        // part can make required years preferred.
        if PREFERRED_AFTER.is_match(part)
            && (i == 0 || (!required_before && !OWN_SUBJECT.is_match(part)))
        {
            return true;
        }
        if REQUIRED_WORD.is_match(part) {
            return false;
        }
    }
    false
}

static REQUIRED_WORD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:required|must|mandatory|essential|minimum|at least)\b").unwrap()
});

static OWN_SUBJECT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:experience|knowledge|familiarity|exposure|background|degree|certifications?|skills)\b").unwrap()
});

fn number(s: &str) -> Option<u8> {
    let n = match s.to_lowercase().as_str() {
        "one" => 1,
        "two" => 2,
        "three" => 3,
        "four" => 4,
        "five" => 5,
        "six" => 6,
        "seven" => 7,
        "eight" => 8,
        "nine" => 9,
        "ten" => 10,
        "eleven" => 11,
        "twelve" => 12,
        "fifteen" => 15,
        d => d.parse().ok()?,
    };
    Some(n)
}

fn clip(s: &str) -> String {
    s.chars().take(240).collect()
}

fn floor(s: &str, mut i: usize) -> usize {
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil(s: &str, mut i: usize) -> usize {
    while !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

pub fn experience_from_field(text: &str) -> Option<Experience> {
    let t = text.trim();
    let lower = t.to_lowercase();
    let min = if lower.starts_with("any") || lower.contains("new grad") {
        0
    } else {
        number(&YEARS.captures(t)?["min"])?
    };
    Some(Experience {
        min_years: min,
        max_years: None,
        method: ExperienceMethod::Source,
        evidence: t.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn min(t: &str) -> Option<u8> {
        experience_from_text(t).map(|e| e.min_years)
    }

    #[test]
    fn reads_required_years() {
        assert_eq!(
            min("- 5+ years of experience building distributed systems"),
            Some(5)
        );
        assert_eq!(
            min("At least two years of professional experience."),
            Some(2)
        );
        assert_eq!(min("Experience: 3+ years"), Some(3));
        assert_eq!(
            min("- 8+ years professional experience\n- 4+ years of sales engineering experience"),
            Some(8)
        );
        let e = experience_from_text("3-5 years of experience in data engineering").unwrap();
        assert_eq!((e.min_years, e.max_years), (3, Some(5)));
    }

    #[test]
    fn leaves_out_what_is_not_required() {
        assert_eq!(
            min("### Nice to have\n- 7+ years of experience with Go"),
            None
        );
        assert_eq!(
            min("- 3+ years of experience\n- 5+ years in fintech preferred"),
            Some(3)
        );
        assert_eq!(
            min("For over 20 years, we have built payment software."),
            None
        );
        assert_eq!(min("A 4-year degree in computer science."), None);
        assert_eq!(min("Several years of experience with Python."), None);
        assert_eq!(min("Must be 18 years of age or older."), None);
    }

    #[test]
    fn leaves_out_what_is_not_the_hires_bar() {
        assert_eq!(
            min("With 20+ years of business expertise, we provide staffing."),
            None
        );
        assert_eq!(
            min("Your data is kept for up to 2 years in our candidate pool."),
            None
        );
        assert_eq!(
            min("You're not a CFO (we don't need 10+ years of experience)."),
            None
        );
        assert_eq!(
            min("Hugo has spent 3+ years at Qonto, moving to product."),
            None
        );
        // "asset management" is a field, not "is an asset".
        assert_eq!(
            min("- 6+ years of sales experience within asset management"),
            Some(6)
        );
        let e = experience_from_text("- less than 2 years of full-time work experience").unwrap();
        assert_eq!((e.min_years, e.max_years), (0, Some(2)));
    }

    #[test]
    fn alternatives_take_the_smallest() {
        assert_eq!(
            min(
                "- **For SMB AE:**\n    - 1–4 years in B2B sales\n- **For MM AE:**\n    - 3+ years in B2B sales"
            ),
            Some(1)
        );
        assert_eq!(
            min(
                "Bachelor's degree and 5 years of experience, or a master's degree and 3 years of experience."
            ),
            Some(3)
        );
    }

    #[test]
    fn none_needed_is_zero() {
        assert_eq!(min("No prior experience required; we train you."), Some(0));
        assert_eq!(
            experience_from_field("Any (new grads ok)").map(|e| e.min_years),
            Some(0)
        );
        assert_eq!(
            experience_from_field("6+ years").map(|e| e.min_years),
            Some(6)
        );
        assert_eq!(experience_from_field(""), None);
    }
}
