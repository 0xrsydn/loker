use isocountry::CountryCode;

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

fn short_name(full: &str) -> &str {
    let s = full.split(" (").next().unwrap_or(full);
    s.split(',').next().unwrap_or(s).trim()
}

// States and provinces job boards write instead of a country, and the cities
// they hire in most. Only names that mean one country in job posts: no
// "Cambridge", "Georgia" (a country first) or "Santiago de Compostela".
// "New York" is a city here: in job posts it is mostly the office.

const US_STATES: &[&str] = &[
    "alabama",
    "alaska",
    "arizona",
    "arkansas",
    "california",
    "colorado",
    "connecticut",
    "delaware",
    "florida",
    "hawaii",
    "idaho",
    "illinois",
    "indiana",
    "iowa",
    "kansas",
    "kentucky",
    "louisiana",
    "maine",
    "maryland",
    "massachusetts",
    "michigan",
    "minnesota",
    "mississippi",
    "missouri",
    "montana",
    "nebraska",
    "nevada",
    "new hampshire",
    "new jersey",
    "new mexico",
    "north carolina",
    "north dakota",
    "ohio",
    "oklahoma",
    "oregon",
    "pennsylvania",
    "rhode island",
    "south carolina",
    "south dakota",
    "tennessee",
    "texas",
    "utah",
    "vermont",
    "virginia",
    "washington",
    "west virginia",
    "wisconsin",
    "wyoming",
    "district of columbia",
    "washington dc",
    "washington d.c.",
    "u.s.",
    "u.s",
    "the us",
    "the u.s.",
    "the u.s",
    "the united states",
    "the usa",
    "any state",
];
const CA_PROVINCES: &[&str] = &[
    "ontario",
    "quebec",
    "québec",
    "british columbia",
    "alberta",
    "manitoba",
    "saskatchewan",
    "nova scotia",
    "new brunswick",
    "newfoundland",
    "prince edward island",
];

const CITIES: &[(&str, &str)] = &[
    ("new york", "US"),
    ("new york city", "US"),
    ("nyc", "US"),
    ("san francisco", "US"),
    ("bay area", "US"),
    ("sf bay area", "US"),
    ("silicon valley", "US"),
    ("los angeles", "US"),
    ("chicago", "US"),
    ("boston", "US"),
    ("seattle", "US"),
    ("austin", "US"),
    ("denver", "US"),
    ("atlanta", "US"),
    ("miami", "US"),
    ("houston", "US"),
    ("dallas", "US"),
    ("palo alto", "US"),
    ("menlo park", "US"),
    ("mountain view", "US"),
    ("san jose", "US"),
    ("salt lake city", "US"),
    ("philadelphia", "US"),
    ("phoenix", "US"),
    ("san diego", "US"),
    ("pittsburgh", "US"),
    ("detroit", "US"),
    ("minneapolis", "US"),
    ("nashville", "US"),
    ("raleigh", "US"),
    ("waltham", "US"),
    ("brooklyn", "US"),
    ("toronto", "CA"),
    ("montreal", "CA"),
    ("montréal", "CA"),
    ("vancouver", "CA"),
    ("ottawa", "CA"),
    ("calgary", "CA"),
    ("waterloo", "CA"),
    ("edmonton", "CA"),
    ("london", "GB"),
    ("manchester", "GB"),
    ("edinburgh", "GB"),
    ("paris", "FR"),
    ("berlin", "DE"),
    ("munich", "DE"),
    ("münchen", "DE"),
    ("hamburg", "DE"),
    ("frankfurt", "DE"),
    ("amsterdam", "NL"),
    ("dublin", "IE"),
    ("madrid", "ES"),
    ("barcelona", "ES"),
    ("lisbon", "PT"),
    ("warsaw", "PL"),
    ("krakow", "PL"),
    ("kraków", "PL"),
    ("prague", "CZ"),
    ("vienna", "AT"),
    ("zurich", "CH"),
    ("zürich", "CH"),
    ("geneva", "CH"),
    ("lausanne", "CH"),
    ("copenhagen", "DK"),
    ("stockholm", "SE"),
    ("oslo", "NO"),
    ("helsinki", "FI"),
    ("brussels", "BE"),
    ("milan", "IT"),
    ("rome", "IT"),
    ("athens", "GR"),
    ("bucharest", "RO"),
    ("budapest", "HU"),
    ("sofia", "BG"),
    ("tallinn", "EE"),
    ("riga", "LV"),
    ("vilnius", "LT"),
    ("kyiv", "UA"),
    ("istanbul", "TR"),
    ("tokyo", "JP"),
    ("seoul", "KR"),
    ("beijing", "CN"),
    ("shanghai", "CN"),
    ("shenzhen", "CN"),
    ("taipei", "TW"),
    ("bangalore", "IN"),
    ("bengaluru", "IN"),
    ("mumbai", "IN"),
    ("delhi", "IN"),
    ("new delhi", "IN"),
    ("gurgaon", "IN"),
    ("gurugram", "IN"),
    ("pune", "IN"),
    ("hyderabad", "IN"),
    ("chennai", "IN"),
    ("noida", "IN"),
    ("jakarta", "ID"),
    ("bandung", "ID"),
    ("surabaya", "ID"),
    ("yogyakarta", "ID"),
    ("semarang", "ID"),
    ("medan", "ID"),
    ("denpasar", "ID"),
    ("bali", "ID"),
    ("tangerang", "ID"),
    ("bekasi", "ID"),
    ("depok", "ID"),
    ("batam", "ID"),
    ("haifa", "IL"),
    ("herzliya", "IL"),
    ("jerusalem", "IL"),
    ("ramat gan", "IL"),
    ("kuala lumpur", "MY"),
    ("bangkok", "TH"),
    ("manila", "PH"),
    ("ho chi minh city", "VN"),
    ("hanoi", "VN"),
    ("sydney", "AU"),
    ("melbourne", "AU"),
    ("brisbane", "AU"),
    ("perth", "AU"),
    ("auckland", "NZ"),
    ("wellington", "NZ"),
    ("dubai", "AE"),
    ("abu dhabi", "AE"),
    ("riyadh", "SA"),
    ("tel aviv", "IL"),
    ("cairo", "EG"),
    ("lagos", "NG"),
    ("nairobi", "KE"),
    ("cape town", "ZA"),
    ("johannesburg", "ZA"),
    ("mexico city", "MX"),
    ("guadalajara", "MX"),
    ("são paulo", "BR"),
    ("sao paulo", "BR"),
    ("rio de janeiro", "BR"),
    ("buenos aires", "AR"),
    ("bogotá", "CO"),
    ("bogota", "CO"),
    ("medellín", "CO"),
    ("medellin", "CO"),
    ("lima", "PE"),
    ("montevideo", "UY"),
];

