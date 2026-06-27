use std::sync::{Arc, OnceLock};

use crate::agent_environment::AgentEnvironmentStatus;
use crate::cache::{CachePolicy, CacheStore};

const AGENT_ENVIRONMENT_CACHE_TTL_SECONDS: u64 = 300;

pub struct AgentEnvironmentCache {
    full: CacheStore<String, AgentEnvironmentStatus>,
    fast: CacheStore<String, AgentEnvironmentStatus>,
}

impl AgentEnvironmentCache {
    pub fn new(policy: CachePolicy) -> Self {
        Self {
            full: CacheStore::new(policy),
            fast: CacheStore::new(policy),
        }
    }

    pub fn get_full(&self, provider_id: &str) -> Option<AgentEnvironmentStatus> {
        self.full.get_fresh(&provider_key(provider_id))
    }

    pub fn get_fast(&self, provider_id: &str) -> Option<AgentEnvironmentStatus> {
        self.fast.get_fresh(&provider_key(provider_id))
    }

    pub fn set_full(&self, provider_id: &str, status: AgentEnvironmentStatus) {
        let key = provider_key(provider_id);
        self.full.set(key.clone(), status.clone());
        self.fast.set(key, status);
    }

    pub fn set_fast(&self, provider_id: &str, status: AgentEnvironmentStatus) {
        self.fast.set(provider_key(provider_id), status);
    }

    pub fn invalidate(&self, provider_id: &str) {
        let key = provider_key(provider_id);
        self.full.invalidate(&key);
        self.fast.invalidate(&key);
    }

    pub fn invalidate_all(&self) {
        self.full.clear();
        self.fast.clear();
    }
}

static AGENT_ENVIRONMENT_CACHE: OnceLock<Arc<AgentEnvironmentCache>> = OnceLock::new();

pub fn agent_environment_cache() -> Arc<AgentEnvironmentCache> {
    AGENT_ENVIRONMENT_CACHE
        .get_or_init(|| {
            Arc::new(AgentEnvironmentCache::new(CachePolicy::ttl_seconds(
                AGENT_ENVIRONMENT_CACHE_TTL_SECONDS,
            )))
        })
        .clone()
}

fn provider_key(provider_id: &str) -> String {
    provider_id.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(version: Option<&str>) -> AgentEnvironmentStatus {
        AgentEnvironmentStatus {
            installed: true,
            executable_path: Some("/bin/demo".to_string()),
            executable_dir: Some("/bin".to_string()),
            config_path: "/tmp/demo".to_string(),
            install_method: "unknown".to_string(),
            executable_version: version.map(str::to_string),
        }
    }

    #[test]
    fn full_status_is_returned_while_fresh() {
        let cache = AgentEnvironmentCache::new(CachePolicy::ttl_seconds(30));
        let value = status(Some("demo 1.0.0"));

        cache.set_full("demo", value.clone());

        assert_eq!(cache.get_full("demo"), Some(value));
    }

    #[test]
    fn fast_status_is_returned_while_fresh() {
        let cache = AgentEnvironmentCache::new(CachePolicy::ttl_seconds(30));
        let value = status(None);

        cache.set_fast("demo", value.clone());

        assert_eq!(cache.get_fast("demo"), Some(value));
    }

    #[test]
    fn set_full_also_populates_fast_lane() {
        let cache = AgentEnvironmentCache::new(CachePolicy::ttl_seconds(30));
        let value = status(Some("demo 1.0.0"));

        cache.set_full("demo", value.clone());

        assert_eq!(cache.get_fast("demo"), Some(value));
    }

    #[test]
    fn invalidate_removes_provider_from_both_lanes() {
        let cache = AgentEnvironmentCache::new(CachePolicy::ttl_seconds(30));

        cache.set_full("demo", status(Some("demo 1.0.0")));
        cache.invalidate("demo");

        assert!(cache.get_full("demo").is_none());
        assert!(cache.get_fast("demo").is_none());
    }
}
