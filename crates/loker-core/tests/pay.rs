//! The pay rules: what is kept, what is dropped, and the text written from
//! it. A wrong figure is worse than none, so most rows are drops.
use loker_core::model::{Money, PayExtra, PayExtraKind, Period};
use loker_core::normalize::{money, pay, pay_from, pay_period, percent};

#[test]
fn pay_from_a_source_range() {
    let usd = |min, max, period| pay(min, max, Some("USD"), Some(period));
    let cases = [
        // (pay, USD per month min and max, text)
        (
            usd(Some(120e3), Some(150e3), "annual"),
            (Some(10_000), Some(12_500)),
            "USD 120,000–150,000 a year",
        ),
        (
            usd(Some(120e3), Some(120e3), "yearly"),
            (Some(10_000), Some(10_000)),
            "USD 120,000 a year",
        ),
        (
            usd(None, Some(150e3), "per-year-salary"),
            (None, Some(12_500)),
            "up to USD 150,000 a year",
        ),
        (
            usd(Some(18.5), Some(24.0), "hourly"),
            (Some(2_960), Some(3_840)),
            "USD 18.50–24 an hour",
        ),
        // Zero means not stated.
        (
            usd(Some(0.0), Some(60e3), "annual"),
            (None, Some(5_000)),
            "up to USD 60,000 a year",
        ),
        // Other currencies are kept as stated, with no USD figures yet.
        (
            pay(Some(53e3), Some(59e3), Some("eur"), Some("1 YEAR")),
            (None, None),
            "EUR 53,000–59,000 a year",
        ),
    ];
    for (p, usd_month, text) in cases {
        let p = p.unwrap_or_else(|| panic!("{text}: no pay"));
        assert_eq!((p.usd_month_min, p.usd_month_max), usd_month, "{text}");
        assert_eq!(p.raw, text);
    }
}

#[test]
fn doubtful_figures_are_dropped() {
    let drops = [
        // min above max
        pay(Some(150e3), Some(120e3), Some("USD"), Some("annual")),
        // a top over 10× the bottom, in any currency
        pay(Some(65e3), Some(6e6), Some("CAD"), Some("annual")),
        // no figures, no currency, a currency that isn't a code
        pay(None, None, Some("USD"), Some("annual")),
        pay(Some(50e3), None, None, Some("annual")),
        pay(Some(50e3), None, Some("$"), Some("annual")),
        // an unknown or missing period
        pay(Some(5e3), None, Some("USD"), Some("biweekly")),
        pay(Some(5e3), None, Some("USD"), Some("NONE")),
        pay(Some(5e3), None, Some("USD"), None),
        // USD outside 100..=100k a month: a typo or a placeholder
        pay(Some(114e6), Some(140e6), Some("USD"), Some("annual")),
        pay(Some(1.0), Some(2.0), Some("USD"), Some("monthly")),
        pay(Some(f64::NAN), None, Some("USD"), Some("annual")),
    ];
    for (i, p) in drops.iter().enumerate() {
        assert!(p.is_none(), "case {i}: {p:?}");
    }
    assert_eq!(pay_period("per-hour-wage"), Some(Period::Hour));
    assert_eq!(pay_period("monthly yearly"), None);
    assert!(percent(Some(0.25), Some(1.0)).is_some());
    assert!(percent(Some(5.0), Some(150.0)).is_none());
}

#[test]
fn ranges_by_level_and_extras() {
    let year = |min, max, label: &str| Money {
        label: Some(label.into()),
        ..money(Some(min), Some(max), Some("USD"), Some(Period::Year)).unwrap()
    };
    let extra = |kind, amount| PayExtra {
        kind,
        amount,
        percent: None,
    };
    let p = pay_from(
        vec![
            year(120e3, 150e3, "Lead"),
            year(150e3, 200e3, "Senior Lead"),
        ],
        vec![
            extra(PayExtraKind::Bonus, None),
            extra(PayExtraKind::Equity, None),
            // A second equity with figures replaces the bare one.
            extra(
                PayExtraKind::Equity,
                money(None, Some(1e6), Some("USD"), None),
            ),
        ],
        None,
    )
    .unwrap();
    assert_eq!(
        (p.usd_month_min, p.usd_month_max),
        (Some(10_000), Some(16_667))
    );
    assert_eq!(
        p.raw,
        "Lead: USD 120,000–150,000 a year; Senior Lead: USD 150,000–200,000 a year \
         + equity up to USD 1,000,000, bonus"
    );

    // Only extras: "competitive salary + equity" is pay, with no base.
    let p = pay_from(vec![], vec![extra(PayExtraKind::Equity, None)], None).unwrap();
    assert!(p.base.is_empty() && p.usd_month_max.is_none());
    assert_eq!(p.raw, "equity");
    // Base pay needs a period; nothing left means no pay.
    assert!(
        pay_from(
            vec![money(Some(5e3), None, Some("USD"), None).unwrap()],
            vec![],
            None
        )
        .is_none()
    );
}

