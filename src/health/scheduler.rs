//! Host/Domain Scheduler and Rate Limiter

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
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

struct HostState {
    last_request: Option<Instant>,
    cooldown_until: Option<Instant>,
    active_requests: usize,
}

/// Host & Domain Rate Limiter for probes
#[derive(Clone)]
pub struct HostScheduler {
    config: SchedulerConfig,
    hosts: Arc<Mutex<HashMap<String, HostState>>>,
    global_semaphore: Arc<tokio::sync::Semaphore>,
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
                let mut guard = self.hosts.lock().await;
                let now = Instant::now();
                let mut delay = Duration::ZERO;

                for &k in keys {
                    let state = guard.entry(k.to_string()).or_insert(HostState {
                        last_request: None,
                        cooldown_until: None,
                        active_requests: 0,
                    });

                    let key_delay = if let Some(until) = state.cooldown_until {
                        if now < until {
                            now.duration_since(now) + (until - now)
                        } else {
                            state.cooldown_until = None;
                            Duration::ZERO
                        }
                    } else if state.active_requests >= self.config.per_host_concurrency {
                        Duration::from_millis(100)
                    } else if let Some(last) = state.last_request {
                        let interval = Duration::from_millis(self.config.host_interval_ms);
                        let elapsed = now.duration_since(last);
                        if elapsed < interval {
                            interval.checked_sub(elapsed).unwrap_or_default()
                        } else {
                            Duration::ZERO
                        }
                    } else {
                        Duration::ZERO
                    };

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
                sleep(max_delay).await;
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
        let mut guard = self.hosts.lock().await;
        let now = Instant::now();
        for &k in keys {
            if let Some(state) = guard.get_mut(k) {
                if state.active_requests > 0 {
                    state.active_requests -= 1;
                }
                state.last_request = Some(now);

                if let Some(cooldown) = cooldown_duration {
                    state.cooldown_until = Some(now + cooldown);
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
        const MAX_COOLDOWN_SECS: u64 = 60;
        let bounded = |seconds: u64| Duration::from_secs(seconds.min(MAX_COOLDOWN_SECS));
        if let Ok(secs) = header_val.trim().parse::<u64>() {
            bounded(secs)
        } else if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(header_val.trim()) {
            let now = chrono::Utc::now();
            let target = dt.with_timezone(&chrono::Utc);
            if target > now {
                bounded((target - now).num_seconds().max(1) as u64)
            } else {
                bounded(default_secs)
            }
        } else {
            bounded(default_secs)
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
            Duration::from_secs(60)
        );
        assert_eq!(
            HostScheduler::parse_retry_after("invalid", 45),
            Duration::from_secs(45)
        );
        assert_eq!(
            HostScheduler::parse_retry_after("18446744073709551615", 30),
            Duration::from_secs(60)
        );
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
