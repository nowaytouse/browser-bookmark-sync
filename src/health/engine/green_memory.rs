//! Positive-only local memory for isolated batches. Never writes health runs or attempts.
use super::{
    BackendObservation, DateTime, HealthVerdict, ObservationSource, ProbeContext, ProbeScope,
    ProbeTarget, StateMachine, Utc,
};
use crate::health::{models::ConfigHashInput, state_machine::CURRENT_HEALTH_RULE_VERSION};
use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs::OpenOptions, io::ErrorKind, path::Path};

type ResultRow = (String, String, HealthVerdict, Vec<BackendObservation>);
const APP_ID: i32 = 0x42534731; // BSG1
pub(super) struct GreenMemory(Connection);
pub(super) struct Stage<'a> {
    db: &'a Connection,
    scope: ProbeScope,
    config: String,
    settings: String,
}

#[derive(Serialize, Deserialize)]
struct Green {
    verdict: HealthVerdict,
    source: ObservationSource,
    started: DateTime<Utc>,
    finished: DateTime<Utc>,
    status: u16,
}

fn slot(scope: ProbeScope, url: &str) -> String {
    let mut hash = blake3::Hasher::new();
    hash.update(match scope {
        ProbeScope::Site => b"site\0",
        ProbeScope::Content => b"page\0",
    });
    hash.update(url.as_bytes());
    hash.finalize().to_hex().to_string()
}

fn policy(ctx: &ProbeContext) -> Result<(String, String)> {
    let config = ConfigHashInput::from_probe_context(ctx).stable_hash()?;
    // The public evidence hash intentionally hides proxy endpoints. For local reuse a
    // different endpoint must miss; do not persist the address or its credentials.
    let settings = blake3::hash(&serde_json::to_vec(&(
        &config,
        &ctx.proxy_url,
        ctx.slow_mode,
    ))?)
    .to_hex()
    .to_string();
    Ok((config, settings))
}

impl Green {
    fn valid(&self, config: &str, now: DateTime<Utc>) -> bool {
        matches!(self.verdict, HealthVerdict::Live | HealthVerdict::Recovered)
            && (200..300).contains(&self.status)
            && self.source.config_hash == config
            && self.source.rule_version == CURRENT_HEALTH_RULE_VERSION
            && !self.source.run_id.trim().is_empty()
            && !self.source.check_id.trim().is_empty()
            && StateMachine::is_current_evidence(self.started, now)
            && self.started <= self.finished
            && self.finished <= now
    }

    fn reused(self, id: &str, url: &str) -> ResultRow {
        let obs = BackendObservation {
            backend: "green_memory".into(),
            backend_version: env!("CARGO_PKG_VERSION").into(),
            route: "history".into(),
            initial_url: url.into(),
            request_method: "REUSE".into(),
            actual_request: "no".into(),
            cache_hit: true,
            status_code: Some(self.status),
            started_at: self.started,
            finished_at: self.finished,
            run_id: self.source.run_id.clone(),
            check_id: self.source.check_id.clone(),
            source: Some(self.source),
            ..Default::default()
        };
        (id.into(), url.into(), self.verdict, vec![obs])
    }
}

impl GreenMemory {
    pub(super) fn open(path: &Path) -> Result<Self> {
        // macOS exposes temporary directories below /var -> /private/var. Resolve
        // the parent only: SQLite NOFOLLOW must still reject a linked file itself.
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let path = parent
            .canonicalize()?
            .join(path.file_name().context("Green memory needs a file name")?);
        let mut opts = OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let new = match opts.open(&path) {
            Ok(_) => true,
            Err(e) if e.kind() == ErrorKind::AlreadyExists => false,
            Err(e) => return Err(e.into()),
        };
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        conn.pragma_update(None, "trusted_schema", false)?;
        conn.pragma_update(None, "locking_mode", "EXCLUSIVE")?;
        let tx = conn.unchecked_transaction()?;
        if new {
            tx.execute_batch("CREATE TABLE greens(slot TEXT PRIMARY KEY, settings TEXT NOT NULL, started TEXT NOT NULL, evidence TEXT NOT NULL)")?;
            tx.pragma_update(None, "application_id", APP_ID)?;
        } else {
            ensure!(
                tx.pragma_query_value(None, "application_id", |r| r.get::<_, i32>(0))? == APP_ID,
                "Not a Bookmark Cleaner green-memory database"
            );
        }
        // Expiry never slides on reading. This write also takes the exclusive writer
        // lock for the life of the engine, so overlapping runs cannot publish stale wins.
        tx.execute("DELETE FROM greens WHERE julianday(started) < julianday(?1) OR julianday(started) IS NULL",
            [(Utc::now() - chrono::Duration::hours(24)).to_rfc3339()])?;
        tx.commit()?;
        Ok(Self(conn))
    }

