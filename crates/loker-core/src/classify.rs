//! Which role a job is ([`crate::role`]), from its title, then the source's
//! category, then the start of its description. No IO: a job in, a
//! [`JobRole`] or nothing out.
//!
//! The title decides when it names a role: it is the employer's own words. A
//! category or the description only refines a title that says just
//! "engineer" (or nothing), never overrides a specific one. Phrases are whole
//! words, matched on lowercase text with punctuation as spaces, so
//! "Full-Stack" is "full stack" and "ML/AI" is "ml ai". Each table lists the
//! most specific role first; the first match is the primary role, later
//! matches of other roles are secondary.
//!
//! Change a table, bump [`RULES`]: stored jobs are classified again when the
//! store opens (see `Store::open`).
use crate::model::NewJob;
use crate::role::{self, JobRole, MAX_ROLES, Modifier, Role, RoleMethod};

/// Version of these rules, bumped on any change to them.
pub const RULES: u32 = 1;

/// The rules and vocabulary versions in one number, stored with each job's
/// role so the store knows which jobs to classify again.
pub const STAMP: i64 = role::VERSION as i64 * 1000 + RULES as i64;

use Role::*;

/// Title phrases per role, most specific first. A role may appear twice
/// when some of its phrases must win over a role in between.
const TITLE: &[(Role, &[&str])] = &[
    (
        EngineeringManagement,
        &[
            "engineering manager",
            "manager engineering",
            "head of engineering",
            "director of engineering",
            "engineering director",
            "vp engineering",
            "vp of engineering",
            "vice president engineering",
            "vice president of engineering",
            "cto",
            "chief technology officer",
        ],
    ),
    (
        AiTraining,
        &[
            "ai trainer",
            "ai training",
            "ai tutor",
            "data annotator",
            "annotator",
            "annotation",
            "data labeling",
            "data labelling",
            "rlhf",
            "rater",
            "task designer",
            "talent pool",
            "freelance expert",
            "expert",
        ],
    ),
    (
        UxResearch,
        &[
            "ux researcher",
            "user researcher",
            "ux research",
            "user research",
            "design researcher",
        ],
    ),
    (
        MlInfrastructure,
        &[
            "inference",
            "mlops",
            "ml ops",
            "ml infrastructure",
            "ml infra",
            "ml platform",
            "machine learning infrastructure",
            "machine learning platform",
            "training infrastructure",
            "model serving",
            "gpu",
            "cuda",
            "kernel engineer",
        ],
    ),
    (
        Research,
        &[
            "research engineer",
            "research scientist",
            "applied scientist",
            "researcher",
            "research",
        ],
    ),
    (DataScience, &["data scientist", "data science"]),
    (
        MlEngineering,
        &[
            "machine learning engineer",
            "ml engineer",
            "ai ml engineer",
            "ml ai engineer",
            "machine learning",
            "deep learning",
            "computer vision",
            "nlp engineer",
        ],
    ),
    (
        AiEngineering,
        &[
            "ai engineer",
            "ai engineering",
            "applied ai",
            "llm",
            "llms",
            "genai",
            "gen ai",
            "generative ai",
            "agent engineer",
            "agents",
            "ai agent",
            "ai agents",
            "agentic",
            "ai developer",
            "prompt engineer",
            "ai software engineer",
        ],
    ),
    (
        DataEngineering,
        &[
            "data engineer",
            "data engineering",
            "analytics engineer",
            "data platform",
            "data infrastructure",
            "etl",
            "dbt",
            "big data",
        ],
    ),
    (
        DataAnalysis,
        &[
            "data analyst",
            "bi analyst",
            "business intelligence",
            "analytics analyst",
            "product analyst",
            "insights analyst",
            "reporting analyst",
            "data analytics",
        ],
    ),
    (
        Security,
        &[
            "security engineer",
            "security",
            "cybersecurity",
            "devsecops",
            "appsec",
            "penetration",
            "pentester",
            "soc analyst",
            "threat",
            "detection engineer",
            "incident response",
            "red team",
        ],
    ),
    (
        SalesEngineering,
        &[
            "solutions engineer",
            "solution engineer",
            "sales engineer",
            "pre sales",
            "presales",
            "solutions architect",
            "solution architect",
            "customer engineer",
            "solutions consultant",
            "consulting architect",
            "deployment strategist",
        ],
    ),
    (
        Devrel,
        &[
            "developer relations",
            "developer advocate",
            "developer evangelist",
            "devrel",
            "developer experience",
            "devex",
        ],
    ),
    (
        TechnicalWriting,
        &[
            "technical writer",
            "documentation engineer",
            "docs engineer",
            "technical writing",
        ],
    ),
    (
        Qa,
        &[
            "qa",
            "quality assurance",
            "test engineer",
            "test automation",
            "automation tester",
            "sdet",
            "tester",
            "quality engineer",
        ],
    ),
    (
        Embedded,
        &[
            "embedded",
            "firmware",
            "robotics",
            "robotic",
            "controls engineer",
        ],
    ),
    (
        Hardware,
        &[
            "hardware",
            "mechanical engineer",
            "mechanical design",
            "mechanical",
            "electrical engineer",
            "electrical design",
            "pcb",
            "asic",
            "silicon",
            "fpga",
            "rf engineer",
            "manufacturing engineer",
        ],
    ),
    (
        DesignEngineer,
        &[
            "design engineer",
            "design engineering",
            "design technologist",
            "ux engineer",
        ],
    ),
    (
        Mobile,
        &[
            "ios",
            "android",
            "mobile",
            "react native",
            "flutter",
            "swift developer",
            "swiftui",
        ],
    ),
    (
        Infrastructure,
        &[
            "devops",
            "dev ops",
            "sre",
            "site reliability",
            "platform engineer",
            "platform engineering",
            "infrastructure",
            "cloud engineer",
            "cloud architect",
            "kubernetes",
            "production engineer",
            "developer productivity",
            "build engineer",
            "release engineer",
            "network engineer",
            "systems administrator",
            "sysadmin",
            "systems engineer",
        ],
    ),
    (
        Backend,
        &[
            "backend",
            "back end",
            "api engineer",
            "server side",
            "distributed systems",
            "golang",
            "go developer",
            "go engineer",
            "java developer",
            "java engineer",
            "python developer",
            "python engineer",
            "rust engineer",
            "rust developer",
            "ruby developer",
            "rails developer",
            "php developer",
            "laravel",
            "node js developer",
            "nodejs developer",
            "net developer",
            "c developer",
            "scala engineer",
            "elixir",
        ],
    ),
    (
        Frontend,
        &[
            "frontend",
            "front end",
            "ui engineer",
            "ui developer",
            "web developer",
            "react developer",
            "react engineer",
            "javascript developer",
            "typescript developer",
            "vue",
            "angular",
            "wordpress developer",
            "shopify developer",
        ],
    ),
    (
        ProductEngineer,
        &["product engineer", "product engineering", "growth engineer"],
    ),
    (Fullstack, &["fullstack", "full stack"]),
    (
        ProductManagement,
        &[
            "product manager",
            "product owner",
            "product management",
            "head of product",
            "vp product",
            "vp of product",
            "director of product",
            "chief product officer",
            "cpo",
            "product lead",
            "product strategist",
        ],
    ),
    (
        ProgramManagement,
        &[
            "technical program manager",
            "program manager",
            "programme manager",
            "project manager",
            "tpm",
            "delivery manager",
            "scrum master",
            "project coordinator",
            "project lead",
            "program management",
            "project management",
        ],
    ),
    (
        ProductDesign,
        &[
            "product designer",
            "product design",
            "ux designer",
            "ui designer",
            "ux ui",
            "ui ux",
            "interaction designer",
            "ux",
        ],
    ),
    (
        BrandDesign,
        &[
            "brand designer",
            "graphic designer",
            "visual designer",
            "motion designer",
            "illustrator",
            "creative designer",
            "animator",
            "art director",
            "video editor",
        ],
    ),
    (ProductMarketing, &["product marketing", "pmm"]),
    (
        CustomerSuccess,
        &[
            "customer success",
            "technical account manager",
            "implementation",
            "onboarding specialist",
            "onboarding manager",
            "client services",
            "client success",
            "professional services",
            "engagement manager",
        ],
    ),
    (
        TechnicalSupport,
        &[
            "support engineer",
            "technical support",
            "product support",
            "application support",
        ],
    ),
    (
        It,
        &[
            "it support",
            "it specialist",
            "it administrator",
            "it manager",
            "it engineer",
            "help desk",
            "helpdesk",
            "service desk",
            "desktop support",
            "business systems",
            "salesforce administrator",
            "salesforce admin",
            "director of it",
            "head of it",
            "workplace technology",
        ],
    ),
    (
        Marketing,
        &[
            "marketing",
            "marketer",
            "growth marketing",
            "seo",
            "sem",
            "content writer",
            "content creator",
            "content strategist",
            "copywriter",
            "social media",
            "community manager",
            "demand generation",
            "lead generation",
            "lead gen",
            "paid media",
            "media buyer",
            "communications",
            "public relations",
            "brand manager",
            "growth",
        ],
    ),
    (
        Sales,
        &[
            "sales",
            "account executive",
            "business development",
            "business developer",
            "bdr",
            "sdr",
            "account manager",
            "partnerships",
            "partnership",
            "partner manager",
            "channel manager",
            "closer",
        ],
    ),
    (
        CustomerSupport,
        &[
            "customer support",
            "customer service",
            "customer care",
            "customer experience",
            "support specialist",
            "support agent",
            "support representative",
            "support associate",
            "call center",
        ],
    ),
    (
        Recruiting,
        &[
            "recruiter",
            "recruiting",
            "recruitment",
            "talent acquisition",
            "sourcer",
            "interview coordinator",
            "interview coordination",
        ],
    ),
    (
        People,
        &[
            "human resources",
            "hr manager",
            "hr generalist",
            "hr specialist",
            "hr coordinator",
            "hr business partner",
            "hr operations",
            "hrbp",
            "people partner",
            "people operations",
            "people ops",
            "payroll",
            "learning and development",
        ],
    ),
    (
        Compliance,
        &[
            "compliance",
            "aml",
            "kyc",
            "kyb",
            "fraud",
            "risk",
            "sanctions",
            "trust and safety",
            "trust safety",
            "grc",
            "surveillance",
            "regulatory",
            "mlro",
            "investigator",
            "investigations",
            "content moderator",
        ],
    ),
    (
        Legal,
        &[
            "counsel",
            "lawyer",
            "attorney",
            "legal",
            "paralegal",
            "contracts manager",
            "law",
        ],
    ),
    (
        Finance,
        &[
            "accountant",
            "accounting",
            "finance",
            "financial",
            "fp a",
            "controller",
            "tax",
            "audit",
            "auditor",
            "treasury",
            "bookkeeper",
            "bookkeeping",
            "accounts payable",
            "accounts receivable",
            "billing",
            "trader",
            "trading",
            "internal controls",
        ],
    ),
    (
        Admin,
        &[
            "executive assistant",
            "virtual assistant",
            "personal assistant",
            "administrative",
            "admin assistant",
            "office manager",
            "office administrator",
            "data entry",
            "receptionist",
        ],
    ),
    (
        BusinessOperations,
        &[
            "chief of staff",
            "business operations",
            "bizops",
            "revenue operations",
            "revops",
            "operations",
            "strategy",
            "procurement",
            "supply chain",
            "business analyst",
            "operations analyst",
            "general manager",
            "category manager",
            "market analyst",
            "capacity planning",
            "government relations",
            "public policy",
        ],
    ),
    (
        Education,
        &[
            "teacher",
            "tutor",
            "instructor",
            "teaching",
            "lecturer",
            "curriculum",
            "educator",
        ],
    ),
    (
        Healthcare,
        &[
            "nurse",
            "physician",
            "doctor",
            "clinical",
            "medical",
            "therapist",
            "pharmacist",
            "dentist",
            "psychologist",
            "counselor",
            "telehealth",
            "psychiatrist",
        ],
    ),
    // Last: engineering whose specialty the title doesn't state.
    (
        Software,
        &[
            "software engineer",
            "software developer",
            "software engineering",
            "member of technical staff",
            "developer",
            "engineer",
            "programmer",
            "swe",
            "coder",
        ],
    ),
    // A designer of no stated kind (game designer, "Designer").
    (Design, &["designer", "design"]),
];

