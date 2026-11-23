use std::fmt;

use percent_encoding::percent_decode_str;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use url::Url;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Ats {
    Ashby,
    Greenhouse,
    Lever,
}

impl Ats {
    pub const ALL: [Ats; 3] = [Ats::Ashby, Ats::Greenhouse, Ats::Lever];

    pub fn as_str(self) -> &'static str {
        match self {
            Ats::Ashby => "ashby",
            Ats::Greenhouse => "greenhouse",
            Ats::Lever => "lever",
        }
    }

    pub fn parse(s: &str) -> Option<Ats> {
        Ats::ALL.into_iter().find(|a| a.as_str() == s)
    }

    pub fn slug_ignores_case(self) -> bool {
        !matches!(self, Ats::Lever)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum BoardStatus {
    Unchecked,
    Live,
    Empty,
    Missing,
    Private,
}

impl BoardStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            BoardStatus::Unchecked => "unchecked",
            BoardStatus::Live => "live",
            BoardStatus::Empty => "empty",
            BoardStatus::Missing => "missing",
            BoardStatus::Private => "private",
        }
    }

    pub fn parse(s: &str) -> Option<BoardStatus> {
        [
            BoardStatus::Unchecked,
            BoardStatus::Live,
            BoardStatus::Empty,
            BoardStatus::Missing,
            BoardStatus::Private,
        ]
        .into_iter()
        .find(|b| b.as_str() == s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardCheck {
    pub slug: String,
    pub status: BoardStatus,
    pub jobs: Option<u32>,
    pub company: Option<Company>,
}

impl BoardCheck {
    pub fn answered(
        slug: String,
        open: bool,
        jobs: Option<u32>,
        company: Option<Company>,
    ) -> BoardCheck {
        let status = match &company {
            _ if !open => BoardStatus::Empty,
            Some(c) if c.names_a_private_board() => BoardStatus::Private,
            _ => BoardStatus::Live,
        };
        BoardCheck {
            slug,
            status,
            jobs,
            company,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Company {
    pub name: String,
    pub website: Option<String>,
}

const PRIVATE_BOARD_PHRASES: &[&str] = &[
    "internal job",
    "internal jobs",
    "internal board",
    "internal employees",
    "internal vacancies",
    "internal mobility",
    "internal candidates",
    "internal only",
    "internal postings",
    "referrals only",
    "referral only",
    "referral board",
    "referral job board",
    "employee referral",
    "alumni network",
    "alumni job board",
    "talent community",
    "talent network",
    "talent pool",
    "invitation only",
    "linkedin",
    "third party job",
    "job links only",
    "integration sandbox",
    "test board",
    "do not use",
];

impl Company {
    pub fn names_a_private_board(&self) -> bool {
        let words: Vec<String> = self
            .name
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(String::from)
            .collect();
        let text = format!(" {} ", words.join(" "));
        text != " linkedin "
            && (text.ends_with(" internal ")
                || PRIVATE_BOARD_PHRASES
                    .iter()
                    .any(|p| text.contains(&format!(" {p} "))))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Board {
    pub ats: Ats,
    pub slug: String,
}

impl fmt::Display for Board {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.ats.as_str(), self.slug)
    }
}

pub fn valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.trim() == slug
        && !slug
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '?' | '#' | '%'))
}

impl Board {
    pub fn new(ats: Ats, slug: &str) -> Option<Board> {
        valid_slug(slug).then(|| Board {
            ats,
            slug: slug.to_string(),
        })
    }

    pub fn from_id(id: &str) -> Option<Board> {
        let (ats, slug) = id.split_once(':')?;
        Board::new(Ats::parse(ats)?, slug)
    }

    pub fn key(&self) -> String {
        let slug = if self.ats.slug_ignores_case() {
            self.slug.to_lowercase()
        } else {
            self.slug.clone()
        };
        format!("{}:{slug}", self.ats.as_str())
    }

    pub fn from_url(raw: &str) -> Option<Board> {
        let raw = raw.trim();
        let url = Url::parse(raw)
            .ok()
            .filter(|u| u.has_host())
            .or_else(|| Url::parse(&format!("https://{raw}")).ok())?;
        if !matches!(url.scheme(), "http" | "https") {
            return None;
        }
        let host = url.host_str()?.trim_start_matches("www.");
        let segments: Vec<String> = url
            .path_segments()?
            .filter(|s| !s.is_empty())
            .map(|s| percent_decode_str(s).decode_utf8_lossy().into_owned())
            .collect();
        let segment = |i: usize| segments.get(i).map(String::as_str);
        let query = |name: &str| {
            url.query_pairs()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.into_owned())
        };
        let (ats, slug) = match host {
            "jobs.ashbyhq.com" => (
                Ats::Ashby,
                page_slug(segment(0)?, &["api", "b", "meeting"])?,
            ),
            "api.ashbyhq.com" => match (segment(0)?, segment(1)?) {
                ("posting-api", "job-board") => (Ats::Ashby, segment(2)?.to_string()),
                _ => return None,
            },
            "boards.greenhouse.io"
            | "job-boards.greenhouse.io"
            | "boards.eu.greenhouse.io"
            | "job-boards.eu.greenhouse.io" => match segment(0)? {
                "embed" => (Ats::Greenhouse, query("for")?),
                first => (Ats::Greenhouse, page_slug(first, &["api", "v1"])?),
            },
            "boards-api.greenhouse.io" | "boards-api.eu.greenhouse.io" | "api.greenhouse.io" => {
                match (segment(0)?, segment(1)?) {
                    ("v1", "boards") => (Ats::Greenhouse, segment(2)?.to_string()),
                    _ => return None,
                }
            }
            "jobs.lever.co" | "jobs.eu.lever.co" => (Ats::Lever, page_slug(segment(0)?, &[])?),
            "api.lever.co" | "api.eu.lever.co" => match (segment(0)?, segment(1)?) {
                ("v0", "postings") => (Ats::Lever, segment(2)?.to_string()),
                _ => return None,
            },
            _ => return None,
        };
        Board::new(ats, &slug)
    }
}

const FILE_EXTENSIONS: [&str; 9] = [
    ".txt", ".xml", ".ico", ".js", ".css", ".png", ".svg", ".json", ".html",
];

fn page_slug(first: &str, reserved: &[&str]) -> Option<String> {
    let lower = first.to_lowercase();
    let not_a_board = reserved.contains(&lower.as_str())
        || lower.starts_with('_')
        || FILE_EXTENSIONS.iter().any(|e| lower.ends_with(e));
    (!not_a_board).then(|| first.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn board_urls_name_one_board() {
        let cases: &[(&str, Option<&str>)] = &[
            (
                "https://jobs.ashbyhq.com/Hippocratic%20AI/c8e7cb17-1934-499d-8f0b-7a5976a8ffd2",
                Some("ashby:Hippocratic AI"),
            ),
            (
                "https://jobs.ashbyhq.com/Ampersand",
                Some("ashby:Ampersand"),
            ),
            (
                "https://jobs.ashbyhq.com/openai/3a9c/application?utm_source=x",
                Some("ashby:openai"),
            ),
            (
                "https://api.ashbyhq.com/posting-api/job-board/ramp?includeCompensation=true",
                Some("ashby:ramp"),
            ),
            ("https://jobs.ashbyhq.com/api/non-user-graphql", None),
            ("https://jobs.ashbyhq.com/robots.txt", None),
            ("https://jobs.ashbyhq.com/_next/static/x.js", None),
            ("https://jobs.ashbyhq.com/", None),
            (
                "http://boards.greenhouse.io/stripe",
                Some("greenhouse:stripe"),
            ),
            (
                "https://job-boards.greenhouse.io/stripe/jobs/8172510?gh_src=abc",
                Some("greenhouse:stripe"),
            ),
            (
                "https://job-boards.eu.greenhouse.io/celonis/jobs/123",
                Some("greenhouse:celonis"),
            ),
            (
                "https://boards.greenhouse.io/embed/job_app?for=figma&token=55",
                Some("greenhouse:figma"),
            ),
            (
                "https://boards.greenhouse.io/embed/job_board?for=Asana",
                Some("greenhouse:Asana"),
            ),
            ("https://boards.greenhouse.io/embed/job_app?token=55", None),
            (
                "https://boards-api.greenhouse.io/v1/boards/datadog/jobs?content=true",
                Some("greenhouse:datadog"),
            ),
            ("https://boards.greenhouse.io/sitemap.xml", None),
            (
                "https://jobs.lever.co/palantir/6ed76ce8-4156-4b60-b120-403538bd66cd/apply",
                Some("lever:palantir"),
            ),
            ("https://jobs.eu.lever.co/Pigment", Some("lever:Pigment")),
            (
                "https://api.lever.co/v0/postings/palantir?mode=json",
                Some("lever:palantir"),
            ),
            ("jobs.lever.co/apryse", Some("lever:apryse")),
            ("https://www.jobs.lever.co/apryse", Some("lever:apryse")),
            ("https://jobs.lever.co/a%2Fb/1", None),
            ("https://example.com/jobs.lever.co/apryse", None),
            ("https://notjobs.lever.co/apryse", None),
            ("ftp://jobs.lever.co/apryse", None),
            ("not a url", None),
        ];
        for (url, want) in cases {
            let got = Board::from_url(url).map(|b| b.to_string());
            assert_eq!(got.as_deref(), *want, "{url}");
        }
    }

    #[test]
    fn a_board_named_for_insiders_or_a_mirror_is_private() {
        let cases = [
            ("Referrals Only", true),
            ("Home Chef (Internal Employees)", true),
            ("Acorn Health - Internal", true),
            ("Jumia Internal Mobility Board", true),
            ("Stone - Linkedin", true),
            ("SingleStore-LinkedIn", true),
            ("Third-Party Job Posts", true),
            ("Scout Motors Talent Community", true),
            ("EVG Hidden (Job Links Only)", true),
            ("Businessolver - Invitation Only Job Board", true),
            ("LinkedIn", false),
            ("HiddenLayer", false),
            ("SandboxAQ", false),
            ("Alumni Ventures", false),
            ("Internal Medicine Partners", false),
            ("Human Agency", false),
        ];
        for (name, private) in cases {
            let company = Company {
                name: name.to_string(),
                website: None,
            };
            assert_eq!(company.names_a_private_board(), private, "{name}");
        }
    }

    #[test]
    fn keys_ignore_case_only_where_the_ats_does() {
        let cases = [
            ("ashby:Hippocratic AI", "ashby:hippocratic ai"),
            ("greenhouse:Stripe", "greenhouse:stripe"),
            ("lever:Palantir", "lever:Palantir"),
        ];
        for (id, key) in cases {
            assert_eq!(Board::from_id(id).unwrap().key(), key, "{id}");
        }
        assert_eq!(Board::from_id("workable:x"), None);
        assert_eq!(Board::from_id("ashby: padded"), None);
    }
}
