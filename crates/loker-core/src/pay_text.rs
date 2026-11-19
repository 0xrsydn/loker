//! Pay read from a job description, for sources with no pay fields
//! (Greenhouse, Lever, We Work Remotely). Precision first, because a wrong
//! figure is worse than none:
//!
//! * a figure needs a currency, and a sentence (or the heading above it) that
//!   talks about pay: "salary", "compensation", "pay range", "per hour", "OTE";
//! * a figure next to a benefit, funding or sales word ("$200 monthly wellness
//!   reimbursement", "raised $3M", "$5M ACV deals") is never pay;
//! * base pay needs a period: stated ("/year", "per hour", "annually"), or a
//!   salary of 15,000 or more, which is a year;
//! * extras (equity, bonus, commission, sign-on) count only on a line that
//!   also states base pay; OTE counts with its figure anywhere a figure would.
//!
//! The checks in `normalize::money` and `normalize::pay_from` apply on top.
//! `eval/pay` measures it against labelled jobs.
use std::sync::LazyLock;

use regex::Regex;

use crate::model::{Money, Pay, PayExtra, PayExtraKind, PayMethod, Period};
use crate::normalize::{self, money, percent};

/// A currency before a figure. Dollar prefixes name their country.
const CUR: &str = r"(?:Php|Rp|US\$|CA\$|C\$|AU\$|A\$|NZ\$|S\$|SG\$|HK\$|R\$|\$|€|£|₹|₱|\b(?:USD|EUR|GBP|CAD|AUD|NZD|SGD|HKD|INR|IDR|PHP|MYR|THB|VND|KRW|TWD|CNY|AED|SAR|ILS|TRY|CHF|JPY|BRL|MXN|ARS|CLP|COP|PEN|PLN|CZK|HUF|RON|SEK|NOK|DKK|ZAR|NGN|KES|EGP|PKR|BDT)\b)";
/// A currency code or sign after a figure ("59,000 EUR", "100k€").
const CODE: &str = r"(?:\b(?:USD|EUR|GBP|CAD|AUD|NZD|SGD|HKD|INR|IDR|PHP|MYR|THB|VND|KRW|TWD|CNY|AED|SAR|ILS|TRY|CHF|JPY|BRL|MXN|ARS|CLP|COP|PEN|PLN|CZK|HUF|RON|SEK|NOK|DKK|ZAR|NGN|KES|EGP|PKR|BDT)\b|€|£)";
/// A figure: "120,000", "15,00,000", "2.800" (thousands), "24.50".
const NUM: &str = r"\d{1,3}(?:\.\d{3})+\b|\d[\d,]*(?:\.\d+)?";
const SUFFIX: &str = r"(?:[kK]\b|MM?\b|\s?million\b)";

static RANGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?P<c1>{CUR})?\.?\s?(?P<n1>{NUM})(?P<s1>{SUFFIX})?(?:\s?(?P<cm>{CODE}))?(?:\s?(?:-|–|—|to|and)\s?(?P<c2>{CUR})?\s?(?P<n2>{NUM})(?P<s2>{SUFFIX})?)?(?P<plus>\+)?(?:\s?\(?(?P<c3>{CODE})\)?)?"
    ))
    .unwrap()
});

static EQUITY_PERCENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?P<a>\d+(?:\.\d+)?)\s?%(?:\s?(?:-|–|—|to)\s?(?P<b>\d+(?:\.\d+)?)\s?%)?\s+(?:in\s+|of\s+)?equity")
        .unwrap()
});

/// Words that make a sentence about this job's pay.
static PAY_WORDS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:salary|salaries|compensation|remuneration|wages?|pay|pays|comp|hourly rate|day rate|rate of|(?:monthly|weekly|daily|annual) rate|gehalt|salaire|sueldo|salario|gaji|ote|on[- ]target|per hour|an hour|/\s?hr|/\s?hour)\b|\brate\s*:").unwrap()
});