/// Engineers that aren't software, hardware or anything this vocabulary has.
const NOT_SOFTWARE: &[&str] = &[
    "civil engineer",
    "structural engineer",
    "chemical engineer",
    "process engineer",
    "field engineer",
    "field service engineer",
    "sound engineer",
    "audio engineer",
    "environmental engineer",
    "geotechnical engineer",
    "biomedical engineer",
    "petroleum engineer",
];

/// Category phrases that name a role or only a family. Matched like titles,
/// after the role phrases of [`TITLE`]; unknown categories ("Delta", "VIP")
/// match nothing.
const CATEGORY: &[(Role, &[&str])] = &[
    (Backend, &["back end programming"]),
    (Frontend, &["front end programming"]),
    (Fullstack, &["full stack programming"]),
    (Infrastructure, &["devops and sysadmin"]),
    (CustomerSupport, &["customer service", "customer support"]),
    (DataEngineering, &["data engineering"]),
    (Data, &["data science", "data", "analytics"]),
    (Ai, &["ai", "artificial intelligence", "machine learning"]),
    (
        Engineering,
        &[
            "engineering",
            "software engineering",
            "software development",
            "developer",
            "programming",
            "tech",
            "technology",
            "r d",
        ],
    ),
    (Research, &["research"]),
    (Design, &["design", "creative"]),
    (Product, &["product"]),
    (
        GoToMarket,
        &[
            "sales and marketing",
            "marketing sales",
            "go to market",
            "gtm",
        ],
    ),
    (Support, &["support", "customer success"]),
    (
        Operations,
        &["operations", "management and finance", "business"],
    ),
    (Education, &["education", "teaching"]),
    (Healthcare, &["healthcare", "medical", "health"]),
];

