use std::time::Duration;

use loker_core::api::{CONTRACT_VERSION, RoleEnrichReport};
use loker_core::classify::{self, RoleAnswer};
use loker_core::store::Store;
use serde_json::{Value, json};

use crate::HttpClient;

const PARALLEL: usize = 8;
const TIMEOUT: Duration = Duration::from_secs(60);

pub async fn enrich_roles(
    store: &Store,
    client: &HttpClient,
    model: &str,
    key: &str,
    limit: usize,
) -> anyhow::Result<RoleEnrichReport> {
    let todo = store.role_questions(model, limit).await?;
    let question = classify::question();
    let mut report = RoleEnrichReport {
        contract_version: CONTRACT_VERSION,
        model: model.into(),
        asked: todo.len() as u32,
        ..Default::default()
    };
    let mut answers = Vec::new();
    for chunk in todo.chunks(PARALLEL) {
        let mut set = tokio::task::JoinSet::new();
        for (cache_key, state) in chunk.iter().cloned() {
            let (client, model, key, question) = (
                client.clone(),
                model.to_string(),
                key.to_string(),
                question.clone(),
            );
            set.spawn(async move {
                let r = ask(&client, &model, &key, &state, &question).await;
                (cache_key, r)
            });
        }
        while let Some(done) = set.join_next().await {
            match done? {
                (cache_key, Ok((answer, cost))) => {
                    report.answered += 1;
                    report.cost_usd += cost;
                    answers.push((cache_key, answer));
                }
                (_, Err(e)) => {
                    report.failed += 1;
                    if report.errors.len() < 5 {
                        report.errors.push(format!("{e:#}"));
                    }
                }
            }
        }
    }
    report.reclassified = store.put_role_answers(&answers).await?;
    Ok(report)
}

async fn ask(
    client: &HttpClient,
    model: &str,
    key: &str,
    state: &Value,
    question: &Value,
) -> anyhow::Result<(RoleAnswer, f64)> {
    let url = crate::url("https://openrouter.ai", "/api/alpha/decisions");
    let body = json!({ "model": model, "state": state, "questions": question });
    let mut attempt = 1;
    let resp: Value = loop {
        let sent = client
            .post(&url)
            .bearer_auth(key)
            .json(&body)
            .timeout(TIMEOUT)
            .send()
            .await;
        let retry = match sent {
            Ok(r) if r.status().is_success() => break r.json().await?,
            Ok(r) if r.status().as_u16() == 429 || r.status().is_server_error() => {
                format!("HTTP {}", r.status())
            }
            Ok(r) => {
                let status = r.status();
                let text = r.text().await.unwrap_or_default();
                anyhow::bail!(
                    "HTTP {status}: {}",
                    text.chars().take(200).collect::<String>()
                );
            }
            Err(e) if crate::transient(&e) => e.to_string(),
            Err(e) => return Err(e.into()),
        };
        if attempt == 3 {
            anyhow::bail!("{retry} after {attempt} attempts");
        }
        tokio::time::sleep(Duration::from_secs(2u64.pow(attempt))).await;
        attempt += 1;
    };
    let a = &resp["answers"]["role"];
    let choice = a["choice"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("no choice in the answer"))?
        .to_string();
    let mut top: Vec<(String, f64)> = a["probabilities"]
        .as_object()
        .map(|p| {
            p.iter()
                .filter_map(|(k, v)| v.as_f64().map(|v| (k.clone(), v)))
                .collect()
        })
        .unwrap_or_default();
    top.sort_by(|x, y| y.1.total_cmp(&x.1));
    top.truncate(3);
    let confidence = top
        .iter()
        .find(|(k, _)| *k == choice)
        .map(|(_, p)| *p)
        .unwrap_or(0.0);
    let cost = resp["usage"]["cost"].as_f64().unwrap_or(0.0);
    Ok((
        RoleAnswer {
            // The model asked for, not the one reported: the cache is keyed
            // by what config names.
            model: model.to_string(),
            choice,
            confidence,
            top,
        },
        cost,
    ))
}