/// A place inside a country, by name (lowercase): the country's code, and
/// whether it is a city (a city may mean an office there; a state or
/// province is just where in the country).
pub fn place_country(name: &str) -> Option<(&'static str, bool)> {
    let n = name.trim();
    if US_STATES.contains(&n) {
        return Some(("US", false));
    }
    if CA_PROVINCES.contains(&n) {
        return Some(("CA", false));
    }
    CITIES
        .iter()
        .find(|(c, _)| *c == n)
        .map(|(_, code)| (*code, true))
}

const US_STATE_CODES: &[&str] = &[
    "al", "ak", "az", "ar", "ca", "co", "ct", "de", "fl", "ga", "hi", "id", "il", "in", "ia", "ks",
    "ky", "la", "me", "md", "ma", "mi", "mn", "ms", "mo", "mt", "ne", "nv", "nh", "nj", "nm", "ny",
    "nc", "nd", "oh", "ok", "or", "pa", "ri", "sc", "sd", "tn", "tx", "ut", "vt", "va", "wa", "wv",
    "wi", "wy", "dc",
];
const CA_PROVINCE_CODES: &[&str] = &[
    "ab", "bc", "mb", "nb", "nl", "ns", "nt", "nu", "on", "pe", "qc", "sk", "yt",
];
const IN_STATE_CODES: &[&str] = &[
    "ap", "as", "br", "cg", "dl", "ga", "gj", "hp", "hr", "jh", "jk", "ka", "kl", "mh", "mp", "od",
    "pb", "rj", "tn", "tg", "up", "uk", "wb",
];
const BR_STATE_CODES: &[&str] = &[
    "ac", "al", "am", "ap", "ba", "ce", "df", "es", "go", "ma", "mg", "ms", "mt", "pa", "pb", "pe",
    "pi", "pr", "rj", "rn", "ro", "rr", "rs", "sc", "se", "sp", "to",
];
const AU_STATE_CODES: &[&str] = &["nt", "sa", "wa"];

fn subdivision_codes(country: &str) -> &'static [&'static str] {
    match country {
        "US" => US_STATE_CODES,
        "CA" => CA_PROVINCE_CODES,
        "IN" => IN_STATE_CODES,
        "BR" => BR_STATE_CODES,
        "AU" => AU_STATE_CODES,
        _ => &[],
    }
}

pub fn code_after_place(code: &str, place_country: Option<&str>) -> Option<&'static str> {
    let code = code.trim().to_lowercase();
    if let Some(near) = place_country.and_then(to_alpha2)
        && (near.eq_ignore_ascii_case(&code) || subdivision_codes(near).contains(&code.as_str()))
    {
        return Some(near);
    }
    if US_STATE_CODES.contains(&code.as_str()) {
        return Some("US");
    }
    if let Some(c) = to_alpha2(&code) {
        return Some(c);
    }
    let mut owners = ["CA", "IN", "BR", "AU"]
        .into_iter()
        .filter(|c| subdivision_codes(c).contains(&code.as_str()));
    match (owners.next(), owners.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    }
}

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
