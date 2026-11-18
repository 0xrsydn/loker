//! Country names as job boards write them → ISO 3166-1 alpha-2.
use isocountry::CountryCode;

/// Common names that differ from the ISO short names.
const ALIASES: &[(&str, &str)] = &[
    ("united states", "US"),
    ("usa", "US"),
    ("united kingdom", "GB"),
    ("uk", "GB"),
    ("south korea", "KR"),
    ("north korea", "KP"),
    ("vietnam", "VN"),
    ("russia", "RU"),
    ("taiwan", "TW"),
    ("iran", "IR"),
    ("syria", "SY"),
    ("laos", "LA"),
    ("tanzania", "TZ"),
    ("venezuela", "VE"),
    ("czech republic", "CZ"),
    ("turkey", "TR"),
    ("türkiye", "TR"),
    ("ivory coast", "CI"),
    ("palestine", "PS"),
    ("macau", "MO"),
    ("hong kong", "HK"),
    ("brunei", "BN"),
    ("cape verde", "CV"),
    ("swaziland", "SZ"),
    ("micronesia", "FM"),
    ("democratic republic of the congo", "CD"),
    ("dr congo", "CD"),
    ("republic of the congo", "CG"),
];

pub fn to_alpha2(name: &str) -> Option<&'static str> {
    let n = name.trim();
    if n.len() == 2
        && let Ok(c) = CountryCode::for_alpha2_caseless(n)
    {
        return Some(c.alpha2());
    }
    if let Some((_, code)) = ALIASES.iter().find(|(a, _)| a.eq_ignore_ascii_case(n)) {
        return Some(code);
    }
    CountryCode::iter()
        .find(|c| {
            let full = c.name();
            full.eq_ignore_ascii_case(n) || short_name(full).eq_ignore_ascii_case(n)
        })
        .map(|c| c.alpha2())
}

/// "Bolivia (Plurinational State of)" → "Bolivia"; "Korea, Republic of" → "Korea".
fn short_name(full: &str) -> &str {
    let s = full.split(" (").next().unwrap_or(full);
    s.split(',').next().unwrap_or(s).trim()
}
