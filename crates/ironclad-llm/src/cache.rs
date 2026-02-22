use std::collections::HashMap;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub struct CachedResponse {
    pub content: String,
    pub model: String,
    pub tokens_saved: u32,
    pub created_at: Instant,
    pub expires_at: Instant,
    pub hits: u32,
    pub involved_tools: bool,
    pub embedding: Option<Vec<f32>>,
}

#[derive(Debug)]
pub struct SemanticCache {
    enabled: bool,
    ttl: Duration,
    tool_ttl: Duration,
    max_entries: usize,
    similarity_threshold: f32,
    entries: HashMap<String, CachedResponse>,
    hit_count: usize,
    miss_count: usize,
}

impl SemanticCache {
    pub fn new(enabled: bool, ttl_seconds: u64, max_entries: usize) -> Self {
        Self {
            enabled,
            ttl: Duration::from_secs(ttl_seconds),
            tool_ttl: Duration::from_secs(ttl_seconds / 4),
            max_entries,
            similarity_threshold: 0.85,
            entries: HashMap::new(),
            hit_count: 0,
            miss_count: 0,
        }
    }

    /// L1: exact hash match.
    pub fn lookup_exact(&mut self, prompt_hash: &str) -> Option<CachedResponse> {
        if !self.enabled {
            self.miss_count += 1;
            return None;
        }

        if let Some(entry) = self.entries.get_mut(prompt_hash)
            && Instant::now() < entry.expires_at
        {
            entry.hits += 1;
            self.hit_count += 1;
            return Some(entry.clone());
        }

        self.miss_count += 1;
        None
    }

    /// L2: semantic embedding similarity lookup.
    /// Computes a lightweight character n-gram embedding of the prompt and searches
    /// all cache entries for the closest match above `similarity_threshold`.
    pub fn lookup_semantic(&mut self, prompt: &str) -> Option<CachedResponse> {
        if !self.enabled {
            return None;
        }

        let query_emb = compute_ngram_embedding(prompt);
        let now = Instant::now();

        let mut best_match: Option<(&str, f32)> = None;

        for (key, entry) in &self.entries {
            if now >= entry.expires_at {
                continue;
            }
            if let Some(ref emb) = entry.embedding {
                let sim = cosine_similarity(&query_emb, emb);
                if sim >= self.similarity_threshold
                    && best_match
                        .as_ref()
                        .is_none_or(|(_, best_sim)| sim > *best_sim)
                {
                    best_match = Some((key, sim));
                }
            }
        }

        if let Some((key, _)) = best_match {
            let key = key.to_string();
            if let Some(entry) = self.entries.get_mut(&key) {
                entry.hits += 1;
                self.hit_count += 1;
                return Some(entry.clone());
            }
        }

        self.miss_count += 1;
        None
    }

    /// L3: tool-aware TTL lookup.
    /// Entries that involved tool calls get a shorter TTL (tool_ttl) since
    /// tool-dependent responses are more likely to become stale.
    pub fn lookup_tool_ttl(&mut self, prompt_hash: &str) -> Option<CachedResponse> {
        if !self.enabled {
            return None;
        }

        if let Some(entry) = self.entries.get_mut(prompt_hash) {
            let effective_ttl = if entry.involved_tools {
                entry.created_at + self.tool_ttl
            } else {
                entry.expires_at
            };

            if Instant::now() < effective_ttl {
                entry.hits += 1;
                self.hit_count += 1;
                return Some(entry.clone());
            }
        }

        None
    }

    /// Multi-level lookup: L1 exact -> L3 tool-TTL -> L2 semantic.
    pub fn lookup(&mut self, prompt_hash: &str, prompt_text: &str) -> Option<CachedResponse> {
        if let Some(hit) = self.lookup_exact(prompt_hash) {
            return Some(hit);
        }
        if let Some(hit) = self.lookup_tool_ttl(prompt_hash) {
            return Some(hit);
        }
        self.lookup_semantic(prompt_text)
    }

    pub fn store(&mut self, prompt_hash: &str, response: CachedResponse) {
        if !self.enabled {
            return;
        }

        if self.entries.len() >= self.max_entries {
            self.evict_lfu();
        }

        let now = Instant::now();
        let entry = CachedResponse {
            created_at: now,
            expires_at: now + self.ttl,
            hits: 0,
            ..response
        };
        self.entries.insert(prompt_hash.to_string(), entry);
    }

