use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct CacheEntry {
    value: String,
    created_at: Instant,
    access_count: u32,
}

struct AiResponseCache {
    entries: HashMap<String, CacheEntry>,
    max_entries: usize,
    ttl: Duration,
    access_order: Vec<String>,
}

impl AiResponseCache {
    fn new(max_entries: usize, ttl: Duration) -> Self {
        Self {
            entries: HashMap::new(),
            max_entries,
            ttl,
            access_order: Vec::new(),
        }
    }

    fn get(&mut self, key: &str) -> Option<String> {
        let expired = self
            .entries
            .get(key)
            .is_some_and(|entry| entry.created_at.elapsed() >= self.ttl);
        if expired {
            self.entries.remove(key);
            self.access_order.retain(|candidate| candidate != key);
            return None;
        }

        let value = {
            let entry = self.entries.get_mut(key)?;
            entry.access_count = entry.access_count.saturating_add(1);
            entry.value.clone()
        };
        self.access_order.retain(|candidate| candidate != key);
        self.access_order.push(key.to_string());
        Some(value)
    }

    fn insert(&mut self, key: String, value: String) {
        self.cleanup_expired();
        if self.entries.len() >= self.max_entries && !self.entries.contains_key(&key) {
            self.evict_lru();
        }
        self.access_order.retain(|candidate| candidate != &key);
        self.access_order.push(key.clone());
        self.entries.insert(
            key,
            CacheEntry {
                value,
                created_at: Instant::now(),
                access_count: 1,
            },
        );
    }

    fn clear_session(&mut self, session_id: &str) {
        let prefix = format!("{session_id}:");
        let keys = self
            .entries
            .keys()
            .filter(|key| key.starts_with(&prefix))
            .cloned()
            .collect::<Vec<_>>();
        for key in keys {
            self.entries.remove(&key);
            self.access_order.retain(|candidate| candidate != &key);
        }
    }

    fn clear_all(&mut self) {
        self.entries.clear();
        self.access_order.clear();
    }

    fn cleanup_expired(&mut self) {
        let keys = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.created_at.elapsed() >= self.ttl)
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        for key in keys {
            self.entries.remove(&key);
            self.access_order.retain(|candidate| candidate != &key);
        }
    }

    fn evict_lru(&mut self) {
        if let Some(key) = self.access_order.first().cloned() {
            self.entries.remove(&key);
            self.access_order.remove(0);
        }
    }
}

/// Cache-clearing boundary consumed by provider management.
pub trait ProviderCacheControl: Send + Sync + 'static {
    fn clear(&self, session_id: Option<&str>);
}

/// Cloneable response cache shared by provider management, analysis, and
/// report generation in either UI shell.
#[derive(Clone)]
pub struct SharedAiCache {
    inner: Arc<Mutex<AiResponseCache>>,
}

impl fmt::Debug for SharedAiCache {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SharedAiCache")
            .finish_non_exhaustive()
    }
}

impl SharedAiCache {
    /// Construct the shipping one-hour cache.
    #[must_use]
    pub fn new(max_entries: usize) -> Self {
        Self::with_ttl(max_entries, Duration::from_secs(3_600))
    }

    /// Construct a cache with an explicit TTL, primarily for deterministic
    /// tests and native-shell composition.
    #[must_use]
    pub fn with_ttl(max_entries: usize, ttl: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(AiResponseCache::new(max_entries, ttl))),
        }
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<String> {
        self.inner.lock().ok().and_then(|mut cache| cache.get(key))
    }

    pub fn insert(&self, key: String, value: String) {
        if let Ok(mut cache) = self.inner.lock() {
            cache.insert(key, value);
        }
    }
}

impl ProviderCacheControl for SharedAiCache {
    fn clear(&self, session_id: Option<&str>) {
        if let Ok(mut cache) = self.inner.lock() {
            if let Some(session_id) = session_id {
                cache.clear_session(session_id);
            } else {
                cache.clear_all();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_get_and_lru_eviction_match_shipping_cache() {
        let cache = SharedAiCache::new(2);
        cache.insert("key1".to_string(), "value1".to_string());
        cache.insert("key2".to_string(), "value2".to_string());
        assert_eq!(cache.get("key1").as_deref(), Some("value1"));
        cache.insert("key3".to_string(), "value3".to_string());
        assert_eq!(cache.get("key1").as_deref(), Some("value1"));
        assert_eq!(cache.get("key2"), None);
        assert_eq!(cache.get("key3").as_deref(), Some("value3"));
    }

    #[test]
    fn session_and_global_clear_are_exact() {
        let cache = SharedAiCache::new(10);
        cache.insert("session1:key1".to_string(), "one".to_string());
        cache.insert("session2:key1".to_string(), "two".to_string());
        cache.clear(Some("session1"));
        assert_eq!(cache.get("session1:key1"), None);
        assert_eq!(cache.get("session2:key1").as_deref(), Some("two"));
        cache.clear(None);
        assert_eq!(cache.get("session2:key1"), None);
    }

    #[test]
    fn expired_entries_are_not_returned() {
        let cache = SharedAiCache::with_ttl(2, Duration::ZERO);
        cache.insert("key".to_string(), "value".to_string());
        assert_eq!(cache.get("key"), None);
    }
}
