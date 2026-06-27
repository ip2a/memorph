use std::sync::{Arc, OnceLock};

use crate::cache::{CachePolicy, CacheStore};
use crate::core::compression::CompressionArchiveSummary;

pub struct CompressionArchivesCache {
    store: CacheStore<Option<String>, Vec<CompressionArchiveSummary>>,
}

impl CompressionArchivesCache {
    pub fn new(policy: CachePolicy) -> Self {
        Self {
            store: CacheStore::new(policy),
        }
    }

    pub fn get(&self, workspace: Option<&str>) -> Option<Vec<CompressionArchiveSummary>> {
        self.store.get_fresh(&workspace_key(workspace))
    }

    pub fn set(&self, workspace: Option<&str>, items: Vec<CompressionArchiveSummary>) {
        self.store.set(workspace_key(workspace), items);
    }

    pub fn invalidate_all(&self) {
        self.store.clear();
    }
}

static COMPRESSION_ARCHIVES_CACHE: OnceLock<Arc<CompressionArchivesCache>> = OnceLock::new();

pub fn compression_archives_cache() -> Arc<CompressionArchivesCache> {
    COMPRESSION_ARCHIVES_CACHE
        .get_or_init(|| Arc::new(CompressionArchivesCache::new(CachePolicy::ttl_seconds(5))))
        .clone()
}

fn workspace_key(workspace: Option<&str>) -> Option<String> {
    workspace
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
}