    /// Store with an explicit embedding for L2 lookups.
    pub fn store_with_embedding(
        &mut self,
        prompt_hash: &str,
        prompt_text: &str,
        mut response: CachedResponse,
    ) {
        response.embedding = Some(compute_ngram_embedding(prompt_text));
        self.store(prompt_hash, response);
    }

    pub fn compute_hash(system: &str, messages: &str, user_msg: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(system.as_bytes());
        hasher.update(b"|");
        hasher.update(messages.as_bytes());
        hasher.update(b"|");
        hasher.update(user_msg.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    pub fn evict_expired(&mut self) {
        let now = Instant::now();
        self.entries.retain(|_, v| v.expires_at > now);
    }

    /// Remove the entry with the fewest hits (LFU) to stay within `max_entries`.
    pub fn evict_lfu(&mut self) {
        if let Some(key) = self
            .entries
            .iter()
            .min_by_key(|(_, v)| v.hits)
            .map(|(k, _)| k.clone())
        {
            self.entries.remove(&key);
        }
    }

    pub fn hit_count(&self) -> usize {
        self.hit_count
    }

    pub fn miss_count(&self) -> usize {
        self.miss_count
    }

    pub fn size(&self) -> usize {
        self.entries.len()
    }
}

const NGRAM_DIM: usize = 128;

/// Lightweight character 3-gram embedding into a fixed-size vector.
fn compute_ngram_embedding(text: &str) -> Vec<f32> {
    let mut vec = vec![0.0f32; NGRAM_DIM];
    let lower = text.to_lowercase();
    let chars: Vec<char> = lower.chars().collect();
    if chars.len() < 3 {
        return vec;
    }
    for window in chars.windows(3) {
        let hash = window
            .iter()
            .fold(0u32, |acc, &c| acc.wrapping_mul(31).wrapping_add(c as u32));
        vec[(hash as usize) % NGRAM_DIM] += 1.0;
    }
    let norm = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for v in &mut vec {
            *v /= norm;
        }
    }
    vec
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn cosine_self_similarity_is_one(v in proptest::collection::vec(-1.0f32..1.0, 8..32)) {
            let sim = cosine_similarity(&v, &v);
            prop_assert!((sim - 1.0).abs() < 0.001);
        }
    }

    fn make_response(content: &str) -> CachedResponse {
        let now = Instant::now();
        CachedResponse {
            content: content.into(),
            model: "test-model".into(),
            tokens_saved: 100,
            created_at: now,
            expires_at: now + Duration::from_secs(3600),
            hits: 0,
            involved_tools: false,
            embedding: None,
        }
    }

    fn make_tool_response(content: &str) -> CachedResponse {
        let mut r = make_response(content);
        r.involved_tools = true;
        r
    }

    #[test]
    fn store_and_exact_hit() {
        let mut cache = SemanticCache::new(true, 3600, 100);
        let hash = SemanticCache::compute_hash("sys", "msgs", "hello");

        cache.store(&hash, make_response("world"));
        let result = cache.lookup_exact(&hash);
        assert!(result.is_some());
        assert_eq!(result.unwrap().content, "world");
        assert_eq!(cache.hit_count(), 1);
        assert_eq!(cache.size(), 1);
    }

    #[test]
    fn miss_for_unknown_hash() {
        let mut cache = SemanticCache::new(true, 3600, 100);
        let result = cache.lookup_exact("nonexistent_hash");
        assert!(result.is_none());
        assert_eq!(cache.miss_count(), 1);
    }

    #[test]
    fn expiration_eviction() {
        let mut cache = SemanticCache::new(true, 0, 100);
        let hash = SemanticCache::compute_hash("sys", "msgs", "expire-me");

        cache.store(&hash, make_response("ephemeral"));
        std::thread::sleep(Duration::from_millis(5));
        cache.evict_expired();
        assert_eq!(cache.size(), 0);
    }

