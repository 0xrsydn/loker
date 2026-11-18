//! loker core: the domain model, eligibility verdicts and the SQLite store.
//! No transport code lives here; the CLI, REST and MCP are thin layers on top.

pub mod country;
pub mod home;
pub mod model;
pub mod store;
pub mod verdict;
