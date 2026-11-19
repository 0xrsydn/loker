//! `$LOKER_HOME/config.toml`: which sources to ingest, with their parameters.
//! Intent lives here; state lives in the database. Versioned like the contract,
//! and unknown keys are errors, so a typo never silently disables a source.
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde::Deserialize;

use crate::api::IngestParams;

pub const CONFIG_VERSION: u32 = 1;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    #[serde(default)]
    pub sources: Vec<SourceConfig>,
    /// Optional third-party services. None is needed: without them loker
    /// works fully offline.
    #[serde(default)]
    pub providers: Providers,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Providers {
    /// Jev (TypeSafe) through OpenRouter's Decisions API classifies job
    /// roles; the key comes from `OPENROUTER_API_KEY`, never this file.
    pub jev: Option<JevConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JevConfig {
    /// A dated model id: aliases move between releases.
    #[serde(default = "default_jev_model")]
    pub model: String,
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
}

pub const DEFAULT_JEV_MODEL: &str = "typesafe/jev-1.14-20261014";

fn default_jev_model() -> String {
    DEFAULT_JEV_MODEL.into()
}

/// One source instance. Field meanings match `IngestParams`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceConfig {
    /// `himalayas`, `jobicy`, `remoteok`, `weworkremotely`, `greenhouse:<board>`,
    /// `ashby:<board>`, `lever:<site>`.
    pub id: String,
    pub pages: Option<u32>,
    pub country: Option<String>,
    /// Company display name for an employer board (Ashby and Lever only give the slug).
    pub company: Option<String>,
    /// `true` when every job on an employer board is remote (see `IngestParams::remote`).
    pub remote: Option<bool>,
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
}

fn enabled_by_default() -> bool {
    true
}

impl SourceConfig {
    pub fn params(&self) -> IngestParams {
        IngestParams {
            source: self.id.clone(),
            pages: self.pages,
            country: self.country.clone(),
            company: self.company.clone(),
            remote: self.remote,
        }
    }
}

impl Config {
    /// Enabled sources, in file order.
    pub fn enabled(&self) -> impl Iterator<Item = &SourceConfig> {
        self.sources.iter().filter(|s| s.enabled)
    }
}

pub fn path(home: &Path) -> PathBuf {
    home.join("config.toml")
}

/// `None` when there is no config file yet.
pub fn load(home: &Path) -> anyhow::Result<Option<Config>> {
    let path = path(home);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let config: Config =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    if config.version != CONFIG_VERSION {
        bail!(
            "{}: version {} is not supported (this loker reads version {CONFIG_VERSION})",
            path.display(),
            config.version
        );
    }
    let mut seen = std::collections::BTreeSet::new();
    for s in &config.sources {
        if !seen.insert(s.id.as_str()) {
            bail!("{}: source `{}` is listed twice", path.display(), s.id);
        }
    }
    Ok(Some(config))
}

/// Written by `loker config init`.
pub const TEMPLATE: &str = r#"# loker configuration: https://github.com/0xrsydn/loker
version = 1

# Each [[sources]] entry is one source, fetched by `loker ingest --all` and
# checked by `loker verify`. Unknown keys are errors.

[[sources]]
id = "himalayas"
pages = 3              # 20 jobs per page
# country = "ID"       # only jobs open to this country (worldwide + listed)

[[sources]]
id = "jobicy"

[[sources]]
id = "remoteok"

[[sources]]
id = "weworkremotely"

# A company's own board: greenhouse:<slug>, ashby:<slug> or lever:<site>.
# [[sources]]
# id = "ashby:clera"
# company = "Clera"    # display name; Ashby and Lever only give the slug
# enabled = false      # keep it listed but skip it
#
# A remote-first company whose postings list where you can be based:
# [[sources]]
# id = "greenhouse:binance"
# remote = true        # every job is remote; its locations are where you can work from

# Optional: Jev (TypeSafe, through OpenRouter) classifies job roles the
# title rules can't place, about $0.0001 a job and cached per job. `loker
# ingest --all` runs it after the sources; `loker roles enrich` runs it
# alone. Needs OPENROUTER_API_KEY in the environment.
# [providers.jev]
# model = "typesafe/jev-1.14-20261014"
"#;
