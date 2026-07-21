use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use anyhow::Result;
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};

use crate::cache::{CacheEntry, CachePolicy, CacheStore};
use crate::provider::ProviderSessionSummary;

pub type CachedSessions = CacheEntry<Vec<ProviderSessionSummary>>;

pub struct SessionCache {
    store: CacheStore<String, Vec<ProviderSessionSummary>>,
}

impl SessionCache {
    pub fn new(policy: CachePolicy) -> Self {
        Self {
            store: CacheStore::new(policy),
        }
    }

    pub fn get(&self, provider_id: &str) -> Option<CachedSessions> {
        self.store.get(&provider_id.to_string())
    }

    pub fn get_or_refresh<F>(
        &self,
        provider_id: &str,
        refresh_fn: F,
    ) -> Result<Vec<ProviderSessionSummary>>
    where
        F: FnOnce() -> Result<Vec<ProviderSessionSummary>>,
    {
        self.store
            .get_or_refresh(provider_id.to_string(), refresh_fn)
    }

    pub fn set(&self, provider_id: &str, sessions: Vec<ProviderSessionSummary>) {
        self.store.set(provider_id.to_string(), sessions);
    }

    pub fn invalidate(&self, provider_id: &str) {
        self.store.invalidate(&provider_id.to_string());
    }

    pub fn invalidate_all(&self) {
        self.store.clear();
    }
}

static CACHE: OnceLock<Arc<SessionCache>> = OnceLock::new();

pub fn global_cache() -> Arc<SessionCache> {
    CACHE
        .get_or_init(|| Arc::new(SessionCache::new(CachePolicy::ttl_seconds(5))))
        .clone()
}

pub fn init_cache() -> Arc<SessionCache> {
    global_cache()
}

pub struct CacheWatcher {
    #[allow(dead_code)]
    cache: Arc<SessionCache>,
    #[allow(dead_code)]
    watcher: RecommendedWatcher,
}

impl CacheWatcher {
    pub fn new(
        cache: Arc<SessionCache>,
        path_to_provider: HashMap<PathBuf, String>,
    ) -> Result<Self> {
        let cache_clone = cache.clone();
        let p2p = Arc::new(path_to_provider);
        let p2p_clone = p2p.clone();

        let mut watcher = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    for changed_path in &event.paths {
                        for (watched_path, provider_id) in p2p_clone.iter() {
                            if changed_path == watched_path
                                || changed_path.starts_with(watched_path)
                            {
                                cache_clone.invalidate(provider_id);
                                break;
                            }
                        }
                    }
                }
            },
            Config::default(),
        )?;

        for (path, _) in p2p.iter() {
            if path.exists() {
                let mode = if path.is_dir() {
                    RecursiveMode::Recursive
                } else {
                    RecursiveMode::NonRecursive
                };
                let _ = watcher.watch(path, mode);
            }
        }

        Ok(Self { cache, watcher })
    }
}

/// Build a path-to-provider mapping from all registered providers.
pub fn build_path_registry() -> HashMap<PathBuf, String> {
    let mut map = HashMap::new();
    for id in crate::providers::all_provider_ids() {
        let Some(prov) = crate::providers::find_provider(id) else {
            continue;
        };
        let pid = prov.id().to_string();
        for path in prov.data_source_paths() {
            if path.exists() {
                map.insert(path, pid.clone());
            }
        }
    }
    map
}

static WATCHER_INIT: std::sync::Once = std::sync::Once::new();

/// Initialize the global cache watcher exactly once per process.
pub fn init_watcher() {
    WATCHER_INIT.call_once(|| {
        let cache = global_cache();
        let registry = build_path_registry();
        match CacheWatcher::new(cache, registry) {
            Ok(watcher) => {
                let _ = Box::leak(Box::new(watcher));
            }
            Err(err) => {
                crate::logging::error(
                    "cache_watcher_init",
                    format!("Failed to initialize cache watcher: {err}"),
                );
            }
        }
    });
}
