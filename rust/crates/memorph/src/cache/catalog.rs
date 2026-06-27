use std::sync::{Arc, OnceLock};

use serde::Serialize;

use crate::cache::{CachePolicy, CacheStore};
use crate::providers::catalog::{ActiveTime, ProviderCatalog};

#[derive(Debug, Clone, Serialize)]
pub struct ProviderActiveInfo {
    pub provider_id: String,
    pub has_sessions: bool,
    pub active_time: ActiveTime,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderActiveCatalog {
    pub providers: Vec<ProviderActiveInfo>,
}

pub struct CatalogCache {
    store: CacheStore<Option<String>, ProviderCatalog>,
}

impl CatalogCache {
    pub fn new(policy: CachePolicy) -> Self {
        Self {
            store: CacheStore::new(policy),
        }
    }

    pub fn get(&self, workspace: Option<&str>) -> Option<ProviderCatalog> {
        self.store.get_fresh(&workspace_key(workspace))
    }

    pub fn set(&self, workspace: Option<&str>, catalog: ProviderCatalog) {
        self.store.set(workspace_key(workspace), catalog);
    }

    pub fn invalidate_all(&self) {
        self.store.clear();
    }
}

pub struct ActiveCatalogCache {
    store: CacheStore<Option<String>, ProviderActiveCatalog>,
}

impl ActiveCatalogCache {
    pub fn new(policy: CachePolicy) -> Self {
        Self {
            store: CacheStore::new(policy),
        }
    }

    pub fn get(&self, workspace: Option<&str>) -> Option<ProviderActiveCatalog> {
        self.store.get_fresh(&workspace_key(workspace))
    }

    pub fn set(&self, workspace: Option<&str>, catalog: ProviderActiveCatalog) {
        self.store.set(workspace_key(workspace), catalog);
    }

    pub fn invalidate_all(&self) {
        self.store.clear();
    }
}

static CATALOG_CACHE: OnceLock<Arc<CatalogCache>> = OnceLock::new();
static ACTIVE_CATALOG_CACHE: OnceLock<Arc<ActiveCatalogCache>> = OnceLock::new();

pub fn catalog_cache() -> Arc<CatalogCache> {
    CATALOG_CACHE
        .get_or_init(|| Arc::new(CatalogCache::new(CachePolicy::ttl_seconds(5))))
        .clone()
}

pub fn active_catalog_cache() -> Arc<ActiveCatalogCache> {
    ACTIVE_CATALOG_CACHE
        .get_or_init(|| Arc::new(ActiveCatalogCache::new(CachePolicy::ttl_seconds(15))))
        .clone()
}

pub fn invalidate_catalog_caches() {
    catalog_cache().invalidate_all();
    active_catalog_cache().invalidate_all();
}

fn workspace_key(workspace: Option<&str>) -> Option<String> {
    workspace
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
}
