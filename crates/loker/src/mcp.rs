//! MCP adapter: each tool deserializes the contract's params, calls [`App`],
//! and returns the result as structured content. No logic lives here.
//!
//! Failures come back as tool results with `isError`, so the calling model can
//! read the message ("unknown country `Narnia`") and retry.
use std::sync::Arc;

use loker_core::api::{SearchParams, SearchResult, Stats};
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
        description = "Search stored jobs. Give `country` (and ideally `tz`) to get an eligibility verdict \
                       per job (yes / maybe / unknown, with a reason); jobs the viewer cannot take are dropped.",
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
}

#[tool_handler(
    name = "loker",
    instructions = "Job search over loker's local store. Pass the viewer's `country` and `tz` to \
                    `search_jobs` so each job carries an eligibility verdict with a reason; \
                    `unknown` means the posting doesn't say, not that the viewer is excluded."
)]
impl ServerHandler for LokerMcp {}

/// Serve MCP over stdin/stdout until the client disconnects.
pub async fn serve_stdio(app: Arc<App>) -> anyhow::Result<()> {
    let service = LokerMcp { app }.serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}