const MODIFIERS: &[(Modifier, &[&str])] = &[
    (
        Modifier::ForwardDeployed,
        &["forward deployed", "deployment strategist", "fde"],
    ),
    (Modifier::Founding, &["founding"]),
];

/// The job's role, or `None` when nothing places it.
pub fn classify(job: &NewJob) -> Option<JobRole> {
    let title = words(&job.title);
    let modifiers: Vec<Modifier> = MODIFIERS
        .iter()
        .filter(|(_, ps)| ps.iter().any(|p| has(&title, p)))
        .map(|(m, _)| *m)
        .collect();
    let found = matches(&title, TITLE);

    // A title that names a role decides.
    if let Some(&(primary, phrase)) = found.iter().find(|(r, _)| !is_generic(*r)) {
        return Some(build(
            primary,
            &found,
            modifiers,
            RoleMethod::Title,
            0.9,
            format!("title: {phrase}"),
        ));
    }
    let generic = found.first().copied();

    // A title that says only "engineer" or nothing: the source's category.
    // A known category ("Data Science", "DevOps and Sysadmin") maps as a
    // whole; otherwise a role phrase inside it ("Product Management").
    if let Some(c) = job.category.as_deref() {
        let parts: Vec<String> = c.split(';').map(|p| words(p).trim().to_string()).collect();
        let known = CATEGORY.iter().find_map(|(r, ps)| {
            ps.iter()
                .find(|p| parts.iter().any(|part| part == *p))
                .map(|p| (*r, *p))
        });
        let named = || {
            matches(&words(c), TITLE)
                .into_iter()
                .find(|(r, _)| !is_generic(*r))
        };
        let refines = |r: Role| fits(generic, r) && !generic.is_some_and(|(g, _)| g.within(r));
        if let Some((role, _)) = known.or_else(named).filter(|(r, _)| refines(*r)) {
            let confidence = if role.is_family() { 0.5 } else { 0.7 };
            return Some(build(
                role,
                &[],
                modifiers,
                RoleMethod::Category,
                confidence,
                format!("category: {c}"),
            ));
        }
    }

    // Then a role named near the start of the description ("as a data engineer").
    if let Some(d) = job.description_md.as_deref() {
        let start: String = d.chars().take(600).collect();
        let desc = words(&start);
        let named = matches(&desc, TITLE)
            .into_iter()
            .find(|(r, p)| !is_generic(*r) && p.contains(' ') && fits(generic, *r));
        if let Some((role, phrase)) = named {
            return Some(build(
                role,
                &[],
                modifiers,
                RoleMethod::Description,
                0.5,
                format!("description: {phrase}"),
            ));
        }
    }

    let (role, phrase) = generic?;
    Some(build(
        role,
        &[],
        modifiers,
        RoleMethod::Title,
        0.6,
        format!("title: {phrase}"),
    ))
}

