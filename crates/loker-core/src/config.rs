use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde::Deserialize;

use crate::api::IngestParams;
use crate::board::Board;

pub const CONFIG_VERSION: u32 = 1;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    #[serde(default)]
    pub sources: Vec<SourceConfig>,
    #[serde(default)]
    pub providers: Providers,
    #[serde(default)]
    pub discovery: Discovery,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Discovery {
    #[serde(default)]
    pub ingest: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Providers {
    pub jev: Option<JevConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JevConfig {
    #[serde(default = "default_jev_model")]
    pub model: String,
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
}

pub const DEFAULT_JEV_MODEL: &str = "typesafe/jev-1.14-20261014";

fn default_jev_model() -> String {
    DEFAULT_JEV_MODEL.into()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceConfig {
    pub id: String,
    pub pages: Option<u32>,
    #[serde(default, deserialize_with = "one_or_many")]
    pub country: Vec<String>,
    pub company: Option<String>,
    pub remote: Option<bool>,
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
}

fn enabled_by_default() -> bool {
    true
}

impl SourceConfig {
    pub fn params(&self) -> Vec<IngestParams> {
        let one = |country: Option<String>| IngestParams {
            source: self.id.clone(),
            pages: self.pages,
            country,
            company: self.company.clone(),
            remote: self.remote,
        };
        if self.country.is_empty() {
            vec![one(None)]
        } else {
            self.country.iter().map(|c| one(Some(c.clone()))).collect()
        }
    }
}

fn one_or_many<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }
    Ok(match OneOrMany::deserialize(d)? {
        OneOrMany::One(s) => vec![s],
        OneOrMany::Many(v) => v,
    })
}

impl Config {
    pub fn enabled(&self) -> impl Iterator<Item = &SourceConfig> {
        self.sources.iter().filter(|s| s.enabled)
    }
}

pub fn path(home: &Path) -> PathBuf {
    home.join("config.toml")
}

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
        let key = Board::from_id(&s.id).map_or_else(|| s.id.clone(), |b| b.key());
        if !seen.insert(key) {
            bail!("{}: source `{}` is listed twice", path.display(), s.id);
        }
    }
    Ok(Some(config))
}

pub fn template(country: Option<&str>) -> String {
    let Some(c) = country else {
        return TEMPLATE.to_string();
    };
    TEMPLATE
        .replace(
            "# country = \"ID\"       # only jobs open to this country (worldwide + listed)",
            &format!("country = \"{c}\"         # only jobs open to this country (worldwide + listed)"),
        )
        .replace(
            "# [[sources]]\n# id = \"workable\"\n# pages = 10           # 20 jobs per page\n# country = \"PH\"       # only jobs Workable places in this country",
            &format!("[[sources]]\nid = \"workable\"\npages = 10             # 20 jobs per page\ncountry = \"{c}\"         # only jobs Workable places in this country"),
        )
}

#[derive(Debug, Clone)]
pub struct SourceStatus {
    pub id: String,
    pub enabled: bool,
    pub error: Option<String>,
}

impl Config {
    pub fn check(&self, known: &dyn Fn(&str) -> Result<(), String>) -> Vec<SourceStatus> {
        self.sources
            .iter()
            .map(|s| SourceStatus {
                id: s.id.clone(),
                enabled: s.enabled,
                error: known(&s.id).err(),
            })
            .collect()
    }
}

pub const STARTER_NOTICE: &str = "no config.toml: using the starter sources (Himalayas, Jobicy, Remote OK, We Work Remotely); `loker config init --country <ISO code>` writes one you can edit";

pub fn init(home: &Path, country: Option<&str>) -> anyhow::Result<PathBuf> {
    let path = path(home);
    if path.exists() {
        bail!("{} already exists", path.display());
    }
    let code = country
        .map(|c| {
            crate::country::to_alpha2(c)
                .ok_or_else(|| anyhow::anyhow!("unknown country `{c}` (use an ISO code, e.g. ID)"))
        })
        .transpose()?;
    std::fs::create_dir_all(home)?;
    std::fs::write(&path, template(code))?;
    Ok(path)
}

pub fn starter() -> Config {
    toml::from_str(TEMPLATE).expect("the template parses")
}

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

# Workable's job search across every company on Workable: remote jobs only.
# Not a published API (the one jobs.workable.com uses), so it is opt-in.
# [[sources]]
# id = "workable"
# pages = 10           # 20 jobs per page
# country = "PH"       # only jobs Workable places in this country; or a list: ["PH", "ID"]

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

# Boards `loker discover` found (Common Crawl, a list) and checked live:
# ingest them all too, except those listed above, enabled or not.
# [discovery]
# ingest = true

# Optional: Jev (TypeSafe, through OpenRouter) classifies job roles the
# title rules can't place, about $0.0001 a job and cached per job. `loker
# ingest --all` runs it after the sources; `loker roles enrich` runs it
# alone. Needs OPENROUTER_API_KEY in the environment.
# [providers.jev]
# model = "typesafe/jev-1.14-20261014"
"#;

#[cfg(test)]
mod tests {
    #[test]
    fn a_country_turns_on_its_searches() {
        let c: super::Config = toml::from_str(&super::template(Some("PH"))).unwrap();
        let get = |id: &str| c.sources.iter().find(|s| s.id == id).unwrap();
        assert_eq!(get("himalayas").country, ["PH"]);
        assert_eq!(get("workable").country, ["PH"]);
        assert_eq!(super::template(Some("PH")).matches("or a list").count(), 1);
        let many: super::Config = toml::from_str(
            "version = 1\n[[sources]]\nid = \"workable\"\ncountry = [\"PH\", \"ID\"]\n",
        )
        .unwrap();
        let fetches = many.sources[0].params();
        assert_eq!(
            fetches
                .iter()
                .map(|p| p.country.as_deref())
                .collect::<Vec<_>>(),
            [Some("PH"), Some("ID")]
        );
        assert_eq!(super::starter().sources.len(), 4);
    }

    #[test]
    fn a_board_is_listed_twice_when_its_ats_reads_both_ids_as_one() {
        let home = std::env::temp_dir().join(format!("loker-config-twice-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        let load = |a: &str, b: &str| {
            let text =
                format!("version = 1\n[[sources]]\nid = \"{a}\"\n[[sources]]\nid = \"{b}\"\n");
            std::fs::write(home.join("config.toml"), text).unwrap();
            super::load(&home)
        };
        assert!(load("greenhouse:Stripe", "greenhouse:stripe").is_err());
        assert!(load("ashby:Hippocratic AI", "ashby:hippocratic ai").is_err());
        assert!(load("lever:Palantir", "lever:palantir").is_ok());
        std::fs::remove_dir_all(&home).unwrap();
    }
}