/// Words that make a figure something other than pay, near it.
static NOT_PAY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:rais(?:e|ed|ing)|funding|funded|series [a-f]|seed|pre-seed|valuation|valued|revenue|arr|acv|gmv|deals?|invest(?:ed|ors?|ment)?|backed|budgets?|grants?|reimburse\w*|allowance|stipend|wellness|laundry|meals?|lunch|relocation|401\(?k?\)?|hsa|fsa|retirement|pension|match(?:ing)?|home office|workstation|set ?up|equipment|learning|education|training|(?:professional|career|personal) development|conference|gym|fitness|health|medical|dental|vision|childcare|vacation|bike|commut\w*|mobility|perks?|vouchers?|subscriptions?|membership|saved|set aside|members|internet|phone|coworking|referral|customers|users|clients|quota|prize|award|fund|fees?|price|pricing|cost|spend|savings|insurance|cpa|cpc|aum|assets|portfolio|transactions|loans?|billion|draw|per (?:referred|active|candidate|lead|sale|task|word|video))\b",
    )
    .unwrap()
});

/// "(about $2,500 a month)", "equivalent to €72,000 annually": a range
/// already given, restated.
static RESTATED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:\(|equivalent to|approximately|approx\.?|roughly|about)\s*$").unwrap()
});

static LABEL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*(?:per year\s+|annually\s+)?(?:in|for)\s+(?:most\s+|all\s+|the\s+)?(?P<l>[A-Z][\w.\- ]{1,30}?)(?:-based)?\s+(?:locations?|roles?|candidates|employees|residents|applicants)").unwrap()
});

/// Pay stated in `text` (a description), or `None`.
pub fn pay_from_text(text: &str) -> Option<Pay> {
    let text = clean(text);
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let (mut base, mut extras, mut evidence) =
        (Vec::<Money>::new(), Vec::new(), Vec::<String>::new());
    let mut base_lines = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        // Pay words may sit around the figure: "Annual Salary" or "Base Pay
        // Range" above it, "Average base salary range." below.
        // A period above counts only from a heading, not from a paragraph.
        let heading: String = lines[i.saturating_sub(2)..i]
            .iter()
            .filter(|l| l.chars().count() < 80)
            .copied()
            .collect::<Vec<_>>()
            .join(" ");
        let around = lines[i.saturating_sub(2)..(i + 4).min(lines.len())].join(" ");
        // An OTE named in the paragraph introducing the figures ("On-Target
        // Earnings (OTE) for this role are listed below").
        let intro = lines[i.saturating_sub(5)..i].join(" ");
        // Ranges listed one after another under one pay heading share it.
        let pay_context =
            PAY_WORDS.is_match(&around) || base_lines.last().is_some_and(|b: &usize| i - b <= 3);
        // With several ranges, the line just above names each: "US Metro Zone 2".
        let label = i
            .checked_sub(1)
            .map(|j| lines[j])
            .filter(|l| l.chars().count() <= 60 && !has_money(l));
        let mut line_has_base = false;
        for sentence in sentences(line) {
            let mut used = false;
            for m in RANGE.captures_iter(sentence) {
                let Some(found) = figure(sentence, &intro, &heading, pay_context, &m) else {
                    continue;
                };
                match found {
                    Found::Base(mut money) => {
                        let at = m.get(0).unwrap();
                        if line_has_base && RESTATED.is_match(&sentence[..at.start()]) {
                            continue;
                        }
                        // "Metro 2: $130,000 - 150,000" names its range first.
                        let prefix = sentence[..at.start()].trim();
                        let prefix = prefix
                            .strip_suffix(':')
                            .filter(|p| {
                                p.chars().count() <= 40 && !has_money(p) && !PAY_WORDS.is_match(p)
                            })
                            .map(|p| p.trim().to_string());
                        money.label = LABEL
                            .captures(&sentence[at.end()..])
                            .map(|c| c["l"].trim().to_string())
                            .or(prefix)
                            .or_else(|| label.map(str::to_string));
                        if !base.contains(&money) {
                            base.push(money);
                        }
                        line_has_base = true;
                    }
                    Found::Extra(e) => extras.push(e),
                }
                used = true;
            }
            if used && !evidence.iter().any(|e| e == sentence) {
                evidence.push(sentence.to_string());
            }
        }
        if line_has_base {
            base_lines.push(i);
        }
    }
    // Extras are named near base pay, or where the posting offers pay ("may
    // also be eligible for bonus, equity"), never alone: "administer equity
    // plans" is a duty, not pay.
    if !base_lines.is_empty() {
        for (i, line) in lines.iter().enumerate() {
            if base_lines.iter().any(|b| b.abs_diff(i) <= 3) || OFFER.is_match(line) {
                extras.extend(extras_named(line));
            }
        }
    }
    // A single range needs no label; several keep theirs only if all have one.
    if base.len() < 2 || base.iter().any(|m| m.label.is_none()) {
        base.iter_mut().for_each(|m| m.label = None);
    }
    let raw: String = evidence.join(" … ").chars().take(300).collect();
    let mut pay = normalize::pay_from(base, extras, Some(&raw))?;
    pay.method = PayMethod::Text;
    Some(pay)
}

