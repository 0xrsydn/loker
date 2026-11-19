//! The role classifier on real and tricky titles: one row per rule worth
//! pinning. Accuracy at large is measured by eval/role, not here.
mod common;

use common::job;
use loker_core::classify::classify;
use loker_core::model::Scope;
use loker_core::role::{Modifier, Role, Role::*, RoleMethod};

struct Case {
    title: &'static str,
    category: Option<&'static str>,
    description: Option<&'static str>,
    primary: Option<Role>,
    secondary: &'static [Role],
    modifiers: &'static [Modifier],
}

const fn t(title: &'static str, primary: Option<Role>) -> Case {
    Case {
        title,
        category: None,
        description: None,
        primary,
        secondary: &[],
        modifiers: &[],
    }
}

#[test]
fn roles_from_titles_categories_and_descriptions() {
    use Modifier::*;
    let cases = [
        // Specific titles.
        t("Senior Backend Engineer", Some(Backend)),
        t("Full-Stack Developer", Some(Fullstack)),
        t("Staff Site Reliability Engineer", Some(Infrastructure)),
        t("Platform Engineer (Kubernetes)", Some(Infrastructure)),
        t(
            "ML Inference Engineer (vLLM / TensorRT)",
            Some(MlInfrastructure),
        ),
        t(
            "Senior Machine Learning Engineer, Ranking",
            Some(MlEngineering),
        ),
        t("Software Engineer, Agents", Some(AiEngineering)),
        t("Research Engineer - Post-training", Some(Research)),
        t("Senior Data Scientist", Some(DataScience)),
        t("Analytics Engineer", Some(DataEngineering)),
        t("Product Data Analyst", Some(DataAnalysis)),
        t("Senior Product Manager, Growth", Some(ProductManagement)),
        t(
            "Technical Program Manager – Security",
            Some(ProgramManagement),
        ),
        t("Senior Product Designer", Some(ProductDesign)),
        t("Sr. UX Researcher", Some(UxResearch)),
        t("Brand Designer", Some(BrandDesign)),
        t("Product Marketing Manager", Some(ProductMarketing)),
        t("Enterprise Account Executive", Some(Sales)),
        t("Sales Solutions Engineer", Some(SalesEngineering)),
        t("Customer Success Manager", Some(CustomerSuccess)),
        t("Senior Support Engineer", Some(TechnicalSupport)),
        t(
            "Customer Service Representative (Spanish/English)",
            Some(CustomerSupport),
        ),
        t("Developer Advocate", Some(Devrel)),
        t("Technical Writer", Some(TechnicalWriting)),
        t("Senior QA Engineer (Rust/Java)", Some(Qa)),
        t("Firmware Engineer", Some(Embedded)),
        t("Principal Hardware Engineer", Some(Hardware)),
        t("Corporate Recruiter", Some(Recruiting)),
        t("HR Business Partner", Some(People)),
        t("KYB Compliance Analyst", Some(Compliance)),
        t("Commercial Legal Counsel", Some(Legal)),
        t("FP&A Manager", Some(Finance)),
        t("Executive Assistant", Some(Admin)),
        t("Chief of Staff", Some(BusinessOperations)),
        t("Online English Teacher", Some(Education)),
        t("Clinical Trial Manager", Some(Healthcare)),
        t(
            "Remote AI Training - $20/hr - United States",
            Some(AiTraining),
        ),
        t("Hardware Expert", Some(AiTraining)),
        // Product engineers also count as full stack.
        Case {
            secondary: &[Fullstack],
            modifiers: &[Founding],
            ..t("Founding Product Engineer", Some(ProductEngineer))
        },
        // Modifiers.
        Case {
            modifiers: &[ForwardDeployed],
            ..t("Forward Deployed AI Engineer", Some(AiEngineering))
        },
        Case {
            modifiers: &[ForwardDeployed],
            ..t(
                "Deployment Strategist - US Government",
                Some(SalesEngineering),
            )
        },
        Case {
            modifiers: &[Founding],
            ..t("Founding Design Engineer", Some(DesignEngineer))
        },
        // Two technical roles in one title.
        Case {
            secondary: &[Backend],
            ..t("Backend / Infrastructure Engineer", Some(Infrastructure))
        },
        // The team a manager runs is a second role.
        Case {
            secondary: &[AiEngineering],
            ..t(
                "Engineering Manager, Applied AI",
                Some(EngineeringManagement),
            )
        },
        // A domain word is not a second role.
        t("Data Engineer, Marketing Analytics", Some(DataEngineering)),
        // False friends.
        t("Member of Technical Staff", Some(Software)),
        t("Mechanical Design Engineer", Some(Hardware)),
        t("Production Engineer", Some(Infrastructure)),
        t("Product Security Engineer", Some(Security)),
        t("Civil Engineer", None),
        t("B2B Lead Generation Specialist", Some(Marketing)),
        t("Line Cook", None),
        // A vague title takes the category; an unknown category is ignored.
        Case {
            category: Some("DevOps and Sysadmin"),
            ..t("Software Engineer", Some(Infrastructure))
        },
        Case {
            category: Some("Delta"),
            modifiers: &[ForwardDeployed],
            ..t("Forward Deployed Software Engineer", Some(Software))
        },
        Case {
            category: Some("Data Science"),
            ..t("Associate", Some(Data))
        },
        // ...but never one broader than the title ("Engineer" stays software).
        Case {
            category: Some("Engineering; Core Engineering"),
            modifiers: &[Founding],
            ..t("Founding Engineer", Some(Software))
        },
        t("Market Surveillance Investigator", Some(Compliance)),
        // A category never overrides a specific title...
        Case {
            category: Some("Back-End Programming"),
            ..t("Senior Frontend Engineer", Some(Frontend))
        },
        // ...and an "engineer" title takes only an engineering category.
        Case {
            category: Some("Sales"),
            ..t("Software Engineer", Some(Software))
        },
        // The description names the role when nothing else does.
        Case {
            description: Some("We're hiring a data engineer to own our pipelines."),
            ..t("Member of Technical Staff", Some(DataEngineering))
        },
    ];
    let mut failures = Vec::new();
    for c in &cases {
        let mut j = job("wwr", "x", "Co", c.title, Scope::Worldwide, 0.9);
        j.category = c.category.map(str::to_string);
        j.description_md = c.description.map(str::to_string);
        let got = classify(&j);
        let ok = match (&got, c.primary) {
            (None, None) => true,
            (Some(r), Some(p)) => {
                r.primary == p && r.secondary == c.secondary && r.modifiers == c.modifiers
            }
            _ => false,
        };
        if !ok {
            failures.push(format!(
                "{:?} (category {:?}): want {:?} {:?} {:?}, got {:?}",
                c.title,
                c.category,
                c.primary,
                c.secondary,
                c.modifiers,
                got.map(|r| (r.primary, r.secondary, r.modifiers, r.evidence))
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn records_how_it_decided() {
    let mut j = job("wwr", "x", "Co", "Software Engineer", Scope::Worldwide, 0.9);
    j.category = Some("DevOps and Sysadmin".into());
    let r = classify(&j).unwrap();
    assert_eq!(r.method, RoleMethod::Category);
    assert_eq!(r.evidence.as_deref(), Some("category: DevOps and Sysadmin"));
    assert_eq!(r.version, loker_core::role::VERSION);
}

#[test]
fn a_model_answer_wins_unless_a_title_names_the_role_and_it_is_unsure() {
    use loker_core::classify::{RoleAnswer, classify_with};
    let answer = |choice: &str, confidence: f64| RoleAnswer {
        model: "test".into(),
        choice: choice.into(),
        confidence,
        top: vec![],
    };
    let with = |title: &str, category: Option<&str>, a: RoleAnswer| {
        let mut j = job("wwr", "x", "Co", title, Scope::Worldwide, 0.9);
        j.category = category.map(str::to_string);
        classify_with(&j, Some(&a)).map(|r| (r.primary, r.method, r.modifiers))
    };
    use Modifier::*;
    let cases = [
        // A vague title: the model decides, whatever its confidence.
        (
            with(
                "Senior Software Engineer",
                None,
                answer("infrastructure", 0.4),
            ),
            Some((Infrastructure, RoleMethod::Model, vec![])),
        ),
        // A category guess gives way to the model.
        (
            with(
                "Founding GTM Lead",
                Some("Engineering"),
                answer("sales", 0.8),
            ),
            Some((Sales, RoleMethod::Model, vec![Founding])),
        ),
        // A title that names the role wins when the model is unsure...
        (
            with(
                "Senior Backend Engineer",
                None,
                answer("infrastructure", 0.5),
            ),
            Some((Backend, RoleMethod::Title, vec![])),
        ),
        // ...and loses when it is sure.
        (
            with(
                "Deployment Strategist",
                None,
                answer("business_operations", 0.8),
            ),
            Some((BusinessOperations, RoleMethod::Model, vec![ForwardDeployed])),
        ),
        // "none" means no role.
        (with("Line Cook", None, answer("none", 0.9)), None),
    ];
    for (got, want) in cases {
        assert_eq!(got, want);
    }
}
