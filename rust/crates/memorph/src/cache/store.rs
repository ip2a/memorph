use std::collections::HashMap;
use std::hash::Hash;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use anyhow::Result;

#[derive(Clone, Debug)]
pub struct CacheEntry<V> {
    value: V,
    refreshed_at: Instant,
}

impl<V> CacheEntry<V> {
    pub fn new(value: V) -> Self {
        Self {
            value,
            refreshed_at: Instant::now(),
        }
    }

    pub fn with_refreshed_at(value: V, refreshed_at: Instant) -> Self {
        Self {
            value,
            refreshed_at,
        }
    }

    pub fn value(&self) -> &V {
        &self.value
    }

    pub fn into_value(self) -> V {
        self.value
    }

    pub fn refreshed_at(&self) -> Instant {
        self.refreshed_at
    }

    pub fn is_fresh(&self, policy: CachePolicy) -> bool {
        self.refreshed_at.elapsed() < policy.duration()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CachePolicy {
    ttl: Duration,
}

impl CachePolicy {
    pub fn ttl(ttl: Duration) -> Self {
        Self { ttl }
    }

    pub fn ttl_seconds(seconds: u64) -> Self {
        Self::ttl(Duration::from_secs(seconds))
    }

    pub fn ttl_millis(millis: u64) -> Self {
        Self::ttl(Duration::from_millis(millis))
    }

    pub fn duration(&self) -> Duration {
        self.ttl
    }
}

pub struct CacheStore<K, V> {
    entries: RwLock<HashMap<K, CacheEntry<V>>>,
    policy: CachePolicy,
}

impl<K, V> CacheStore<K, V>
where
    K: Eq + Hash,
    V: Clone,
{
    pub fn new(policy: CachePolicy) -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            policy,
        }
    }

    pub fn policy(&self) -> CachePolicy {
        self.policy
    }

    pub fn get(&self, key: &K) -> Option<CacheEntry<V>> {
        let entries = self.entries.read().unwrap();
        entries.get(key).cloned()
    }

    pub fn get_fresh(&self, key: &K) -> Option<V> {
        let entries = self.entries.read().unwrap();
        let entry = entries.get(key)?;
        entry.is_fresh(self.policy).then(|| entry.value().clone())
    }

    pub fn get_or_refresh<F>(&self, key: K, refresh_fn: F) -> Result<V>
    where
        F: FnOnce() -> Result<V>,
    {
        if let Some(value) = self.get_fresh(&key) {
            return Ok(value);
        }

        let refreshed = refresh_fn()?;

        let mut entries = self.entries.write().unwrap();
        if let Some(entry) = entries.get(&key) {
            if entry.is_fresh(self.policy) {
                return Ok(entry.value().clone());
            }
        }
        entries.insert(key, CacheEntry::new(refreshed.clone()));
        Ok(refreshed)
    }

    pub fn set(&self, key: K, value: V) {
        self.set_entry(key, CacheEntry::new(value));
    }

    pub fn set_entry(&self, key: K, entry: CacheEntry<V>) {
        self.entries.write().unwrap().insert(key, entry);
    }

    pub fn invalidate(&self, key: &K) {
        self.entries.write().unwrap().remove(key);
    }

    pub fn clear(&self) {
        self.entries.write().unwrap().clear();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn returns_fresh_cached_value_without_refreshing() {
        let store = CacheStore::new(CachePolicy::ttl_seconds(30));
        store.set("provider".to_string(), vec!["a".to_string()]);

        let refresh_count = AtomicUsize::new(0);
        let value = store
            .get_or_refresh("provider".to_string(), || {
                refresh_count.fetch_add(1, Ordering::SeqCst);
                Ok(vec!["b".to_string()])
            })
            .unwrap();

        assert_eq!(value, vec!["a".to_string()]);
        assert_eq!(refresh_count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn refreshes_expired_entries() {
        let store = CacheStore::new(CachePolicy::ttl_millis(1));
        store.set_entry(
            "provider".to_string(),
            CacheEntry::with_refreshed_at(
                vec!["old".to_string()],
                Instant::now() - Duration::from_secs(1),
            ),
        );

        let value = store
            .get_or_refresh("provider".to_string(), || Ok(vec!["new".to_string()]))
            .unwrap();

        assert_eq!(value, vec!["new".to_string()]);
    }

    #[test]
    fn invalidate_removes_entry() {
        let store = CacheStore::new(CachePolicy::ttl_seconds(30));
        let key = "provider".to_string();
        store.set(key.clone(), vec!["a".to_string()]);

        store.invalidate(&key);

        assert!(store.get(&key).is_none());
    }
}
