use std::io::Read;
use std::time::Duration;

use anyhow::{Context, anyhow};
use flate2::read::MultiGzDecoder;
use loker_core::board::Board;
use serde_json::Value;

use crate::discover::Candidates;
use crate::{HttpClient, get_json, segment, transient, url};

const INDEX: &str = "https://index.commoncrawl.org";
const DATA: &str = "https://data.commoncrawl.org";
const CLUSTER_TIMEOUT: Duration = Duration::from_secs(900);
const BLOCK_PAUSE: Duration = Duration::from_millis(200);

pub const HOSTS: [&str; 7] = [
    "co,lever,eu,jobs)/",
    "co,lever,jobs)/",
    "com,ashbyhq,jobs)/",
    "io,greenhouse,boards)/",
    "io,greenhouse,eu,boards)/",
    "io,greenhouse,eu,job-boards)/",
    "io,greenhouse,job-boards)/",
];

pub fn valid_crawl(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

pub async fn latest_crawl(client: &HttpClient) -> anyhow::Result<String> {
    let crawls = get_json(client, &url(INDEX, "/collinfo.json")).await?;
    crawls
        .as_array()
        .and_then(|all| all.first())
        .and_then(|c| c.get("id"))
        .and_then(Value::as_str)
        .map(String::from)
        .ok_or_else(|| anyhow!("collinfo.json lists no crawl"))
}

pub async fn candidates(client: &HttpClient, crawl: &str) -> anyhow::Result<Candidates> {
    let indexes = format!("/cc-index/collections/{}/indexes", segment(crawl));
    let blocks = pick_blocks(client, &url(DATA, &format!("{indexes}/cluster.idx"))).await?;
    let mut found = Vec::new();
    for (i, block) in blocks.iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(BLOCK_PAUSE).await;
        }
        let file = url(DATA, &format!("{indexes}/{}", segment(&block.file)));
        let gz = get_range(client, &file, block.offset, block.length).await?;
        let mut text = String::new();
        MultiGzDecoder::new(gz.as_slice())
            .read_to_string(&mut text)
            .with_context(|| format!("{}: not a gzip block", block.file))?;
        found.extend(cdx_boards(&text));
    }
    Ok(Candidates::collect(format!("commoncrawl:{crawl}"), found))
}

async fn pick_blocks(client: &HttpClient, cluster: &str) -> anyhow::Result<Vec<Block>> {
    let mut resp = client
        .get(cluster)
        .timeout(CLUSTER_TIMEOUT)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .with_context(|| format!("GET {cluster}"))?;
    let mut picker = BlockPicker::default();
    let mut pending: Vec<u8> = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .with_context(|| format!("GET {cluster}"))?
    {
        pending.extend_from_slice(&chunk);
        if let Some(last) = pending.iter().rposition(|&b| b == b'\n') {
            let rest = pending.split_off(last + 1);
            String::from_utf8_lossy(&pending)
                .lines()
                .for_each(|l| picker.push(l));
            pending = rest;
        }
    }
    String::from_utf8_lossy(&pending)
        .lines()
        .for_each(|l| picker.push(l));
    Ok(picker.finish())
}

