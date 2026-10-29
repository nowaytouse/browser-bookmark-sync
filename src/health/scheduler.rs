//! Host/Domain Scheduler and Rate Limiter

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::time::{sleep, Duration, Instant};

#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    pub max_global_concurrency: usize,
    pub per_host_concurrency: usize,
    pub host_interval_ms: u64,
    pub default_429_cooldown_secs: u64,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            max_global_concurrency: 5,
            per_host_concurrency: 1,
            host_interval_ms: 200,
            default_429_cooldown_secs: 30,
        }
    }
}

impl SchedulerConfig {
    pub const fn slow_mode() -> Self {
        Self {
            max_global_concurrency: 2,
            per_host_concurrency: 1,
            host_interval_ms: 2000,
            default_429_cooldown_secs: 60,
        }
    }
}

#[derive(Debug)]
struct HostState {
    last_request: Option<Instant>,
    cooldown: Option<(Instant, Duration)>,
    active_requests: usize,
}

impl HostState {
    fn extend_cooldown(&mut self, now: Instant, duration: Duration) {
        let remaining = self
            .cooldown
            .map(|(start, duration)| duration.saturating_sub(now.duration_since(start)))
            .unwrap_or_default();
        self.cooldown = Some((now, duration.max(remaining)));
    }
}

/// Host & Domain Rate Limiter for probes
#[derive(Debug, Clone)]
pub struct HostScheduler {
    config: SchedulerConfig,
    hosts: Arc<Mutex<HashMap<String, HostState>>>,
    global_semaphore: Arc<tokio::sync::Semaphore>,
}

/// Permission for one physical request. Hold it until the response is consumed
/// or dropped; cancellation releases host accounting as well as the global slot.
#[derive(Debug)]
#[must_use = "hold the request permit until its response is consumed or dropped"]
pub struct RequestPermit {
    scheduler: HostScheduler,
    keys: Vec<String>,
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
}

impl RequestPermit {
    /// Retain the longest server delay observed for this request.
    pub fn set_cooldown(&mut self, duration: Duration) {
        // Other active requests must see headers immediately, not after this body ends.
        self.scheduler
            .register_cooldown_sync(self.keys.iter().map(String::as_str), duration);
    }
}

impl Drop for RequestPermit {
    fn drop(&mut self) {
        self.scheduler
            .release_keys_sync(self.keys.iter().map(String::as_str), None);
        // Host state must be released before another request gets the global slot.
        drop(self.permit.take());
    }
}

impl HostScheduler {
    pub fn new(config: SchedulerConfig) -> Self {
        let global_semaphore = Arc::new(tokio::sync::Semaphore::new(config.max_global_concurrency));
        Self {
            config,
            hosts: Arc::new(Mutex::new(HashMap::new())),
            global_semaphore,
        }
    }

    fn lock_hosts(&self) -> Result<MutexGuard<'_, HashMap<String, HostState>>, &'static str> {
        self.hosts.lock().map_err(|_| {
            self.global_semaphore.close();
            tracing::error!("Health request scheduler state poisoned; stopping further requests");
            "scheduler state lock poisoned"
        })
    }

