//! The role vocabulary: its shape, its names, and how filters take jobs in.
use std::collections::BTreeSet;

use loker_core::role::{JobRole, Modifier, Role, RoleFilter, RoleMethod};

#[test]
fn the_hierarchy_is_two_levels_and_complete() {
    let all: BTreeSet<Role> = Role::ALL.iter().copied().collect();
    assert_eq!(all.len(), Role::ALL.len(), "a role listed twice");
    for r in Role::ALL {
        for p in r.parents() {
            assert!(
                p.parents().is_empty(),
                "{r:?} sits under {p:?}, which is not a family"
            );
            assert!(all.contains(p));
        }
    }
    // Role and modifier names never collide, so a filter entry is unambiguous.
    let name = |v: serde_json::Value| v.as_str().unwrap().to_string();
    let roles: BTreeSet<String> = Role::ALL
        .iter()
        .map(|r| name(serde_json::to_value(r).unwrap()))
        .collect();
    for m in [Modifier::ForwardDeployed, Modifier::Founding] {
        assert!(!roles.contains(&name(serde_json::to_value(m).unwrap())));
    }
}

#[test]
fn within() {
    use Role::*;
    let cases = [
        (Backend, Backend, true),
        (Backend, Engineering, true),
        (Software, Engineering, true),
        // Software is engineering with no stated specialty, not every specialty.
        (Software, Backend, false),
        (Backend, Software, false),
        (DesignEngineer, Engineering, true),
        (DesignEngineer, Design, true),
        (ProductDesign, Engineering, false),
        (Solutions, GoToMarket, true),
        (Solutions, Engineering, false),
        (Engineering, Backend, false),
        (AiTraining, Engineering, false),
    ];
    for (role, filter, want) in cases {
        assert_eq!(role.within(filter), want, "{role:?} within {filter:?}");
    }
    assert!(Engineering.is_broader_than(Backend));
    assert!(!Backend.is_broader_than(Backend));
}

#[test]
fn filters() {
    let fde_infra = JobRole {
        primary: Role::Devops,
        secondary: vec![Role::Backend],
        modifiers: vec![Modifier::ForwardDeployed],
        method: RoleMethod::Title,
        confidence: 0.9,
        evidence: Some("title: Forward Deployed Infrastructure Engineer".into()),
    };
    let parse = |names: &[&str]| -> Vec<RoleFilter> {
        names
            .iter()
            .map(|n| serde_json::from_value(serde_json::json!(n)).unwrap())
            .collect()
    };
    let cases: &[(&[&str], bool)] = &[
        (&["devops"], true),
        (&["engineering"], true),
        (&["backend"], true), // its secondary role
        (&["forward_deployed"], true),
        (&["founding"], false),
        (&["design", "sales"], false),
        (&["sales", "devops"], true),
    ];
    for (names, want) in cases {
        assert_eq!(fde_infra.matches(&parse(names)), *want, "{names:?}");
    }
    assert!(serde_json::from_value::<RoleFilter>(serde_json::json!("no_such_role")).is_err());
}