enum Found {
    Base(Money),
    Extra(PayExtra),
}

/// Currencies whose annual salaries start around 15,000: a salary figure that
/// large with no period is a year. Elsewhere a monthly salary can be that
/// large (25,000 ZAR), so no period means no figure.
const YEARLY_AT_15K: &[&str] = &["USD", "EUR", "GBP", "CAD", "AUD", "NZD", "CHF", "SGD"];

/// One figure in a sentence: base pay, an extra with its figure, or nothing
/// (no currency, no pay words around and no stated period, a benefit or a
/// sales number, a total, no period).
fn figure(
    sentence: &str,
    above: &str,
    heading: &str,
    pay_context: bool,
    m: &regex::Captures,
) -> Option<Found> {
    let all = m.get(0)?;
    let mut currency = currency(m)?;
    if bare_dollar(m) {
        currency = dollar_country(&format!("{sentence} {heading}"))?.to_string();
    }
    let before = &sentence[..all.start()];
    let after = &sentence[all.end()..];
    let near = format!("{} {}", tail(before, 40), head(after, 40));
    if NOT_PAY.is_match(&near) {
        return None;
    }
    // With no pay words around, only a range with its period counts:
    // "$10,000–$25,000 USD annually", not "an average of $1,800 annually".
    let stated = period_after(after);
    // "$185K–$215K base": the word after the figure says what it is.
    let pay_context = pay_context
        || head(after, 8)
            .trim_start()
            .to_lowercase()
            .starts_with("base");
    if !pay_context && (stated.is_none() || m.name("n2").is_none()) {
        return None;
    }
    if TOTAL.is_match(&tail(before, 60).to_lowercase()) {
        return None;
    }
    let lower_before = tail(before, 40).to_lowercase();
    let first = number(&m["n1"], m.name("s1"), m.name("s2"))?;
    let second = match m.name("n2") {
        Some(n2) => Some(number(n2.as_str(), m.name("s2"), None)?),
        None => None,
    };
    let (min, max) = match second {
        Some(b) => (Some(first), Some(b)),
        None if m.name("plus").is_some() || head(after, 12).to_lowercase().contains("or more") => {
            (Some(first), None)
        }
        None if UP_TO.is_match(&lower_before) => (None, Some(first)),
        None if FROM.is_match(&lower_before) => (Some(first), None),
        None => (Some(first), Some(first)),
    };
    let period = stated
        .or_else(|| period_word(&tail(before, 40)))
        .or_else(|| period_word(heading));
    if let Some(kind) = extra_kind(
        &tail(before, 60).to_lowercase(),
        &head(after, 30).to_lowercase(),
        &above.to_lowercase(),
    ) {
        return Some(Found::Extra(PayExtra {
            kind,
            amount: Some(money(min, max, Some(&currency), period)?),
            percent: None,
        }));
    }
    let period = period.or_else(|| {
        (pay_context && YEARLY_AT_15K.contains(&currency.as_str()) && min.or(max)? >= 15_000.0)
            .then_some(Period::Year)
    })?;
    money(min, max, Some(&currency), Some(period)).map(Found::Base)
}

