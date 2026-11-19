//! What a job is: a fixed hierarchy of functions, plus modifiers that cut
//! across it. Like countries and regions, every source's words map onto this
//! one vocabulary, at whatever level the source is specific to, and a filter
//! may name any level.
//!
//! ```text
//! engineering ─ software (specialty not stated), backend, frontend, fullstack,
//!               mobile, devops, data, ml_ai, security, qa, research,
//!               design_engineer
//! design ────── product_design, design_engineer
//! go_to_market  sales, marketing, solutions, devrel, technical_writing
//! product, management, support, operations, ai_training (no children)
//!
//! modifiers: forward_deployed, founding
//! ```
//!
//! A job stores its most specific role; [`Role::within`] answers whether it
//! falls under a filter's role, so `engineering` finds every engineering job.
//! `software` is engineering whose specialty the job doesn't state: a
//! `backend` filter doesn't take it in. A modifier is how the job is done,
//! not what it is: a forward-deployed infrastructure engineer is `devops`
//! with the `forward_deployed` modifier.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A node in the role hierarchy: a family (`engineering`) or a role under one
/// (`backend`).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Engineering,
    /// Engineering whose specialty the job doesn't state ("Software
    /// Engineer", "Member of Technical Staff").
    Software,
    Backend,
    Frontend,
    Fullstack,
    Mobile,
    /// DevOps, SRE, platform and infrastructure.
    Devops,
    /// Data engineering and analytics engineering.
    Data,
    /// Machine learning, and AI/LLM/agent engineering.
    MlAi,
    Security,
    Qa,
    /// Research engineers and scientists.
    Research,
    /// Builds the interface it designs: under engineering and design.
    DesignEngineer,
    Design,
    /// Product, UX and UI design.
    ProductDesign,
    /// Product managers.
    Product,
    /// Engineering managers and heads of engineering.
    Management,
    GoToMarket,
    Sales,
    Marketing,
    /// Sales and solutions engineers, pre-sales.
    Solutions,
    Devrel,
    TechnicalWriting,
    /// Customer support and success.
    Support,
    /// Operations, finance, legal, people.
    Operations,
    /// Data labelling and expert-review gigs that train AI models.
    AiTraining,
}

impl Role {
    /// Every role, families first.
    pub const ALL: &[Role] = &[
        Role::Engineering,
        Role::Design,
        Role::GoToMarket,
        Role::Product,
        Role::Management,
        Role::Support,
        Role::Operations,
        Role::AiTraining,
        Role::Software,
        Role::Backend,
        Role::Frontend,
        Role::Fullstack,
        Role::Mobile,
        Role::Devops,
        Role::Data,
        Role::MlAi,
        Role::Security,
        Role::Qa,
        Role::Research,
        Role::DesignEngineer,
        Role::ProductDesign,
        Role::Sales,
        Role::Marketing,
        Role::Solutions,
        Role::Devrel,
        Role::TechnicalWriting,
    ];

    /// The families this role sits under; empty for a family.
    pub fn parents(self) -> &'static [Role] {
        use Role::*;
        match self {
            Software | Backend | Frontend | Fullstack | Mobile | Devops | Data | MlAi
            | Security | Qa | Research => &[Engineering],
            DesignEngineer => &[Engineering, Design],
            ProductDesign => &[Design],
            Sales | Marketing | Solutions | Devrel | TechnicalWriting => &[GoToMarket],
            Engineering | Design | GoToMarket | Product | Management | Support | Operations
            | AiTraining => &[],
        }
    }

    /// Whether this role is `filter` or falls under it.
    pub fn within(self, filter: Role) -> bool {
        self == filter || self.parents().iter().any(|p| p.within(filter))
    }

    /// Whether `other` sits below this role, so it says more.
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
    /// Works on site with customers, building for their problems.
    ForwardDeployed,
    /// Among a company's first hires.
    Founding,
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

/// A job's role, and why it has it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobRole {
    /// The most specific role found.
    pub primary: Role,
    /// Other roles the job also fills ("Backend / Infrastructure Engineer").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secondary: Vec<Role>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modifiers: Vec<Modifier>,
    pub method: RoleMethod,
    pub confidence: f64,
    /// What decided it, e.g. "title: Forward Deployed Infrastructure Engineer".
    pub evidence: Option<String>,
}

/// One entry of a role filter: a role at any level, or a modifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum RoleFilter {
    Role(Role),
    Modifier(Modifier),
}

impl JobRole {
    /// Whether any of `filters` takes this job in.
    pub fn matches(&self, filters: &[RoleFilter]) -> bool {
        filters.iter().any(|f| match *f {
            RoleFilter::Role(r) => {
                self.primary.within(r) || self.secondary.iter().any(|s| s.within(r))
            }
            RoleFilter::Modifier(m) => self.modifiers.contains(&m),
        })
    }
}
