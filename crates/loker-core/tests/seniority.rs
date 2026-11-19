//! The seniority rule: title first, then the source's stated level.
use loker_core::model::Seniority::{self, *};
use loker_core::normalize::seniority;

#[test]
fn seniority_rules() {
    let cases: &[(&str, Option<&str>, Option<Seniority>)] = &[
        ("Senior Software Engineer", None, Some(Senior)),
        ("Sr. Backend Developer", None, Some(Senior)),
        ("Software Engineer, New Grad", None, Some(Junior)),
        ("Jr Frontend Developer", None, Some(Junior)),
        ("Software Engineer, Internship", None, Some(Intern)),
        ("Staff Engineer - Agentic AI", None, Some(Lead)),
        ("Senior/Staff Software Engineer", None, Some(Lead)),
        ("Head of Engineering", None, Some(Executive)),
        ("AI Engineer (Mid-Level)", None, Some(Mid)),
        // The title is more precise than the source's level.
        (
            "Staff Engineer - Backend/Kotlin",
            Some("senior"),
            Some(Lead),
        ),
        (
            "Full Stack Developer Intern",
            Some("entry-level"),
            Some(Intern),
        ),
        // No level word in the title: the source's level.
        ("Backend Engineer", Some("Mid-level"), Some(Mid)),
        ("Data Engineer", Some("entry-level, junior"), Some(Junior)),
        ("Operations", Some("Executive"), Some(Executive)),
        // Level words that aren't levels.
        ("Member of Technical Staff", None, None),
        (
            "B2B Lead Generation Specialist",
            Some("mid-level"),
            Some(Mid),
        ),
        ("Account Executive", None, None),
        ("Staff Accountant", None, None),
        ("Leading Backend Engineer", None, None),
        ("Backend Engineer", None, None),
    ];
    for (title, stated, want) in cases {
        assert_eq!(seniority(*stated, title), *want, "{title} / {stated:?}");
    }
}