static OFFER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:eligible for|may (?:also )?include|also includes?|total rewards|compensation package|in addition to (?:base|salary))\b").unwrap()
});

/// "total compensation of up to €120,000": not base pay.
static TOTAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"total (?:cash |target |annual )?(?:compensation|comp|cash|package|rewards?|potential|earnings)\b[^$€£\d]*$",
    )
    .unwrap()
});
static UP_TO: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|\s)(?:up to|bis|hingga|sampai)\s*$").unwrap());
static FROM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:^|\s)(?:starting at|starts at|from|at least|minimum(?: of)?|min\.?)\s*$")
        .unwrap()
});

/// A bare "$" (no country prefix, no code after).
fn bare_dollar(m: &regex::Captures) -> bool {
    m.name("c3").is_none()
        && m.name("cm").is_none()
        && m.name("c1")
            .or_else(|| m.name("c2"))
            .is_some_and(|c| c.as_str().trim() == "$")
}

/// "$" is the US dollar unless the text names another dollar country
/// ("Ontario based: $80,000" is CAD); `None` when it names both, since the
/// figure could be either.
fn dollar_country(text: &str) -> Option<&'static str> {
    static OTHERS: LazyLock<Vec<(&str, Regex)>> = LazyLock::new(|| {
        [
            ("CAD", r"(?i)\b(?:cad|canada|canadian|ontario|toronto|vancouver|montr[eé]al|qu[eé]bec|british columbia|alberta|calgary|ottawa|edmonton|winnipeg)\b"),
            ("AUD", r"(?i)\b(?:aud|australia|australian|sydney|melbourne|brisbane|perth|adelaide)\b"),
            ("NZD", r"(?i)\b(?:nzd|new zealand|auckland|wellington)\b"),
            ("SGD", r"(?i)\b(?:sgd|singapore)\b"),
        ]
        .into_iter()
        .map(|(c, r)| (c, Regex::new(r).unwrap()))
        .collect()
    });
    static US: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\b(?:US|U\.S\.|USA|United States)\b|(?i:\b(?:american|california|new york|san francisco|seattle|texas|boston|chicago|austin|denver)\b)").unwrap()
    });
    let named: Vec<&str> = OTHERS
        .iter()
        .filter(|(_, r)| r.is_match(text))
        .map(|(c, _)| *c)
        .collect();
    match (named.as_slice(), US.is_match(text)) {
        ([], _) => Some("USD"),
        ([c], false) => Some(c),
        _ => None,
    }
}

fn has_money(text: &str) -> bool {
    RANGE.captures_iter(text).any(|m| currency(&m).is_some())
}

fn currency(m: &regex::Captures) -> Option<String> {
    // A code after the figure names the dollar: "$175,000 - $235,000 CAD".
    let c = m
        .name("c3")
        .or_else(|| m.name("cm"))
        .or_else(|| m.name("c1"))
        .or_else(|| m.name("c2"))?
        .as_str()
        .trim();
    Some(
        match c {
            "$" | "US$" => "USD",
            "CA$" | "C$" => "CAD",
            "AU$" | "A$" => "AUD",
            "NZ$" => "NZD",
            "S$" | "SG$" => "SGD",
            "HK$" => "HKD",
            "R$" => "BRL",
            "€" => "EUR",
            "£" => "GBP",
            "₹" => "INR",
            "₱" => "PHP",
            "Php" => "PHP",
            "Rp" => "IDR",
            code => code,
        }
        .to_string(),
    )
}