#[test]
fn pay_read_from_descriptions() {
    use loker_core::pay_text::pay_from_text;
    let base = |text: &str| {
        pay_from_text(text).map(|p| {
            p.base
                .iter()
                .map(|m| (m.currency.clone(), m.period, m.min, m.max, m.label.clone()))
                .collect::<Vec<_>>()
        })
    };
    let year = |c: &str, min, max| (c.to_string(), Some(Period::Year), min, max, None);
    let cases = [
        // A stated range and period; markdown escapes are read through.
        (
            "The estimated salary range is \\$125,000- \\$200,000/year.",
            Some(vec![year("USD", Some(125e3), Some(200e3))]),
        ),
        // Pay words on the heading above; a salary this large is a year.
        (
            "Pay Range\n\n$196,000—$230,000 USD",
            Some(vec![year("USD", Some(196e3), Some(230e3))]),
        ),
        // A code after the figure names the dollar; "min" is one end.
        (
            "an annual salary of min 80,000 EUR gross",
            Some(vec![year("EUR", Some(80e3), None)]),
        ),
        (
            "Pay: $15-18 per hour plus bonuses",
            Some(vec![(
                "USD".into(),
                Some(Period::Hour),
                Some(15.0),
                Some(18.0),
                None,
            )]),
        ),
        // A restatement is not a second range.
        (
            "Salary: $30,000 USD a year (about $2,500 a month), as a contractor",
            Some(vec![year("USD", Some(30e3), Some(30e3))]),
        ),
        // Several ranges, each named by the line above.
        (
            "Canada Base Pay Range\n$156,500—$215,000 CAD\nUS General\n$165,000—$225,000 USD",
            Some(vec![
                (
                    "CAD".into(),
                    Some(Period::Year),
                    Some(156_500.0),
                    Some(215e3),
                    Some("Canada Base Pay Range".into()),
                ),
                (
                    "USD".into(),
                    Some(Period::Year),
                    Some(165e3),
                    Some(225e3),
                    Some("US General".into()),
                ),
            ]),
        ),
        // Another dollar, named by place or code; both named is either.
        (
            "Expected salary range, Ontario based: $80,000 - $100,000",
            Some(vec![year("CAD", Some(80e3), Some(100e3))]),
        ),
        ("Salary: $80,000 - $100,000 in the US or Canada", None),
        // A sign after a k figure; "bis" is up to.
        (
            "Gehalt bis 100k€ + Bonus",
            Some(vec![year("EUR", None, Some(100e3))]),
        ),
        // A decimal comma; a broken (mojibake) dash still makes a range.
        (
            "Compensation Range: $60,000,00 - $65,000.00 + commission CAD",
            Some(vec![year("CAD", Some(60e3), Some(65e3))]),
        ),
        (
            "Hybrid in Palo Alto · $185Kâ\u{80}\u{93}$215K base + equity",
            Some(vec![year("USD", Some(185e3), Some(215e3))]),
        ),
        // A benefit after base pay is not a second range.
        (
            "Salary $90,000. Grow with $1500 USD annually for professional development.",
            Some(vec![year("USD", Some(90e3), Some(90e3))]),
        ),
        // Not pay: benefits, funding, sales numbers, totals, savings.
        ("*   $200 monthly wellness reimbursement", None),
        ("*   Up to $15K relocation bonus", None),
        (
            "We just raised a $3M pre-seed. Salary is competitive.",
            None,
        ),
        ("Experience closing $5M-$10M+ ACV deals", None),
        (
            "Members set aside an average of more than $1,800 annually.",
            None,
        ),
        (
            "The total cash compensation range for this position is $230,800 to $461,600.",
            None,
        ),
        // No period and too small to be a year: left out, not guessed.
        ("Competitive base salary of €1500 plus commission", None),
        ("Salary: 25,000 ZAR to 30,000 ZAR", None),
    ];
    for (text, want) in cases {
        assert_eq!(base(text), want, "{text}");
    }

    // Base and OTE in one sentence; extras named next to base pay.
    let p = pay_from_text(
        "Base salary is $200,000 to $250,000 annually, with commission and OTE up to $500,000.",
    )
    .unwrap();
    assert_eq!(p.base.len(), 1);
    let kinds: Vec<_> = p
        .extras
        .iter()
        .map(|e| (e.kind, e.amount.as_ref().and_then(|m| m.max)))
        .collect();
    assert_eq!(
        kinds,
        [
            (PayExtraKind::Commission, None),
            (PayExtraKind::Ote, Some(500e3))
        ]
    );
    assert_eq!(p.method, loker_core::model::PayMethod::Text);
    // Equity a hire administers is not equity a hire gets.
    assert!(pay_from_text("You will administer equity compensation plans.").is_none());
}
