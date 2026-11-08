//! Shared cache + rate-limit counters.
//!
//! Redis when `REDIS_URL` is set (shared by every API replica behind the load
//! balancer), otherwise an in-process store. If Redis errors at runtime the call
//! falls back to the local store instead of failing the request.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use tokio::sync::Mutex;

const PREFIX: &str = "caw:";

#[derive(Clone)]
pub struct Cache {
    redis: Option<ConnectionManager>,
    local: Arc<Mutex<Local>>,
    pub stats: Arc<CacheStats>,
}

#[derive(Default)]
pub struct CacheStats {
    pub hits: std::sync::atomic::AtomicU64,
    pub misses: std::sync::atomic::AtomicU64,
    pub redis_errors: std::sync::atomic::AtomicU64,
}

#[derive(Default)]
struct Local {
    kv: HashMap<String, (Instant, String)>,
    windows: HashMap<String, (u64, u64, u64)>, // key -> (window index, current, previous)
}

#[derive(Debug, Clone, Copy)]
pub struct RateDecision {
    pub allowed: bool,
    pub limit: u64,
    pub remaining: u64,
    pub reset_secs: u64,
}

const SLIDING_WINDOW_LUA: &str = r"
local count = redis.call('INCR', KEYS[1])
if count == 1 then redis.call('PEXPIRE', KEYS[1], tonumber(ARGV[1]) * 2) end
local prev = tonumber(redis.call('GET', KEYS[2]) or '0')
return {count, prev}
";

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl Cache {
    pub async fn connect(url: Option<&str>) -> Cache {
        let redis = match url {
            Some(u) => match redis::Client::open(u) {
                Ok(client) => match tokio::time::timeout(
                    Duration::from_secs(5),
                    ConnectionManager::new(client),
                )
                .await
                {
                    Ok(Ok(m)) => {
                        tracing::info!("connected to Redis");
                        Some(m)
                    }
                    Ok(Err(e)) => {
                        tracing::warn!(error = %e, "Redis unavailable; using in-process cache");
                        None
                    }
                    Err(_) => {
                        tracing::warn!("Redis connect timed out; using in-process cache");
                        None
                    }
                },
                Err(e) => {
                    tracing::warn!(error = %e, "invalid REDIS_URL; using in-process cache");
                    None
                }
            },
            None => None,
        };
        Cache {
            redis,
            local: Arc::new(Mutex::new(Local::default())),
            stats: Arc::new(CacheStats::default()),
        }
    }

    pub fn backend(&self) -> &'static str {
        if self.redis.is_some() {
            "redis"
        } else {
            "memory"
        }
    }

    pub async fn ping(&self) -> bool {
        match &self.redis {
            Some(m) => {
                let mut m = m.clone();
                redis::cmd("PING")
                    .query_async::<String>(&mut m)
                    .await
                    .is_ok()
            }
            None => true,
        }
    }

    fn redis_failed(&self, e: redis::RedisError) {
        self.stats
            .redis_errors
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        tracing::warn!(error = %e, "Redis call failed; falling back to local cache");
    }

    pub async fn get(&self, key: &str) -> Option<String> {
        let key = format!("{PREFIX}{key}");
        let found = match &self.redis {
            Some(m) => match m.clone().get::<_, Option<String>>(&key).await {
                Ok(v) => v,
                Err(e) => {
                    self.redis_failed(e);
                    self.local_get(&key).await
                }
            },
            None => self.local_get(&key).await,
        };
        let counter = if found.is_some() {
            &self.stats.hits
        } else {
            &self.stats.misses
        };
        counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        found
    }

    async fn local_get(&self, key: &str) -> Option<String> {
        let mut l = self.local.lock().await;
        match l.kv.get(key) {
            Some((exp, v)) if *exp > Instant::now() => Some(v.clone()),
            Some(_) => {
                l.kv.remove(key);
                None
            }
            None => None,
        }
    }

    pub async fn set(&self, key: &str, value: &str, ttl: Duration) {
        let key = format!("{PREFIX}{key}");
        if let Some(m) = &self.redis {
            match m
                .clone()
                .set_ex::<_, _, ()>(&key, value, ttl.as_secs().max(1))
                .await
            {
                Ok(()) => return,
                Err(e) => self.redis_failed(e),
            }
        }
        let mut l = self.local.lock().await;
        if l.kv.len() > 10_000 {
            let now = Instant::now();
            l.kv.retain(|_, (exp, _)| *exp > now);
        }
        l.kv.insert(key, (Instant::now() + ttl, value.to_string()));
    }

    /// Set only if absent (idempotency keys). Returns true when this call set it.
    pub async fn set_nx(&self, key: &str, value: &str, ttl: Duration) -> bool {
        let key = format!("{PREFIX}{key}");
        if let Some(m) = &self.redis {
            let res: Result<Option<String>, _> = redis::cmd("SET")
                .arg(&key)
                .arg(value)
                .arg("NX")
                .arg("EX")
                .arg(ttl.as_secs().max(1))
                .query_async(&mut m.clone())
                .await;
            match res {
                Ok(v) => return v.is_some(),
                Err(e) => self.redis_failed(e),
            }
        }
        let mut l = self.local.lock().await;
        let now = Instant::now();
        if matches!(l.kv.get(&key), Some((exp, _)) if *exp > now) {
            return false;
        }
        l.kv.insert(key, (now + ttl, value.to_string()));
        true
    }

    pub async fn delete(&self, key: &str) {
        let key = format!("{PREFIX}{key}");
        if let Some(m) = &self.redis {
            if let Err(e) = m.clone().del::<_, ()>(&key).await {
                self.redis_failed(e);
            }
        }
        self.local.lock().await.kv.remove(&key);
    }

    /// Sliding-window rate limit (weighted previous + current fixed window).
    pub async fn rate_limit(
        &self,
        bucket: &str,
        id: &str,
        limit: u64,
        window: Duration,
    ) -> RateDecision {
        let win_ms = window.as_millis().max(1) as u64;
        let now = unix_ms();
        let idx = now / win_ms;
        let elapsed = (now % win_ms) as f64 / win_ms as f64;
        let reset_secs = ((win_ms - now % win_ms) / 1000).max(1);

        let counts = match &self.redis {
            Some(m) => {
                let cur = format!("{PREFIX}rl:{bucket}:{id}:{idx}");
                let prev = format!("{PREFIX}rl:{bucket}:{id}:{}", idx.saturating_sub(1));
                let res: Result<(u64, u64), _> = redis::Script::new(SLIDING_WINDOW_LUA)
                    .key(cur)
                    .key(prev)
                    .arg(win_ms)
                    .invoke_async(&mut m.clone())
                    .await;
                match res {
                    Ok(v) => Some(v),
                    Err(e) => {
                        self.redis_failed(e);
                        None
                    }
                }
            }
            None => None,
        };
        let (current, previous) = match counts {
            Some(c) => c,
            None => {
                let mut l = self.local.lock().await;
                if l.windows.len() > 50_000 {
                    l.windows.retain(|_, (w, _, _)| *w + 1 >= idx);
                }
                let key = format!("{bucket}:{id}");
                let entry = l.windows.entry(key).or_insert((idx, 0, 0));
                if entry.0 != idx {
                    entry.2 = if entry.0 + 1 == idx { entry.1 } else { 0 };
                    entry.1 = 0;
                    entry.0 = idx;
                }
                entry.1 += 1;
                (entry.1, entry.2)
            }
        };
        let weighted = previous as f64 * (1.0 - elapsed) + current as f64;
        let allowed = weighted <= limit as f64;
        RateDecision {
            allowed,
            limit,
            remaining: (limit as f64 - weighted).max(0.0).floor() as u64,
            reset_secs,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn memory_cache_and_limit() {
        let c = Cache::connect(None).await;
        c.set("a", "1", Duration::from_secs(60)).await;
        assert_eq!(c.get("a").await.as_deref(), Some("1"));
        assert!(c.set_nx("k", "x", Duration::from_secs(60)).await);
        assert!(!c.set_nx("k", "y", Duration::from_secs(60)).await);
        let mut allowed = 0;
        for _ in 0..10 {
            if c.rate_limit("t", "ip", 5, Duration::from_secs(60))
                .await
                .allowed
            {
                allowed += 1;
            }
        }
        assert_eq!(allowed, 5);
    }
}