    /// Acquire one cancellation-safe physical-request permit for the supplied
    /// host/domain keys, respecting the same limits and cooldowns as other probes.
    pub async fn acquire_request(&self, keys: &[&str]) -> Result<RequestPermit, &'static str> {
        let owned_keys = keys.iter().map(|key| (*key).to_owned()).collect();
        let permit = self.acquire_keys(keys).await?;
        Ok(RequestPermit {
            scheduler: self.clone(),
            keys: owned_keys,
            permit: Some(permit),
        })
    }

    /// Acquire permission to probe target keys (host and/or registrable domain), respecting interval and cooldown
    pub async fn acquire_keys(
        &self,
        keys: &[&str],
    ) -> Result<tokio::sync::OwnedSemaphorePermit, &'static str> {
        if keys.is_empty() {
            return self
                .global_semaphore
                .clone()
                .acquire_owned()
                .await
                .map_err(|_| "scheduler semaphore closed");
        }

        let mut permit = None;
        loop {
            let max_delay = {
                let mut guard = self.lock_hosts()?;
                let now = Instant::now();
                let mut delay = Duration::ZERO;

                for &k in keys {
                    let state = guard.entry(k.to_string()).or_insert(HostState {
                        last_request: None,
                        cooldown: None,
                        active_requests: 0,
                    });

                    let cooldown_delay = state
                        .cooldown
                        .map(|(start, duration)| duration.saturating_sub(now.duration_since(start)))
                        .unwrap_or_default();
                    let concurrency_delay =
                        if state.active_requests >= self.config.per_host_concurrency {
                            Duration::from_millis(100)
                        } else {
                            Duration::ZERO
                        };
                    let interval_delay = state
                        .last_request
                        .map(|last| {
                            Duration::from_millis(self.config.host_interval_ms)
                                .saturating_sub(now.duration_since(last))
                        })
                        .unwrap_or_default();
                    let key_delay = cooldown_delay.max(concurrency_delay).max(interval_delay);

                    if key_delay > delay {
                        delay = key_delay;
                    }
                }

                if delay.is_zero() {
                    let ready_permit = permit
                        .take()
                        .or_else(|| self.global_semaphore.clone().try_acquire_owned().ok());
                    if let Some(permit) = ready_permit {
                        let now = Instant::now();
                        for &k in keys {
                            if let Some(state) = guard.get_mut(k) {
                                state.active_requests += 1;
                                state.last_request = Some(now);
                            }
                        }
                        return Ok(permit);
                    }
                }

                delay
            };

            if max_delay.is_zero() {
                let acquired = self
                    .global_semaphore
                    .clone()
                    .acquire_owned()
                    .await
                    .map_err(|_| "scheduler semaphore closed")?;
                permit = Some(acquired);
            } else {
                drop(permit.take());
                // Long server delays remain enforced without overflowing the timer deadline.
                sleep(max_delay.min(Duration::from_secs(60))).await;
            }
        }
    }

    /// Single key helper for acquire
    pub async fn acquire(
        &self,
        host_key: &str,
    ) -> Result<tokio::sync::OwnedSemaphorePermit, &'static str> {
        self.acquire_keys(&[host_key]).await
    }

    /// Mark request complete for multiple keys and optionally register a 429 cooldown
    pub async fn release_keys(&self, keys: &[&str], cooldown_duration: Option<Duration>) {
        self.release_keys_sync(keys.iter().copied(), cooldown_duration);
    }

    fn register_cooldown_sync<'a>(
        &self,
        keys: impl IntoIterator<Item = &'a str>,
        duration: Duration,
    ) {
        let Ok(mut guard) = self.lock_hosts() else {
            return;
        };
        let now = Instant::now();
        for key in keys {
            if let Some(state) = guard.get_mut(key) {
                state.extend_cooldown(now, duration);
            }
        }
    }

    fn release_keys_sync<'a>(
        &self,
        keys: impl IntoIterator<Item = &'a str>,
        cooldown_duration: Option<Duration>,
    ) {
        let Ok(mut guard) = self.lock_hosts() else {
            // lock_hosts has closed scheduling and reported the poisoned state.
            return;
        };
        let now = Instant::now();
        for k in keys {
            if let Some(state) = guard.get_mut(k) {
                if state.active_requests > 0 {
                    state.active_requests -= 1;
                }
                state.last_request = Some(now);

                if let Some(cooldown) = cooldown_duration {
                    state.extend_cooldown(now, cooldown);
                }
            }
        }
    }

    /// Single key helper for release
    pub async fn release(&self, host_key: &str, cooldown_duration: Option<Duration>) {
        self.release_keys(&[host_key], cooldown_duration).await;
    }

    /// Parse Retry-After header string into Duration
    pub fn parse_retry_after(header_val: &str, default_secs: u64) -> Duration {
        if let Ok(secs) = header_val.trim().parse::<u64>() {
            Duration::from_secs(secs)
        } else if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(header_val.trim()) {
            let now = chrono::Utc::now();
            let target = dt.with_timezone(&chrono::Utc);
            if target > now {
                (target - now)
                    .to_std()
                    .unwrap_or_else(|_| Duration::from_secs(default_secs))
            } else {
                Duration::from_secs(default_secs)
            }
        } else {
            Duration::from_secs(default_secs)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retry_after_parsing() {
        assert_eq!(
            HostScheduler::parse_retry_after("120", 30),
            Duration::from_secs(120)
        );
        assert_eq!(
            HostScheduler::parse_retry_after("invalid", 45),
            Duration::from_secs(45)
        );
        assert_eq!(
            HostScheduler::parse_retry_after("18446744073709551615", 30),
            Duration::from_secs(u64::MAX)
        );
        let date = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc2822();
        let delay = HostScheduler::parse_retry_after(&date, 30);
        assert!(delay >= Duration::from_secs(3599) && delay <= Duration::from_secs(3600));
    }

    #[tokio::test]
    async fn retry_after_preserves_pacing_and_large_delays_without_blocking_other_hosts() {
        let scheduler = HostScheduler::new(SchedulerConfig {
            max_global_concurrency: 1,
            per_host_concurrency: 1,
            host_interval_ms: 100,
            default_429_cooldown_secs: 60,
        });
        let permit = scheduler.acquire("a").await.unwrap();
        scheduler.release("a", Some(Duration::ZERO)).await;
        drop(permit);
        assert!(
            tokio::time::timeout(Duration::from_millis(10), scheduler.acquire("a"))
                .await
                .is_err(),
            "Retry-After: 0 must not bypass host pacing"
        );

        scheduler
            .release("a", Some(Duration::from_secs(u64::MAX)))
            .await;
        scheduler.release("a", Some(Duration::ZERO)).await;
        assert!(
            tokio::time::timeout(Duration::from_millis(150), scheduler.acquire("a"))
                .await
                .is_err(),
            "a shorter cooldown must not erase an existing longer delay"
        );
        let other = tokio::time::timeout(Duration::from_millis(50), scheduler.acquire("b"))
            .await
            .expect("a delayed host blocked an unrelated host")
            .unwrap();
        scheduler.release("b", None).await;
        drop(other);
    }

    #[tokio::test]
    async fn cooldown_does_not_hold_global_permit_for_other_hosts() {
        let scheduler = HostScheduler::new(SchedulerConfig {
            max_global_concurrency: 1,
            per_host_concurrency: 1,
            host_interval_ms: 0,
            default_429_cooldown_secs: 1,
        });

        let permit = scheduler.acquire("a").await;
        scheduler
            .release("a", Some(Duration::from_millis(200)))
            .await;
        drop(permit);

        let waiting_scheduler = scheduler.clone();
        let waiting = tokio::spawn(async move {
            let permit = waiting_scheduler.acquire("a").await;
            sleep(Duration::from_millis(1)).await;
            drop(permit);
        });
        sleep(Duration::from_millis(10)).await;

        let other = tokio::time::timeout(Duration::from_millis(50), scheduler.acquire("b"))
            .await
            .expect("cooldown for host a blocked host b");
        drop(other);
        waiting.abort();
    }
}
