//! Health Audit and Multi-Probe Verification System

pub mod backends;
pub mod cli;
pub mod content;
pub mod db;
pub mod engine;
pub mod models;
pub mod quarantine;
pub mod reporting;
pub mod review;
pub mod scheduler;
pub mod security;
pub mod state_machine;
pub mod targets;

#[cfg(test)]
pub mod tests;

pub use cli::HealthCommands;
pub use engine::{HealthEngine, HealthEngineOptions};
pub use models::{HealthVerdict, ProbeTarget};
pub use targets::{
    build_probe_plan, build_probe_plan_from_probe_targets, BookmarkTargetInput, DnsPolicy,
    InvalidTarget, ProbePlan, ProbeScope, SiteTarget, SkippedTarget, TrustedSiteSkipList,
};
