use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct CacheEntry {
    value: Arc<str>,
    created_at: Instant,
    access_count: u32,
}

struct AiResponseCache {
    entries: HashMap<String, CacheEntry>,
    // Vec + retain instead of a real LRU deque on purpose: entry count is
    // capped by `max_entries` (tens, not thousands), so the O(n) scan per
    // access is a few pointer compares and keeps the code simple.
    access_order: Vec<String>,
    max_entries: usize,
    max_bytes: usize,
    max_entry_bytes: usize,
    retained_bytes: usize,
    ttl: Duration,
}

impl AiResponseCache {
    fn new(max_entries: usize, ttl: Duration, max_bytes: usize, max_entry_bytes: usize) -> Self {
        Self {
            entries: HashMap::new(),
            max_entries,
            max_bytes,
            max_entry_bytes,
            retained_bytes: 0,
            ttl,
            access_order: Vec::new(),
        }
    }

    fn get(&mut self, key: &str) -> Option<Arc<str>> {
        let expired = self
            .entries
            .get(key)
            .is_some_and(|entry| entry.created_at.elapsed() >= self.ttl);
        if expired {
            self.remove(key);
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
        let entry_bytes = key.len().saturating_add(value.len());
        if self.max_entries == 0 || entry_bytes > self.max_bytes {
            self.remove(&key);
            return;
        }
        if entry_bytes > self.max_entry_bytes {
            // The oversized value cannot be cached, but an existing (smaller)
            // entry for this key is still valid — keep it instead of evicting
            // a perfectly good response.
            return;
        }
        self.remove(&key);
        while self.entries.len() >= self.max_entries
            || self.retained_bytes.saturating_add(entry_bytes) > self.max_bytes
        {
            if self.entries.is_empty() {
                break;
            }
            self.evict_lru();
        }
        self.access_order.push(key.clone());
        self.retained_bytes = self.retained_bytes.saturating_add(entry_bytes);
        self.entries.insert(
            key,
            CacheEntry {
                value: Arc::from(value),
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
            self.remove(&key);
        }
    }

    fn clear_all(&mut self) {
        self.entries.clear();
        self.access_order.clear();
        self.retained_bytes = 0;
    }

    fn cleanup_expired(&mut self) {
        let keys = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.created_at.elapsed() >= self.ttl)
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        for key in keys {
            self.remove(&key);
        }
    }

    fn evict_lru(&mut self) {
        if let Some(key) = self.access_order.first().cloned() {
            self.remove(&key);
        }
    }

    fn remove(&mut self, key: &str) {
        if let Some(entry) = self.entries.remove(key) {
            self.retained_bytes = self
                .retained_bytes
                .saturating_sub(key.len().saturating_add(entry.value.len()));
        }
        self.access_order.retain(|candidate| candidate != key);
    }
}

const DEFAULT_MAX_CACHE_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_MAX_ENTRY_BYTES: usize = 1024 * 1024;

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
        Self::with_limits(
            max_entries,
            ttl,
            DEFAULT_MAX_CACHE_BYTES,
            DEFAULT_MAX_ENTRY_BYTES,
        )
    }

    /// Construct a cache with explicit count and byte budgets.
    #[must_use]
    pub fn with_limits(
        max_entries: usize,
        ttl: Duration,
        max_bytes: usize,
        max_entry_bytes: usize,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(AiResponseCache::new(
                max_entries,
                ttl,
                max_bytes,
                max_entry_bytes,
            ))),
        }
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<String> {
        self.get_shared(key).map(|value| value.to_string())
    }

    /// Return the cached allocation itself for consumers that can retain a
    /// shared immutable response rather than cloning a potentially large
    /// string.
    #[must_use]
    pub fn get_shared(&self, key: &str) -> Option<Arc<str>> {
        self.inner.lock().ok().and_then(|mut cache| cache.get(key))
    }

    pub fn insert(&self, key: String, value: String) {
        if let Ok(mut cache) = self.inner.lock() {
            cache.insert(key, value);
        }
    }

    /// Approximate owned UTF-8 bytes retained by keys and response values.
    #[must_use]
    pub fn retained_bytes(&self) -> usize {
        self.inner
            .lock()
            .map(|cache| cache.retained_bytes)
            .unwrap_or_default()
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

    #[test]
    fn byte_budget_evicts_lru_entries() {
        let cache = SharedAiCache::with_limits(10, Duration::from_secs(60), 18, 18);
        cache.insert("a".to_string(), "1234567".to_string());
        cache.insert("b".to_string(), "1234567".to_string());
        assert_eq!(cache.retained_bytes(), 16);
        assert!(cache.get_shared("a").is_some());

        cache.insert("c".to_string(), "1234567".to_string());
        assert!(cache.get_shared("a").is_some());
        assert_eq!(cache.get("b"), None);
        assert_eq!(cache.get("c").as_deref(), Some("1234567"));
        assert!(cache.retained_bytes() <= 18);
    }

    #[test]
    fn oversized_entry_is_not_retained() {
        let cache = SharedAiCache::with_limits(10, Duration::from_secs(60), 100, 8);
        cache.insert("key".to_string(), "123456".to_string());
        assert_eq!(cache.get("key"), None);
        assert_eq!(cache.retained_bytes(), 0);
    }

    #[test]
    fn oversized_refresh_keeps_the_smaller_cached_value() {
        // Limits count key + value bytes: "key" (3) + value must stay within
        // max_entry_bytes = 16 for the initial insert.
        let cache = SharedAiCache::with_limits(10, Duration::from_secs(60), 100, 16);
        cache.insert("key".to_string(), "123456".to_string());
        assert_eq!(cache.get("key").as_deref(), Some("123456"));
        // A refresh that exceeds the per-entry limit (3 + 16 = 19 > 16) must
        // not evict the good cached response.
        cache.insert("key".to_string(), "1234567890123456".to_string());
        assert_eq!(cache.get("key").as_deref(), Some("123456"));
    }
}
