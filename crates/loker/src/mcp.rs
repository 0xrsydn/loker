//! MCP adapter: each tool deserializes the contract's params, calls [`App`],
//! and returns the result as structured content. No logic lives here.
//!
//! Failures come back as tool results with `isError`, so the calling model can
//! read the message ("unknown country `Narnia`") and retry.
use std::sync::Arc;

use loker_core::api::{
    GetJobsParams, GetJobsResult, SearchParams, SearchResult, SourcesEval, Stats, VerifyParams,
    VerifyReport,
};
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::{ServerHandler, ServiceExt, tool, tool_handler, tool_router};

use crate::App;

#[derive(Clone)]
pub struct LokerMcp {
    app: Arc<App>,
}

#[tool_router]
impl LokerMcp {
    #[tool(
        description = "Full details (eligibility evidence, pay, tags, every source) for job ids from \
                       `search_jobs`, with the viewer's verdict when `country` is given. Closed jobs are \
                       included with `status: closed`; unknown ids come back in `missing`.",
        annotations(read_only_hint = true)
    )]
    async fn get_jobs(
        &self,
        Parameters(p): Parameters<GetJobsParams>,
    ) -> Result<Json<GetJobsResult>, String> {
        self.app
            .get_jobs(p)
            .await
            .map(Json)
            .map_err(|e| e.to_string())
    }

    #[tool(
        description = "Search stored jobs. Give `country` (and ideally `tz`) to get an eligibility verdict \
                       per job (yes / maybe / unknown, with a reason); jobs the viewer cannot take are dropped. \
                       `minVerdict: \"yes\"` keeps only jobs the viewer can apply to. To browse cheaply, use \
                       `view: \"compact\"` (about a third of the size), then `get_jobs` for full details of the picks. \
                       Every query word must match, so prefer short queries (\"backend\", not \"backend engineer python\").",
        annotations(read_only_hint = true)
    )]
    async fn search_jobs(
        &self,
        Parameters(p): Parameters<SearchParams>,
    ) -> Result<Json<SearchResult>, String> {
        self.app
            .search_jobs(p)
            .await
            .map(Json)
            .map_err(|e| e.to_string())
    }

    #[tool(
        description = "Count active jobs, total and per eligibility scope.",
        annotations(read_only_hint = true)
    )]
    async fn stats(&self) -> Result<Json<Stats>, String> {
        self.app.stats().await.map(Json).map_err(|e| e.to_string())
    }

    #[tool(
        description = "Per-source quality and overlap: job counts, unique jobs, field coverage \
                       (eligibility scope, timezone, pay, posted date), last run, cross-source \
                       overlaps, blocked merges and eligibility conflicts between merged sources.",
        annotations(read_only_hint = true)
    )]
    async fn sources_eval(&self) -> Result<Json<SourcesEval>, String> {
        self.app
            .sources_eval()
            .await
            .map(Json)
            .map_err(|e| e.to_string())
    }

    #[tool(
        description = "Fetch sources live (nothing is stored) and check adapter invariants: \
                       non-empty, parse rate, unique ids, required fields, ids for closures, dates. \
                       Default: every source in the store.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn verify_sources(
        &self,
        Parameters(p): Parameters<VerifyParams>,
    ) -> Result<Json<VerifyReport>, String> {
        self.app
            .verify(p)
            .await
            .map(Json)
            .map_err(|e| e.to_string())
    }
}

#[tool_handler(
    name = "loker",
    instructions = "Job search over loker's local store. Pass the viewer's `country` and `tz` to \
                    `search_jobs` so each job carries an eligibility verdict with a reason; \
                    `unknown` means the posting doesn't say, not that the viewer is excluded. \
                    Browse with `view: \"compact\"` and get full details for the picks with `get_jobs`."
)]
impl ServerHandler for LokerMcp {}

/// Serve MCP over stdin/stdout until the client disconnects.
pub async fn serve_stdio(app: Arc<App>) -> anyhow::Result<()> {
    let service = LokerMcp { app }.serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}
