use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 1;

pub const MAX_ROLES: usize = 3;

/// A family (`engineering`) or a role in one (`backend`). The list is open:
/// it grows, and clients must accept ids they don't know.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Engineering,
    Ai,
    Data,
    Product,
    Design,
    GoToMarket,
    Support,
    Operations,
    AiTraining,
    Education,
    Healthcare,
    Software,
    Backend,
    Frontend,
    Fullstack,
    ProductEngineer,
    Mobile,
    Infrastructure,
    Security,
    Qa,
    Embedded,
    Hardware,
    TechnicalWriting,
    EngineeringManagement,
    DesignEngineer,
    Devrel,
    AiEngineering,
    MlEngineering,
    MlInfrastructure,
    Research,
    DataEngineering,
    DataScience,
    DataAnalysis,
    ProductManagement,
    ProgramManagement,
    ProductDesign,
    UxResearch,
    BrandDesign,
    Sales,
    Marketing,
    ProductMarketing,
    SalesEngineering,
    CustomerSuccess,
    CustomerSupport,
    TechnicalSupport,
    BusinessOperations,
    Finance,
    Legal,
    Compliance,
    People,
    Recruiting,
    It,
    Admin,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoleInfo {
    pub label: &'static str,
    pub definition: &'static str,
    pub home: Option<Role>,
    pub also_in: &'static [Role],
    pub replaced_by: Option<Role>,
}

const fn info(
    label: &'static str,
    definition: &'static str,
    home: Option<Role>,
    also_in: &'static [Role],
) -> RoleInfo {
    RoleInfo {
        label,
        definition,
        home,
        also_in,
        replaced_by: None,
    }
}

impl Role {
    pub const ALL: &[Role] = &[
        Role::Engineering,
        Role::Ai,
        Role::Data,
        Role::Product,
        Role::Design,
        Role::GoToMarket,
        Role::Support,
        Role::Operations,
        Role::AiTraining,
        Role::Education,
        Role::Healthcare,
        Role::Software,
        Role::Backend,
        Role::Frontend,
        Role::Fullstack,
        Role::ProductEngineer,
        Role::Mobile,
        Role::Infrastructure,
        Role::Security,
        Role::Qa,
        Role::Embedded,
        Role::Hardware,
        Role::TechnicalWriting,
        Role::EngineeringManagement,
        Role::DesignEngineer,
        Role::Devrel,
        Role::AiEngineering,
        Role::MlEngineering,
        Role::MlInfrastructure,
        Role::Research,
        Role::DataEngineering,
        Role::DataScience,
        Role::DataAnalysis,
        Role::ProductManagement,
        Role::ProgramManagement,
        Role::ProductDesign,
        Role::UxResearch,
        Role::BrandDesign,
        Role::Sales,
        Role::Marketing,
        Role::ProductMarketing,
        Role::SalesEngineering,
        Role::CustomerSuccess,
        Role::CustomerSupport,
        Role::TechnicalSupport,
        Role::BusinessOperations,
        Role::Finance,
        Role::Legal,
        Role::Compliance,
        Role::People,
        Role::Recruiting,
        Role::It,
        Role::Admin,
    ];