/// A figure with its suffix ("120k", "1.2M"); `other` is the range's other
/// suffix, for "$130-225K".
fn number(n: &str, suffix: Option<regex::Match>, other: Option<regex::Match>) -> Option<f64> {
    // "60,000,00": a comma before two final digits is a decimal comma.
    let n = &match n.rsplit_once(',') {
        Some((head, cents)) if cents.len() == 2 && !n.contains('.') => format!("{head}.{cents}"),
        _ => n.to_string(),
    };
    let digits = if n.matches('.').count() >= 1
        && n.split('.').skip(1).all(|g| g.len() == 3)
        && !n.contains(',')
    {
        n.replace('.', "")
    } else {
        n.replace(',', "")
    };
    let v: f64 = digits.parse().ok()?;
    let scale = |s: &str| match s.trim().to_lowercase().as_str() {
        "k" => 1e3,
        "m" | "mm" | "million" => 1e6,
        _ => 1.0,
    };
    Some(match (suffix, other) {
        (Some(s), _) => v * scale(s.as_str()),
        // "$130-225K": the first figure takes the second's suffix when it would
        // otherwise be far smaller.
        (None, Some(o)) if v < 1000.0 => v * scale(o.as_str()),
        _ => v,
    })
}

/// What a figure is, when not base pay: a kind word right after it ("€90,000
/// OTE", "$10,000 signing bonus"), else the nearest kind word before it
/// unless a base-pay word is nearer ("salary is $200K … OTE up to $500K"),
/// else a heading that says OTE ("Annual OTE Salary").
fn extra_kind(before: &str, after: &str, above: &str) -> Option<PayExtraKind> {
    static WORD: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\b(?:ote|on[- ]target|sign[- ]on|signing|bonus|commission|equity|stock|rsus?|salary|base|wage|rate)\b").unwrap()
    });
    let kind = |w: &str| match w {
        "ote" | "on-target" | "on target" => Some(PayExtraKind::Ote),
        "sign-on" | "sign on" | "signing" => Some(PayExtraKind::SignOn),
        "bonus" => Some(PayExtraKind::Bonus),
        "commission" => Some(PayExtraKind::Commission),
        "equity" | "stock" | "rsu" | "rsus" => Some(PayExtraKind::Equity),
        _ => None,
    };
    let after = after
        .trim_start()
        .trim_start_matches(|c: char| c == '(' || c.is_whitespace())
        .trim_start_matches("in ")
        .trim_start_matches("of ");
    // "€30,000 – €40,000 per year (total compensation, including base and variable)"
    if [
        "total compensation",
        "including base",
        "base and variable",
        "base plus variable",
    ]
    .iter()
    .any(|w| after.contains(w))
    {
        return Some(PayExtraKind::Ote);
    }
    if let Some(w) = WORD.find(after).filter(|w| w.start() == 0) {
        return kind(w.as_str());
    }
    let before = before.replace("base plus", "").replace("base +", "");
    if let Some(w) = WORD.find_iter(&before).last() {
        return kind(w.as_str());
    }
    WORD.find_iter(above)
        .any(|w| matches!(w.as_str(), "ote" | "on-target" | "on target"))
        .then_some(PayExtraKind::Ote)
}

