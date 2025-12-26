//! AI Response Cache with LRU eviction and TTL
//!
//! Caches AI responses to avoid redundant API calls for the same diagnostic output.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Cache entry with value and metadata
struct CacheEntry {
    value: String,
    created_at: Instant,
    #[allow(dead_code)] // Tracked for potential future analytics
    access_count: u32,
}

/// LRU Cache with TTL for AI responses
pub struct AICache {
    entries: HashMap<String, CacheEntry>,
    max_entries: usize,
    ttl: Duration,
    access_order: Vec<String>,
}

impl AICache {
    /// Create a new cache with the specified maximum entries
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: HashMap::new(),
            max_entries,
            ttl: Duration::from_secs(3600), // 1 hour default TTL
            access_order: Vec::new(),
        }
    }

    /// Set the TTL for cache entries
    #[allow(dead_code)]
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    /// Get a cached value if it exists and hasn't expired
    pub fn get(&self, key: &str) -> Option<String> {
        self.entries.get(key).and_then(|entry| {
            if entry.created_at.elapsed() < self.ttl {
                Some(entry.value.clone())
            } else {
                None // Expired
            }
        })
    }

    /// Insert a value into the cache
    pub fn insert(&mut self, key: String, value: String) {
        // Remove expired entries first
        self.cleanup_expired();

        // If at capacity, remove least recently used
        if self.entries.len() >= self.max_entries && !self.entries.contains_key(&key) {
            self.evict_lru();
        }

        // Update access order
        self.access_order.retain(|k| k != &key);
        self.access_order.push(key.clone());

        // Insert or update entry
        self.entries.insert(
            key,
            CacheEntry {
                value,
                created_at: Instant::now(),
                access_count: 1,
            },
        );
    }

    /// Clear all entries for a specific session
    pub fn clear_session(&mut self, session_id: &str) {
        let prefix = format!("{}:", session_id);
        let keys_to_remove: Vec<String> = self
            .entries
            .keys()
            .filter(|k| k.starts_with(&prefix))
            .cloned()
            .collect();

        for key in keys_to_remove {
            self.entries.remove(&key);
            self.access_order.retain(|k| k != &key);
        }
    }

    /// Clear all cache entries
    pub fn clear_all(&mut self) {
        self.entries.clear();
        self.access_order.clear();
    }

    /// Get cache statistics
    #[allow(dead_code)]
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            entry_count: self.entries.len(),
            max_entries: self.max_entries,
        }
    }

    /// Remove expired entries
    fn cleanup_expired(&mut self) {
        let expired_keys: Vec<String> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.created_at.elapsed() >= self.ttl)
            .map(|(k, _)| k.clone())
            .collect();

        for key in expired_keys {
            self.entries.remove(&key);
            self.access_order.retain(|k| k != &key);
        }
    }

    /// Evict the least recently used entry
    fn evict_lru(&mut self) {
        if let Some(key) = self.access_order.first().cloned() {
            self.entries.remove(&key);
            self.access_order.remove(0);
        }
    }
}

/// Cache statistics
#[allow(dead_code)]
pub struct CacheStats {
    pub entry_count: usize,
    pub max_entries: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_insert_and_get() {
        let mut cache = AICache::new(10);
        cache.insert("key1".to_string(), "value1".to_string());
        assert_eq!(cache.get("key1"), Some("value1".to_string()));
    }

    #[test]
    fn test_cache_lru_eviction() {
        let mut cache = AICache::new(2);
        cache.insert("key1".to_string(), "value1".to_string());
        cache.insert("key2".to_string(), "value2".to_string());
        cache.insert("key3".to_string(), "value3".to_string());

        // key1 should be evicted
        assert_eq!(cache.get("key1"), None);
        assert_eq!(cache.get("key2"), Some("value2".to_string()));
        assert_eq!(cache.get("key3"), Some("value3".to_string()));
    }

    #[test]
    fn test_cache_session_clear() {
        let mut cache = AICache::new(10);
        cache.insert("session1:key1".to_string(), "value1".to_string());
        cache.insert("session1:key2".to_string(), "value2".to_string());
        cache.insert("session2:key1".to_string(), "value3".to_string());

        cache.clear_session("session1");

        assert_eq!(cache.get("session1:key1"), None);
        assert_eq!(cache.get("session1:key2"), None);
        assert_eq!(cache.get("session2:key1"), Some("value3".to_string()));
    }
}
