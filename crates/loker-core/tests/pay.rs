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