    pub fn info(self) -> RoleInfo {
        use Role::*;
        let eng = Some(Engineering);
        match self {
            Engineering => info("Engineering", "Builds software and hardware.", None, &[]),
            Ai => info(
                "AI and ML",
                "Builds, trains, serves or researches AI models.",
                eng,
                &[],
            ),
            Data => info("Data", "Moves, models and analyses data.", None, &[]),
            Product => info(
                "Product",
                "Decides what gets built and when it ships.",
                None,
                &[],
            ),
            Design => info(
                "Design",
                "Designs products, interfaces and brands.",
                None,
                &[],
            ),
            GoToMarket => info(
                "Go to market",
                "Sells, markets and grows accounts.",
                None,
                &[],
            ),
            Support => info("Support", "Helps customers use the product.", None, &[]),
            Operations => info(
                "Operations",
                "Runs the business: ops, finance, legal, people, IT.",
                None,
                &[],
            ),
            AiTraining => info(
                "AI training",
                "Contract labelling, rating and expert-review work that trains AI models.",
                None,
                &[],
            ),
            Education => info("Education", "Teaching and tutoring.", None, &[]),
            Healthcare => info("Healthcare", "Clinical and medical practice.", None, &[]),

            Software => info(
                "Software engineering",
                "Engineering whose specialty the job doesn't state.",
                eng,
                &[],
            ),
            Backend => info("Backend", "Servers, APIs and services.", eng, &[]),
            Frontend => info("Frontend", "Web interfaces.", eng, &[]),
            Fullstack => info("Full stack", "Both backend and frontend.", eng, &[]),
            ProductEngineer => info(
                "Product engineering",
                "Full-stack engineers who also own product decisions with users.",
                eng,
                &[],
            ),
            Mobile => info("Mobile", "iOS, Android and cross-platform apps.", eng, &[]),
            Infrastructure => info(
                "Infrastructure",
                "DevOps, SRE, platform and cloud infrastructure.",
                eng,
                &[],
            ),
            Security => info(
                "Security",
                "Application, cloud and infrastructure security, and security operations.",
                eng,
                &[],
            ),
            Qa => info("QA", "Testing and test automation.", eng, &[]),
            Embedded => info(
                "Embedded",
                "Software on devices: firmware, robotics, controls.",
                eng,
                &[],
            ),
            Hardware => info(
                "Hardware",
                "Electrical, mechanical and silicon design.",
                eng,
                &[],
            ),
            TechnicalWriting => info(
                "Technical writing",
                "Documentation for developers and products.",
                eng,
                &[],
            ),
            EngineeringManagement => info(
                "Engineering management",
                "Managers, heads and directors of engineering teams.",
                eng,
                &[],
            ),
            DesignEngineer => info(
                "Design engineering",
                "Builds the interfaces it designs.",
                eng,
                &[Design],
            ),
            Devrel => info(
                "Developer relations",
                "Developer advocacy, developer experience and SDKs.",
                eng,
                &[GoToMarket],
            ),

            AiEngineering => info(
                "AI engineering",
                "Builds products on models: LLM apps, agents, retrieval, evals.",
                Some(Ai),
                &[],
            ),
            MlEngineering => info(
                "ML engineering",
                "Trains, fine-tunes and evaluates models.",
                Some(Ai),
                &[],
            ),
            MlInfrastructure => info(
                "ML infrastructure",
                "Inference and serving, GPU kernels, training infrastructure, MLOps.",
                Some(Ai),
                &[],
            ),
            Research => info(
                "Research",
                "Research engineers and scientists: training, alignment, interpretability.",
                Some(Ai),
                &[],
            ),

            DataEngineering => info(
                "Data engineering",
                "Pipelines, warehouses and analytics engineering.",
                Some(Data),
                &[Engineering],
            ),
            DataScience => info(
                "Data science",
                "Statistical, experimental and predictive modelling.",
                Some(Data),
                &[],
            ),
            DataAnalysis => info(
                "Data analysis",
                "Data, BI and product analysts.",
                Some(Data),
                &[],
            ),

            ProductManagement => info(
                "Product management",
                "Product managers of every kind, product owners, heads of product.",
                Some(Product),
                &[],
            ),
            ProgramManagement => info(
                "Program management",
                "Technical program, program, project and delivery managers.",
                Some(Product),
                &[],
            ),

            ProductDesign => info(
                "Product design",
                "Product, UX and UI design.",
                Some(Design),
                &[],
            ),
            UxResearch => info("UX research", "User and UX research.", Some(Design), &[]),
            BrandDesign => info(
                "Brand design",
                "Brand, graphic, visual and motion design.",
                Some(Design),
                &[],
            ),

            Sales => info(
                "Sales",
                "Account executives, business development, partnerships.",
                Some(GoToMarket),
                &[],
            ),
            Marketing => info(
                "Marketing",
                "Growth, content, brand and performance marketing.",
                Some(GoToMarket),
                &[],
            ),
            ProductMarketing => info(
                "Product marketing",
                "Positioning, launches and sales enablement.",
                Some(GoToMarket),
                &[],
            ),
            SalesEngineering => info(
                "Sales engineering",
                "Pre-sales solutions and sales engineers.",
                Some(GoToMarket),
                &[Engineering],
            ),
            CustomerSuccess => info(
                "Customer success",
                "Customer success, onboarding, implementation, account management after the sale.",
                Some(GoToMarket),
                &[Support],
            ),

            CustomerSupport => info(
                "Customer support",
                "Front-line customer service.",
                Some(Support),
                &[],
            ),
            TechnicalSupport => info(
                "Technical support",
                "Support engineers and technical product support.",
                Some(Support),
                &[],
            ),

            BusinessOperations => info(
                "Business operations",
                "Bizops, strategy, chief of staff, procurement, revenue operations.",
                Some(Operations),
                &[],
            ),
            Finance => info(
                "Finance",
                "Accounting, FP&A, tax and audit.",
                Some(Operations),
                &[],
            ),
            Legal => info(
                "Legal",
                "Counsel, paralegals and contracts.",
                Some(Operations),
                &[],
            ),
            Compliance => info(
                "Compliance",
                "Compliance, AML/KYC, fraud, risk, trust and safety.",
                Some(Operations),
                &[],
            ),
            People => info(
                "People",
                "HR, people operations, payroll.",
                Some(Operations),
                &[],
            ),
            Recruiting => info(
                "Recruiting",
                "Recruiters, sourcers and recruiting coordinators.",
                Some(Operations),
                &[],
            ),
            It => info(
                "IT",
                "Corporate IT, help desk and business systems.",
                Some(Operations),
                &[],
            ),
            Admin => info(
                "Administration",
                "Executive and virtual assistants, office management.",
                Some(Operations),
                &[],
            ),
        }
    }

