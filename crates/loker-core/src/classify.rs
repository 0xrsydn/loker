use serde::{Deserialize, Serialize};

use crate::model::NewJob;
use crate::role::{self, JobRole, MAX_ROLES, Modifier, Role, RoleMethod};

pub const RULES: u32 = 2;

pub const QUESTION: u32 = 1;

pub const MODEL_THRESHOLD: f64 = 0.6;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoleAnswer {
    pub model: String,
    pub choice: String,
    pub confidence: f64,
    #[serde(default)]
    pub top: Vec<(String, f64)>,
}

pub fn model_state(job: &NewJob) -> serde_json::Value {
    let description: String = job
        .description_md
        .as_deref()
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(800)
        .collect();
    serde_json::json!({
        "title": job.title,
        "company": job.company,
        "category": job.category.as_deref().unwrap_or(""),
        "description": description,
    })
}

pub fn answer_key(job: &NewJob) -> String {
    let state = model_state(job).to_string();
    let seed = format!("{}|{QUESTION}|{state}", role::VERSION);
    blake3::hash(seed.as_bytes()).to_hex()[..32].to_string()
}

pub fn question() -> serde_json::Value {
    let mut criteria = serde_json::Map::new();
    for r in Role::ALL {
        let i = r.info();
        criteria.insert(
            serde_json::to_value(r)
                .expect("role id")
                .as_str()
                .expect("string")
                .into(),
            format!("{}: {}", i.label, i.definition).into(),
        );
    }
    criteria.insert(
        "none".into(),
        "None of these: the job is outside this vocabulary (e.g. cooking, driving, retail).".into(),
    );
    serde_json::json!({
        "role": {
            "type": "choice",
            "instructions": "What does the person in this job do? Pick the most specific option; \
                             a family only when the posting doesn't make the role clear.",
            "criteria": criteria,
        }
    })
}

pub const STAMP: i64 = role::VERSION as i64 * 1000 + RULES as i64;

use Role::*;

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
    (Design, &["designer", "design"]),
];

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

pub fn classify(job: &NewJob) -> Option<JobRole> {
    classify_with(job, None)
}

pub fn classify_with(job: &NewJob, answer: Option<&RoleAnswer>) -> Option<JobRole> {
    let rules = rules(job);
    let Some(a) = answer else {
        return rules;
    };
    let title_names_it = rules
        .as_ref()
        .is_some_and(|r| r.method == RoleMethod::Title && r.confidence >= 0.9);
    if title_names_it && a.confidence < MODEL_THRESHOLD {
        return rules;
    }
    let modifiers = rules
        .as_ref()
        .map(|r| r.modifiers.clone())
        .unwrap_or_default();
    let chosen: Option<Role> = serde_json::from_value(serde_json::json!(a.choice)).ok();
    let primary = chosen?;
    let found: Vec<(Role, &str)> = match &rules {
        Some(r) if r.primary == primary => r.secondary.iter().map(|&s| (s, "from rules")).collect(),
        _ => Vec::new(),
    };
    Some(build(
        primary,
        &found,
        modifiers,
        RoleMethod::Model,
        a.confidence,
        format!("model: {} ({:.2})", a.model, a.confidence),
    ))
}

fn rules(job: &NewJob) -> Option<JobRole> {
    let title = words(&job.title);
    let modifiers: Vec<Modifier> = MODIFIERS
        .iter()
        .filter(|(_, ps)| ps.iter().any(|p| has(&title, p)))
        .map(|(m, _)| *m)
        .collect();
    let found = matches(&title, TITLE);

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

fn is_generic(r: Role) -> bool {
    matches!(r, Software | Design)
}

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
    if primary == ProductEngineer {
        secondary.push(Fullstack);
    }
    let tech = |r: Role| r.within(Engineering) || r.within(Data);
    for &(r, phrase) in found {
        if secondary.len() + 1 >= MAX_ROLES {
            break;
        }
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

fn matches(text: &str, table: &[(Role, &'static [&'static str])]) -> Vec<(Role, &'static str)> {
    let mut text = text.to_string();
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

fn words(text: &str) -> String {
    let lower = text.to_lowercase();
    let ws: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    format!(" {} ", ws.join(" "))
}
