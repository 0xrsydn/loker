use std::sync::LazyLock;

use crate::model::{Experience, NewJob, Seniority, Visa};
use crate::posting::Posting;

#[derive(Debug, Clone, PartialEq)]
pub struct Derived {
    pub seniority: Option<Seniority>,
    pub visa: Option<Visa>,
    pub experience: Option<Experience>,
}

pub fn derive(job: &NewJob) -> Derived {
    let posting = (job.visa.is_none() || job.experience.is_none())
        .then(|| job.description_md.as_deref().map(Posting::parse))
        .flatten();
    Derived {
        seniority: crate::normalize::seniority(job.seniority.as_deref(), &job.title),
        visa: job
            .visa
            .clone()
            .or_else(|| posting.as_ref().and_then(crate::visa_text::read)),
        experience: job
            .experience
            .clone()
            .or_else(|| posting.as_ref().and_then(crate::experience_text::read)),
    }
}

const RULE_SOURCES: [&str; 5] = [
    include_str!("derived.rs"),
    include_str!("posting.rs"),
    include_str!("visa_text.rs"),
    include_str!("experience_text.rs"),
    include_str!("normalize.rs"),
];

pub static STAMP: LazyLock<i64> = LazyLock::new(|| {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for src in RULE_SOURCES {
        for b in src.bytes() {
            h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
    }
    (h >> 1) as i64
});
