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

// ---------- regions ----------
//
// Which countries a region a job names takes in, for the per-viewer verdict.
// Common usage in job posts, not geography exams: APAC is East, Southeast and
// South Asia plus Oceania; EMEA is Europe, the Middle East and Africa; LATAM
// is Mexico, Central America, the Spanish-, Portuguese- and French-speaking
// Caribbean and South America. Türkiye and Egypt sit in two blocks, as posts
// use them.

const EUROPE: &[&str] = &[
    "AD", "AL", "AT", "BA", "BE", "BG", "BY", "CH", "CY", "CZ", "DE", "DK", "EE", "ES", "FI", "FO",
    "FR", "GB", "GG", "GI", "GR", "HR", "HU", "IE", "IM", "IS", "IT", "JE", "LI", "LT", "LU", "LV",
    "MC", "MD", "ME", "MK", "MT", "NL", "NO", "PL", "PT", "RO", "RS", "RU", "SE", "SI", "SK", "SM",
    "TR", "UA", "VA",
];
const MIDDLE_EAST: &[&str] = &[
    "AE", "BH", "EG", "IL", "IQ", "IR", "JO", "KW", "LB", "OM", "PS", "QA", "SA", "SY", "TR", "YE",
];
const AFRICA: &[&str] = &[
    "AO", "BF", "BI", "BJ", "BW", "CD", "CF", "CG", "CI", "CM", "CV", "DJ", "DZ", "EG", "EH", "ER",
    "ET", "GA", "GH", "GM", "GN", "GQ", "GW", "KE", "KM", "LR", "LS", "LY", "MA", "MG", "ML", "MR",
    "MU", "MW", "MZ", "NA", "NE", "NG", "RE", "RW", "SC", "SD", "SH", "SL", "SN", "SO", "SS", "ST",
    "SZ", "TD", "TG", "TN", "TZ", "UG", "YT", "ZA", "ZM", "ZW",
];
const EAST_ASIA: &[&str] = &["CN", "HK", "JP", "KP", "KR", "MN", "MO", "TW"];
const SOUTHEAST_ASIA: &[&str] = &[
    "BN", "ID", "KH", "LA", "MM", "MY", "PH", "SG", "TH", "TL", "VN",
];
const SOUTH_ASIA: &[&str] = &["AF", "BD", "BT", "IN", "LK", "MV", "NP", "PK"];
const CENTRAL_ASIA: &[&str] = &["KG", "KZ", "TJ", "TM", "UZ"];
const OCEANIA: &[&str] = &[
    "AS", "AU", "CK", "FJ", "FM", "GU", "KI", "MH", "NC", "NR", "NU", "NZ", "PF", "PG", "PW", "SB",
    "TO", "TV", "VU", "WS",
];
const NORTH_AMERICA: &[&str] = &["CA", "MX", "US"];
const CENTRAL_AMERICA: &[&str] = &["BZ", "CR", "GT", "HN", "NI", "PA", "SV"];
const LATIN_CARIBBEAN: &[&str] = &["CU", "DO", "HT", "PR"];
const OTHER_CARIBBEAN: &[&str] = &[
    "AG", "AI", "AW", "BB", "BL", "BM", "BQ", "BS", "CW", "DM", "GD", "GP", "JM", "KN", "KY", "LC",
    "MF", "MQ", "MS", "SX", "TC", "TT", "VC", "VG", "VI",
];
const SOUTH_AMERICA: &[&str] = &[
    "AR", "BO", "BR", "CL", "CO", "EC", "FK", "GF", "GY", "PE", "PY", "SR", "UY", "VE",
];

/// Whether `region` (a name `normalize` produces, e.g. "APAC") takes in the
/// country with ISO code `code`. `None` for a region this table doesn't know.
pub fn region_contains(region: &str, code: &str) -> Option<bool> {
    let blocks: &[&[&str]] = match region {
        "Europe" => &[EUROPE],
        "EMEA" => &[EUROPE, MIDDLE_EAST, AFRICA],
        "Middle East" => &[MIDDLE_EAST],
        "Africa" => &[AFRICA],
        "APAC" => &[EAST_ASIA, SOUTHEAST_ASIA, SOUTH_ASIA, OCEANIA],
        "Asia" => &[EAST_ASIA, SOUTHEAST_ASIA, SOUTH_ASIA, CENTRAL_ASIA],
        "Southeast Asia" => &[SOUTHEAST_ASIA],
        "Oceania" => &[OCEANIA],
        "North America" => &[NORTH_AMERICA],
        "South America" => &[SOUTH_AMERICA],
        "LATAM" => &[&["MX"], CENTRAL_AMERICA, LATIN_CARIBBEAN, SOUTH_AMERICA],
        "Americas" => &[
            NORTH_AMERICA,
            CENTRAL_AMERICA,
            LATIN_CARIBBEAN,
            OTHER_CARIBBEAN,
            SOUTH_AMERICA,
        ],
        _ => return None,
    };
    Some(blocks.iter().any(|b| b.contains(&code)))
}
