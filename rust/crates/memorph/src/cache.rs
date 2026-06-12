use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use anyhow::Result;
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};

use crate::provider::ProviderSessionSummary;

#[derive(Clone, Debug)]
pub struct CachedSessions {
    pub sessions: Vec<ProviderSessionSummary>,
    pub refreshed_at: Instant,
}

pub struct SessionCache {
    data: RwLock<HashMap<String, CachedSessions>>,
    ttl: Duration,
}

impl SessionCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            data: RwLock::new(HashMap::new()),
            ttl,
        }
    }

    pub fn get(&self, provider_id: &str) -> Option<CachedSessions> {
        let data = self.data.read().unwrap();
        data.get(provider_id).cloned()
    }

    pub fn get_or_refresh<F>(
        &self,
        provider_id: &str,
        refresh_fn: F,
    ) -> Result<Vec<ProviderSessionSummary>>
    where
        F: FnOnce() -> Result<Vec<ProviderSessionSummary>>,
    {
        {
            let data = self.data.read().unwrap();
            if let Some(cached) = data.get(provider_id) {
                if cached.refreshed_at.elapsed() < self.ttl {
                    return Ok(cached.sessions.clone());
                }
            }
        }

        let sessions = refresh_fn()?;

        let mut data = self.data.write().unwrap();
        if let Some(cached) = data.get(provider_id) {
            if cached.refreshed_at.elapsed() < self.ttl {
                return Ok(cached.sessions.clone());
            }
        }
        let cached = CachedSessions {
            sessions: sessions.clone(),
            refreshed_at: Instant::now(),
        };
        data.insert(provider_id.to_string(), cached);
        Ok(sessions)
    }

    pub fn set(&self, provider_id: &str, sessions: Vec<ProviderSessionSummary>) {
        let cached = CachedSessions {
            sessions,
            refreshed_at: Instant::now(),
        };
        self.data.write().unwrap().insert(provider_id.to_string(), cached);
    }

    pub fn invalidate(&self, provider_id: &str) {
        self.data.write().unwrap().remove(provider_id);
    }

    pub fn invalidate_all(&self) {
        self.data.write().unwrap().clear();
    }
}

static CACHE: std::sync::OnceLock<Arc<SessionCache>> = std::sync::OnceLock::new();

pub fn global_cache() -> Arc<SessionCache> {
    CACHE
        .get_or_init(|| Arc::new(SessionCache::new(Duration::from_secs(5))))
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
    pub fn new(cache: Arc<SessionCache>, path_to_provider: HashMap<PathBuf, String>) -> Result<Self> {
        let cache_clone = cache.clone();
        let p2p = Arc::new(path_to_provider);
        let p2p_clone = p2p.clone();

        let mut watcher = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    for changed_path in &event.paths {
                        for (watched_path, provider_id) in p2p_clone.iter() {
                            if changed_path == watched_path || changed_path.starts_with(watched_path) {
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
        let Some(prov) = crate::providers::find_provider(id) else { continue };
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
        let watcher = CacheWatcher::new(cache, registry)
            .expect("Failed to initialize cache watcher");
        let _ = Box::leak(Box::new(watcher));
    });
}