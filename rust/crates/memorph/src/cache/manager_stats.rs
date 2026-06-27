use std::collections::BTreeSet;
use std::sync::{Arc, OnceLock};

use crate::cache::{CachePolicy, CacheStore};
use crate::core::manager::{ManagerFilter, ManagerStatsResult};

pub struct ManagerStatsCache {
    store: CacheStore<String, ManagerStatsResult>,
}

impl ManagerStatsCache {
    pub fn new(policy: CachePolicy) -> Self {
        Self {
            store: CacheStore::new(policy),
        }
    }

    pub fn get(&self, filter: &ManagerFilter) -> Option<ManagerStatsResult> {
        self.store.get_fresh(&cache_key(filter))
    }

    pub fn set(&self, filter: &ManagerFilter, result: ManagerStatsResult) {
        self.store.set(cache_key(filter), result);
    }

    pub fn invalidate_all(&self) {
        self.store.clear();
    }
}

static MANAGER_STATS_CACHE: OnceLock<Arc<ManagerStatsCache>> = OnceLock::new();

pub fn manager_stats_cache() -> Arc<ManagerStatsCache> {
    MANAGER_STATS_CACHE
        .get_or_init(|| Arc::new(ManagerStatsCache::new(CachePolicy::ttl_seconds(30))))
        .clone()
}

fn cache_key(filter: &ManagerFilter) -> String {
    let providers = filter.providers.iter().collect::<BTreeSet<_>>();
    format!(
        "providers={}|older_days={:?}|older_ms={:?}|larger_mb={:?}|larger_bytes={:?}|smaller_bytes={:?}|workspace={:?}|sort={:?}",
        providers
            .into_iter()
            .map(|value| value.as_str())
            .collect::<Vec<_>>()
            .join(","),
        filter.older_than_days,
        filter.older_than_ms,
        filter.larger_than_mb,
        filter.larger_than_bytes,
        filter.smaller_than_bytes,
        filter.workspace,
        filter.sort
    )
}
