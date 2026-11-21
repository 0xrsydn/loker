use std::collections::BTreeSet;

use loker_core::role::{self, JobRole, Modifier, Role, RoleFilter, RoleMethod};

fn name<T: serde::Serialize>(v: T) -> String {
    serde_json::to_value(v)
        .unwrap()
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn the_vocabulary_is_well_formed() {
    let schema = serde_json::to_value(schemars::schema_for!(Role)).unwrap();
    // Plain variants come as `enum`; documented ones as `oneOf` of `const`s.
    let variants: BTreeSet<String> = match schema.get("enum") {
        Some(e) => e.as_array().unwrap().clone(),
        None => schema["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["const"].clone())
            .collect(),
    }
    .iter()
    .map(|v| v.as_str().unwrap().to_string())
    .collect();
    let listed: BTreeSet<String> = Role::ALL.iter().map(name).collect();
    assert_eq!(listed.len(), Role::ALL.len(), "an id listed twice");
    assert_eq!(listed, variants, "Role::ALL and the enum differ");

    for &r in Role::ALL {
        let i = r.info();
        assert!(!i.label.is_empty() && !i.definition.is_empty(), "{r:?}");
        if let Some(home) = i.home {
            assert!(home.is_family(), "{r:?}'s home {home:?} is not a family");
        }
        let mut depth = 0;
        let mut at = r;
        while let Some(h) = at.info().home {
            at = h;
            depth += 1;
            assert!(
                depth <= 2,
                "{r:?} nests too deep: families nest at most one deep"
            );
        }
        for p in i.also_in {
            assert_ne!(*p, r, "{r:?} is inside itself");
            assert!(!p.within(r), "{r:?} and {p:?} form a cycle");
        }
        if let Some(new) = i.replaced_by {
            assert!(
                new.info().replaced_by.is_none(),
                "{r:?} is replaced by a retired id"
            );
        }
    }

    for m in Modifier::ALL {
        assert!(
            !listed.contains(&name(m)),
            "{m:?} is also a role, so a filter entry is ambiguous"
        );
    }
}

#[test]
fn within() {
    use Role::*;
    let cases = [
        (Backend, Backend, true),
        (Backend, Engineering, true),
        (AiEngineering, Ai, true),
        (MlInfrastructure, Engineering, true),
        (Ai, Engineering, true),
        (Backend, Ai, false),
        (Software, Engineering, true),
        (Software, Backend, false),
        (Backend, Software, false),
        (ProductEngineer, Fullstack, false),
        (ProductEngineer, Product, false),
        (DataEngineering, Data, true),
        (DataEngineering, Engineering, true),
        (DataScience, Engineering, false),
        (DesignEngineer, Design, true),
        (Devrel, GoToMarket, true),
        (SalesEngineering, Engineering, true),
        (CustomerSuccess, Support, true),
        (ProgramManagement, Product, true),
        (ProductDesign, Product, false),
        (AiTraining, Ai, false),
        (AiTraining, Engineering, false),
        (Engineering, Backend, false),
    ];
    for (id, filter, want) in cases {
        assert_eq!(id.within(filter), want, "{id:?} within {filter:?}");
    }
    assert!(Engineering.is_broader_than(Backend));
    assert!(!Backend.is_broader_than(Backend));
    assert!(Ai.is_family() && Engineering.is_family() && Healthcare.is_family());
    assert!(!Backend.is_family());
}

#[test]
fn filters() {
    let fde_ai = JobRole {
        primary: Role::AiEngineering,
        secondary: vec![Role::Backend],
        modifiers: vec![Modifier::ForwardDeployed],
        method: RoleMethod::Title,
        confidence: 0.9,
        evidence: Some("title: Forward Deployed AI Engineer".into()),
        version: role::VERSION,
    };
    let parse = |names: &[&str]| -> Vec<RoleFilter> {
        names
            .iter()
            .map(|n| serde_json::from_value(serde_json::json!(n)).unwrap())
            .collect()
    };
    let cases: &[(&[&str], bool)] = &[
        (&["ai_engineering"], true),
        (&["ai"], true),
        (&["engineering"], true),
        (&["backend"], true),
        (&["forward_deployed"], true),
        (&["founding"], false),
        (&["ml_engineering"], false),
        (&["design", "sales"], false),
        (&["sales", "ai"], true),
    ];
    for (names, want) in cases {
        assert_eq!(fde_ai.matches(&parse(names)), *want, "{names:?}");
    }
    assert!(serde_json::from_value::<RoleFilter>(serde_json::json!("no_such_role")).is_err());
}