async fn get_range(
    client: &HttpClient,
    url: &str,
    offset: u64,
    length: u64,
) -> anyhow::Result<Vec<u8>> {
    const ATTEMPTS: u32 = 3;
    let range = format!("bytes={offset}-{}", offset + length.saturating_sub(1));
    let mut attempt = 1;
    loop {
        let result = async {
            let resp = client
                .get(url)
                .header(reqwest::header::RANGE, &range)
                .send()
                .await?
                .error_for_status()?;
            resp.bytes().await
        }
        .await;
        match result {
            Ok(bytes) => return Ok(bytes.to_vec()),
            Err(e)
                if attempt < ATTEMPTS
                    && (transient(&e) || e.status().is_some_and(|s| s.is_server_error())) =>
            {
                tracing::warn!(%url, attempt, "retrying a Common Crawl block: {e}");
                tokio::time::sleep(Duration::from_secs(2u64.pow(attempt))).await;
                attempt += 1;
            }
            Err(e) => return Err(e).with_context(|| format!("GET {url} ({range})")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub file: String,
    pub offset: u64,
    pub length: u64,
}

fn cluster_line(line: &str) -> Option<(&str, Block)> {
    let mut fields = line.split('\t');
    let key = fields.next()?.split(' ').next()?;
    let file = fields.next()?.to_string();
    let offset = fields.next()?.parse().ok()?;
    let length = fields.next()?.parse().ok()?;
    Some((
        key,
        Block {
            file,
            offset,
            length,
        },
    ))
}

#[derive(Default)]
pub struct BlockPicker {
    last: Option<(String, Block, bool)>,
    picked: Vec<Block>,
}

impl BlockPicker {
    pub fn push(&mut self, line: &str) {
        let Some((key, block)) = cluster_line(line) else {
            return;
        };
        if let Some((last_key, last_block, taken)) = &mut self.last
            && !*taken
            && HOSTS.iter().any(|h| last_key.as_str() < *h && key >= *h)
        {
            self.picked.push(last_block.clone());
            *taken = true;
        }
        let take = HOSTS.iter().any(|h| key.starts_with(h));
        if take {
            self.picked.push(block.clone());
        }
        self.last = Some((key.to_string(), block, take));
    }

    pub fn finish(mut self) -> Vec<Block> {
        if let Some((last_key, last_block, false)) = self.last
            && HOSTS.iter().any(|h| last_key.as_str() < *h)
        {
            self.picked.push(last_block);
        }
        self.picked
    }
}

pub fn cdx_boards(text: &str) -> impl Iterator<Item = Option<Board>> + '_ {
    text.lines()
        .filter(|line| HOSTS.iter().any(|h| line.starts_with(h)))
        .map(|line| {
            let json = &line[line.find('{')?..];
            let record: Value = serde_json::from_str(json).ok()?;
            Board::from_url(record.get("url")?.as_str()?)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picked(lines: &[&str]) -> Vec<String> {
        let mut picker = BlockPicker::default();
        for (i, key) in lines.iter().enumerate() {
            picker.push(&format!(
                "{key} 20261001\tcdx-{i:05}.gz\t{}\t10\t{i}",
                i * 10
            ));
        }
        picker.finish().into_iter().map(|b| b.file).collect()
    }

    #[test]
    fn blocks_that_may_hold_a_board_host_are_read() {
        let cases: &[(&[&str], &[&str])] = &[
            (
                &[
                    "co,example)/",
                    "com,ashbyhq,jobs)/a",
                    "com,ashbyhq,jobs)/m",
                    "org,example)/",
                ],
                &["cdx-00000.gz", "cdx-00001.gz", "cdx-00002.gz"],
            ),
            (&["co,example)/", "org,example)/"], &["cdx-00000.gz"]),
            (
                &["io,greenhouse,job-boards)/z", "org,example)/"],
                &["cdx-00000.gz"],
            ),
            (&["net,example)/", "org,example)/"], &[]),
            (&["io,a)/"], &["cdx-00000.gz"]),
            (&["org,example)/"], &[]),
        ];
        for (lines, want) in cases {
            assert_eq!(picked(lines), *want, "{lines:?}");
        }
    }

    #[test]
    fn a_cdx_line_names_its_board_by_the_original_url() {
        let text = "co,example)/ 2026 {\"url\": \"https://example.co/\"}\n\
            co,lever,jobs)/pigment/1 2026 {\"url\": \"https://jobs.lever.co/Pigment/1\", \"status\": \"200\"}\n\
            io,greenhouse,boards)/robots.txt 2026 {\"url\": \"https://boards.greenhouse.io/robots.txt\"}\n";
        let found: Vec<Option<String>> =
            cdx_boards(text).map(|b| b.map(|b| b.to_string())).collect();
        assert_eq!(found, [Some("lever:Pigment".to_string()), None]);
    }
}
