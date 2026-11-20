//! Visa sponsorship read from descriptions. A wrong "not offered" turns a job
//! away, so the rows that say nothing about sponsorship matter as much.
use loker_core::model::Sponsorship::{self, *};
use loker_core::visa_text::visa_from_text;

#[test]
fn sponsorship_read_from_descriptions() {
    let cases: [(&str, Option<Sponsorship>); 25] = [
        ("Visa sponsorship is available.", Some(Offered)),
        (
            "We'll fly you out, cover your visa sponsorship, and bring you in.",
            Some(Offered),
        ),
        (
            "Health insurance, visa sponsorship, optional: a hackerhouse.",
            Some(Offered),
        ),
        ("Visa sponsorship is **not available**", Some(NotOffered)),
        // An initialism doesn't end the sentence.
        (
            "Must be authorized to work in the U.S. without visa sponsorship.",
            Some(NotOffered),
        ),
        (
            "This job is not eligible for employment sponsorship.",
            Some(NotOffered),
        ),
        (
            "Revlon is unable to sponsor or transfer employment visas for this role.",
            Some(NotOffered),
        ),
        // A heading with its answer on the next line.
        (
            "### VISA Sponsorship:\n\nNo\n\n### Travel:",
            Some(NotOffered),
        ),
        ("Visa sponsorship: Yes", Some(Offered)),
        (
            "A relocation package with visa support for those who need it.",
            Some(Offered),
        ),
        ("Visa sponsorship for you and your family", Some(Offered)),
        ("Visa \\& relocation will be provided.", Some(Offered)),
        (
            "you'll need the right to work in the UK, as we're unable to provide sponsorship at this time.",
            Some(NotOffered),
        ),
        // Hedged, transfers only, or different by place.
        (
            "You must be work authorized in the United States without the need for new visa sponsorship. The company can support visa transfers but will not sponsor individuals for H-1B CAP applications.",
            Some(Limited),
        ),
        ("We can sponsor visas for the UK (not Spain)", Some(Limited)),
        (
            "the Company may not be able to support future H-1B sponsorship at this time",
            Some(Limited),
        ),
        (
            "Visa support is available for candidates with an H-1B visa eligible for transfer.",
            Some(Limited),
        ),
        (
            "No visa sponsorship for candidates outside the US.\nVisa sponsorship available for eligible candidates",
            Some(Limited),
        ),
        // Says nothing about sponsorship.
        (
            "Candidates must be legally authorized to work in the United States.",
            None,
        ),
        ("without regard to citizenship or immigration status", None),
        ("Visa sponsorship information was not provided", None),
        (
            "company-sponsored team events and employer-sponsored health insurance",
            None,
        ),
        (
            "Will you now or in the future require visa sponsorship?",
            None,
        ),
        (
            "Manage immigration and visa sponsorship processes for new hires.",
            None,
        ),
        (
            "You have hands-on experience supporting immigration and employee relocation programs.",
            None,
        ),
    ];
    for (text, want) in cases {
        let got = visa_from_text(text).map(|v| v.sponsorship);
        assert_eq!(got, want, "{text}");
    }
}
