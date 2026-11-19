//! loker core: the domain model, eligibility verdicts and the SQLite store.
//! No transport code lives here; the CLI, REST and MCP are thin layers on top.

pub mod api;
pub mod config;
pub mod country;
pub mod dedup;
pub mod home;
pub mod model;
pub mod normalize;
pub mod role;
pub mod store;
pub mod verdict;
