//! # kept-core
//!
//! Everything that makes Kept work, independent of the desktop UI:
//!
//! | module        | responsibility                                                       |
//! |---------------|----------------------------------------------------------------------|
//! | `db`          | SQLite connection, migrations, backup, audit log                     |
//! | `prefs`       | user preferences                                                     |
//! | `connectors`  | Google, Microsoft Graph, file imports, OAuth (PKCE + loopback)       |
//! | `ingest`      | store provider data; turn extractions into tasks / updates / evidence|
//! | `extract`     | commitment detection — rules (offline) and AI (schema-validated)     |
//! | `ai`          | provider-agnostic LLM client (Anthropic, OpenAI, compatible)         |
//! | `dates`       | deterministic deadline interpretation                                |
//! | `tasks`       | lifecycle, deadlines precedence, dedup, merge, undo                  |
//! | `meetings`    | prep state and source-grounded briefings                             |
//! | `planner`     | priority scoring, daily plan, workload, dashboard                    |
//! | `notify`      | notification policy: dedup, stages, quiet hours, rate limits         |
//! | `agent`       | the background loop that ties it together                            |

pub mod agent;
pub mod ai;
pub mod connectors;
pub mod connsettings;
pub mod dates;
pub mod documents;
pub mod db;
pub mod extract;
pub mod followups;
pub mod ingest;
pub mod meetings;
pub mod models;
pub mod notify;
pub mod planner;
pub mod prefs;
pub mod secrets;
pub mod tasks;
pub mod text;

pub use agent::Agent;