/// Whether a title naming both may give the job both: roles in one family,
/// or two technical roles ("Data Scientist / ML Engineer"). Otherwise the
/// second is a domain ("Data Engineer, Marketing Analytics").
fn related(a: Role, b: Role) -> bool {
    let top = |mut r: Role| {
        while let Some(h) = r.info().home {
            r = h;
        }
        r
    };
    let tech = |r: Role| r.within(Engineering) || r.within(Data);
    top(a) == top(b) || (tech(a) && tech(b))
}

/// `software` (no stated specialty) and the bare `design` family.
fn is_generic(r: Role) -> bool {
    matches!(r, Software | Design)
}

/// Whether evidence for `role` may refine a generic title match: an
/// "engineer" title takes only engineering roles.
fn fits(generic: Option<(Role, &str)>, role: Role) -> bool {
    match generic {
        Some((g, _)) => role.within(g.info().home.unwrap_or(g)) || role.within(g),
        None => true,
    }
}

fn build(
    primary: Role,
    found: &[(Role, &str)],
    modifiers: Vec<Modifier>,
    method: RoleMethod,
    confidence: f64,
    evidence: String,
) -> JobRole {
    let mut secondary: Vec<Role> = Vec::new();
    // A product engineer is a full-stack engineer who also owns the product:
    // full-stack searches should find one.
    if primary == ProductEngineer {
        secondary.push(Fullstack);
    }
    let tech = |r: Role| r.within(Engineering) || r.within(Data);
    for &(r, phrase) in found {
        if secondary.len() + 1 >= MAX_ROLES {
            break;
        }
        // A single word next to a role is usually a qualifier ("Sales
        // Solutions Engineer"); it counts only between two technical roles
        // ("Backend / Infrastructure Engineer").
        let strong = phrase.contains(' ') || (tech(primary) && tech(r));
        if r != primary
            && !is_generic(r)
            && !secondary.contains(&r)
            && !primary.within(r)
            && related(primary, r)
            && strong
        {
            secondary.push(r);
        }
    }
    JobRole {
        primary,
        secondary,
        modifiers,
        method,
        confidence,
        evidence: Some(evidence),
        version: role::VERSION,
    }
}