    pub fn parents(self) -> Vec<Role> {
        let i = self.info();
        i.home
            .into_iter()
            .chain(i.also_in.iter().copied())
            .collect()
    }

    pub fn is_family(self) -> bool {
        self.info().home.is_none() || Role::ALL.iter().any(|r| r.info().home == Some(self))
    }

    pub fn within(self, filter: Role) -> bool {
        self == filter || self.parents().into_iter().any(|p| p.within(filter))
    }

    pub fn is_broader_than(self, other: Role) -> bool {
        self != other && other.within(self)
    }
}

/// How a job is done, whatever its role.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Modifier {
    /// Works with customers, on site or embedded, building for their problems.
    ForwardDeployed,
    /// Among a company's first hires.
    Founding,
}

impl Modifier {
    pub const ALL: &[Modifier] = &[Modifier::ForwardDeployed, Modifier::Founding];

    pub fn definition(self) -> &'static str {
        match self {
            Modifier::ForwardDeployed => {
                "Works with customers, on site or embedded, building for their problems."
            }
            Modifier::Founding => "Among a company's first hires.",
        }
    }
}

/// Where a job's role came from, most trusted first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RoleMethod {
    /// A phrase in the title.
    Title,
    /// The source's own category or department.
    Category,
    /// A phrase in the description.
    Description,
    /// A model choosing from this vocabulary.
    Model,
}

/// A job's role, and why it has it. A job no step could place has no
/// `JobRole` at all, rather than an "other" role.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobRole {
    /// The most specific id found: a role, or a family when that is all the
    /// evidence says.
    pub primary: Role,
    /// Other roles the job also fills, at most [`MAX_ROLES`] - 1
    /// ("Backend / Infrastructure Engineer").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secondary: Vec<Role>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modifiers: Vec<Modifier>,
    pub method: RoleMethod,
    pub confidence: f64,
    /// What decided it, e.g. "title: Forward Deployed Infrastructure Engineer".
    pub evidence: Option<String>,
    /// The vocabulary [`VERSION`] it was classified with.
    pub version: u32,
}

/// One entry of a role filter: an id at any level, or a modifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum RoleFilter {
    Role(Role),
    Modifier(Modifier),
}

impl JobRole {
    pub fn matches(&self, filters: &[RoleFilter]) -> bool {
        filters.iter().any(|f| match *f {
            RoleFilter::Role(r) => {
                self.primary.within(r) || self.secondary.iter().any(|s| s.within(r))
            }
            RoleFilter::Modifier(m) => self.modifiers.contains(&m),
        })
    }
}