/// Extras named on a line that states base pay ("plus equity", "eligible
/// for a bonus"), with equity as a percentage when given.
fn extras_named(line: &str) -> Vec<PayExtra> {
    let lower = line.to_lowercase();
    let named = |words: &[&str]| {
        words.iter().any(|w| {
            lower.match_indices(w).any(|(i, _)| {
                let before = tail(&lower[..i], 12);
                !(before.contains("no ") || before.contains("not ") || before.contains("without "))
            })
        })
    };
    let bare = |kind| PayExtra {
        kind,
        amount: None,
        percent: None,
    };
    let mut out = Vec::new();
    if let Some(c) = EQUITY_PERCENT.captures(line) {
        let a = c["a"].parse().ok();
        let b = c.name("b").and_then(|b| b.as_str().parse().ok()).or(a);
        out.push(PayExtra {
            percent: percent(a, b),
            ..bare(PayExtraKind::Equity)
        });
    } else if named(&["equity", "stock option", "restricted stock", "rsu"]) {
        out.push(bare(PayExtraKind::Equity));
    }
    if named(&["sign-on bonus", "signing bonus", "sign on bonus"]) {
        out.push(bare(PayExtraKind::SignOn));
    }
    if lower.match_indices("bonus").any(|(i, _)| {
        let before = tail(&lower[..i], 12);
        ![
            "sign-on ",
            "signing ",
            "sign on ",
            "relocation ",
            "referral ",
            "no ",
            "not ",
        ]
        .iter()
        .any(|w| before.ends_with(w) || before.contains(w))
    }) {
        out.push(bare(PayExtraKind::Bonus));
    }
    if named(&["commission"]) {
        out.push(bare(PayExtraKind::Commission));
    }
    out
}

/// A period stated right after the figure: "/year", " per hour", " USD annually".
fn period_after(after: &str) -> Option<Period> {
    static AFTER: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)^\s*(?:(?:USD|EUR|GBP|CAD|AUD|gross|base|salary)\s+)*(?:/\s?|per\s+|an?\s+|each\s+)?(?P<p>year|yr|annum|annually|annual|yearly|hour|hr|hourly|month|mo|monthly|week|weekly|day|daily)\b").unwrap()
    });
    normalize::pay_period(&AFTER.captures(after)?["p"])
}

/// The first period word in `text`: "annual base salary", "Hourly Pay Range".
fn period_word(text: &str) -> Option<Period> {
    static WORD: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)\b(?:per\s+|an?\s+)?(?P<p>annum|annually|annual|yearly|year|hourly|hour|monthly|month|weekly|daily)\b").unwrap()
    });
    let p = WORD.captures(text)?;
    let w = p["p"].to_lowercase();
    normalize::pay_period(match w.as_str() {
        "annum" => "year",
        w => w,
    })
}

/// Markdown escapes and emphasis out, so "\$120,000" and "**Salary:**" read
/// plainly; "Min: $18.53, Max: $24.35" as the range it is.
fn clean(text: &str) -> String {
    static MIN_MAX: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"(?i)\bmin(?:imum)?\s*:?\s*(?P<a>[$€£]\s?\d[\d.,]*\d)\s*[,;/]?\s*max(?:imum)?\s*:?\s*",
        )
        .unwrap()
    });
    // UTF-8 dashes and dots a feed decoded as Latin-1: "$185Kâ\u{80}\u{93}$215K".
    let text = text
        .replace("â\u{80}\u{93}", "–")
        .replace("â\u{80}\u{94}", "—")
        .replace("Â·", "·")
        .replace("Â ", " ")
        .replace("\\n", "\n")
        .replace("\\$", "$")
        .replace("\\-", "-")
        .replace("\\.", ".")
        .replace("\\+", "+")
        .replace("**", "")
        .replace("__", "")
        .replace(['\u{a0}', '\u{2009}', '\u{202f}'], " ");
    MIN_MAX.replace_all(&text, "${a} - ").into_owned()
}

/// Sentences of a line, split after ". ", "! ", "? " and "; " but not inside
/// figures ("$24.50").
fn sentences(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let b = line.as_bytes();
    for i in 0..b.len().saturating_sub(1) {
        if matches!(b[i], b'.' | b'!' | b'?' | b';') && b[i + 1] == b' ' {
            out.push(line[start..=i].trim());
            start = i + 1;
        }
    }
    out.push(line[start..].trim());
    out.retain(|s| !s.is_empty());
    out
}

fn tail(s: &str, n: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    chars[chars.len().saturating_sub(n)..].iter().collect()
}

fn head(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}