/// Every role whose phrase occurs, each role once with the phrase that
/// matched: multi-word phrases first (a role noun like "technical program
/// manager" beats a keyword like "security"), then single words, each in
/// table order. A matched phrase is used up, so its words can't also match
/// another role ("ux researcher" is not also "ux"). False friends are taken
/// out first.
fn matches(text: &str, table: &[(Role, &'static [&'static str])]) -> Vec<(Role, &'static str)> {
    let mut text = text.to_string();
    // A mechanical or electrical "design engineer" is hardware.
    for p in [
        "mechanical design engineer",
        "electrical design engineer",
        "hardware design engineer",
    ] {
        text = text.replace(&format!(" {p} "), " hardware ");
    }
    for p in NOT_SOFTWARE {
        text = text.replace(&format!(" {p} "), " ");
    }
    let mut out: Vec<(Role, &str)> = Vec::new();
    for multi in [true, false] {
        for (role, phrases) in table {
            if out.iter().any(|(r, _)| r == role) {
                continue;
            }
            let found = phrases
                .iter()
                .filter(|p| p.contains(' ') == multi)
                .find(|p| has(&text, p));
            if let Some(p) = found {
                out.push((*role, p));
                text = text.replace(&format!(" {p} "), " ");
            }
        }
    }
    out
}

fn has(text: &str, phrase: &str) -> bool {
    text.contains(&format!(" {phrase} "))
}

/// Lowercase alphanumeric words joined by single spaces and padded with a
/// space on each side, so a phrase only matches whole words.
fn words(text: &str) -> String {
    let lower = text.to_lowercase();
    let ws: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    format!(" {} ", ws.join(" "))
}