    #[test]
    fn lfu_eviction_at_capacity() {
        let mut cache = SemanticCache::new(true, 3600, 2);

        let h1 = "hash_1".to_string();
        let h2 = "hash_2".to_string();
        let h3 = "hash_3".to_string();

        cache.store(&h1, make_response("first"));
        cache.store(&h2, make_response("second"));

        cache.lookup_exact(&h2);

        cache.store(&h3, make_response("third"));
        assert_eq!(cache.size(), 2);
        assert!(cache.lookup_exact(&h1).is_none());
        assert!(cache.lookup_exact(&h2).is_some());
    }

    #[test]
    fn semantic_similarity_finds_near_matches() {
        let mut cache = SemanticCache::new(true, 3600, 100);
        let prompt1 = "What is the capital city of France?";
        let hash1 = SemanticCache::compute_hash("sys", "", prompt1);

        cache.store_with_embedding(&hash1, prompt1, make_response("Paris"));

        let similar_prompt = "What is the capital of France?";
        let result = cache.lookup_semantic(similar_prompt);
        assert!(result.is_some(), "semantically similar prompt should hit");
        assert_eq!(result.unwrap().content, "Paris");
    }

    #[test]
    fn semantic_dissimilar_miss() {
        let mut cache = SemanticCache::new(true, 3600, 100);
        let prompt1 = "What is the capital city of France?";
        let hash1 = SemanticCache::compute_hash("sys", "", prompt1);

        cache.store_with_embedding(&hash1, prompt1, make_response("Paris"));

        let different_prompt = "How do quantum computers work in detail?";
        let result = cache.lookup_semantic(different_prompt);
        assert!(result.is_none(), "dissimilar prompt should miss");
    }

    #[test]
    fn tool_ttl_shorter_than_normal() {
        let mut cache = SemanticCache::new(true, 100, 100);

        let hash = "tool_hash";
        cache.store(hash, make_tool_response("tool result"));

        let hit = cache.lookup_tool_ttl(hash);
        assert!(hit.is_some(), "fresh tool entry should hit");

        let non_tool_hash = "normal_hash";
        cache.store(non_tool_hash, make_response("normal result"));
        let hit = cache.lookup_tool_ttl(non_tool_hash);
        assert!(
            hit.is_some(),
            "fresh non-tool entry should hit via tool_ttl"
        );
    }

    #[test]
    fn multi_level_lookup_prefers_exact() {
        let mut cache = SemanticCache::new(true, 3600, 100);
        let prompt = "hello world test prompt";
        let hash = SemanticCache::compute_hash("sys", "", prompt);

        cache.store_with_embedding(&hash, prompt, make_response("exact match"));

        let result = cache.lookup(&hash, prompt);
        assert!(result.is_some());
        assert_eq!(result.unwrap().content, "exact match");
    }

    #[test]
    fn ngram_embedding_properties() {
        let emb1 = compute_ngram_embedding("hello world");
        let emb2 = compute_ngram_embedding("hello world");
        assert_eq!(emb1, emb2, "same text should produce identical embeddings");

        let emb3 = compute_ngram_embedding("completely different text");
        assert_ne!(emb1, emb3);

        let norm: f32 = emb1.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 0.01,
            "embedding should be unit-normalized"
        );
    }

    #[test]
    fn cosine_similarity_properties() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert!((cosine_similarity(&a, &b) - 1.0).abs() < f64::EPSILON as f32);

        let c = vec![0.0, 1.0, 0.0];
        assert!(cosine_similarity(&a, &c).abs() < f64::EPSILON as f32);
    }

    // 9C: Edge cases — 0 capacity, duplicate keys
    #[test]
    fn cache_zero_capacity_still_stores_one() {
        let mut cache = SemanticCache::new(true, 3600, 0);
        let hash = SemanticCache::compute_hash("", "", "q");
        cache.store(&hash, make_response("a"));
        assert_eq!(cache.size(), 1);
        let hit = cache.lookup_exact(&hash);
        assert!(hit.is_some());
        assert_eq!(hit.unwrap().content, "a");
    }

    #[test]
    fn cache_duplicate_key_overwrites() {
        let mut cache = SemanticCache::new(true, 3600, 10);
        let hash = "dup_key".to_string();
        cache.store(&hash, make_response("first"));
        cache.store(&hash, make_response("second"));
        assert_eq!(cache.size(), 1);
        let hit = cache.lookup_exact(&hash);
        assert!(hit.is_some());
        assert_eq!(hit.unwrap().content, "second");
    }
}