    pub(super) fn stage(&self, ctx: &ProbeContext) -> Result<Stage<'_>> {
        let (config, settings) = policy(ctx)?;
        Ok(Stage {
            db: &self.0,
            scope: ctx.probe_scope,
            config,
            settings,
        })
    }
}

impl Stage<'_> {
    pub(super) fn lookup(
        &self,
        targets: &[ProbeTarget],
    ) -> Result<HashMap<(String, String), ResultRow>> {
        let now = Utc::now();
        let mut hits = HashMap::new();
        let tx = self.db.unchecked_transaction()?;
        {
            let mut get = tx.prepare("SELECT settings,evidence FROM greens WHERE slot=?1")?;
            let mut invalidate = tx.prepare("DELETE FROM greens WHERE slot=?1")?;
            for target in targets {
                let key = slot(self.scope, &target.original_url);
                let row: Option<(String, String)> = get
                    .query_row([&key], |r| Ok((r.get(0)?, r.get(1)?)))
                    .optional()?;
                let Some((saved, json)) = row else { continue };
                if let Some(green) = serde_json::from_str::<Green>(&json)
                    .ok()
                    .filter(|g| saved == self.settings && g.valid(&self.config, now))
                {
                    hits.insert(
                        (target.bookmark_id.clone(), target.original_url.clone()),
                        green.reused(&target.bookmark_id, &target.original_url),
                    );
                } else {
                    // A fresh probe (even one interrupted before completion) supersedes this memory.
                    invalidate.execute([&key])?;
                }
            }
        }
        tx.commit()?;
        Ok(hits)
    }

    pub(super) fn remember(&self, (_, url, verdict, observations): &ResultRow) -> Result<()> {
        let now = Utc::now();
        if !matches!(verdict, HealthVerdict::Live | HealthVerdict::Recovered) {
            return Ok(());
        }
        for obs in observations {
            if obs.actual_request != "yes"
                || obs.cache_hit
                || obs.initial_url != *url
                || obs.error_kind.is_some()
                || !matches!(obs.status_code, Some(200..=299))
                || StateMachine::compute_single_run_verdict_for_scope(
                    std::slice::from_ref(obs),
                    self.scope,
                ) != HealthVerdict::Live
            {
                continue;
            }
            let Some(source) = &obs.source else { continue };
            if source.run_id != obs.run_id || source.check_id != obs.check_id {
                continue;
            }
            let green = Green {
                verdict: *verdict,
                source: source.clone(),
                started: obs.started_at,
                finished: obs.finished_at,
                status: obs.status_code.unwrap_or_default(),
            };
            if green.valid(&self.config, now) {
                self.db.prepare_cached("INSERT OR REPLACE INTO greens(slot,settings,started,evidence) VALUES (?1,?2,?3,?4)")?.execute(params![
                            slot(self.scope, url),
                            self.settings,
                            green.started.to_rfc3339(),
                            serde_json::to_string(&green)?
                        ])?;
                break;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn green_memory_refuses_foreign_databases_and_linked_files() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let other = temp.path().join("existing.sqlite");
        Connection::open(&other)?.execute_batch("CREATE TABLE precious (value TEXT)")?;
        let original = std::fs::read(&other)?;
        assert!(GreenMemory::open(&other).is_err());
        assert_eq!(std::fs::read(&other)?, original);
        #[cfg(unix)]
        {
            let real = temp.path().join("valid.sqlite");
            drop(GreenMemory::open(&real)?);
            let link = temp.path().join("linked.sqlite");
            std::os::unix::fs::symlink(&real, &link)?;
            assert!(GreenMemory::open(&link).is_err());
        }
        Ok(())
    }
}
