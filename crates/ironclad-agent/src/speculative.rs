use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::debug;

/// Prediction of a likely tool call based on conversation context.
#[derive(Debug, Clone)]
pub struct ToolPrediction {
    pub tool_name: String,
    pub predicted_params: serde_json::Value,
    pub confidence: f64,
}

/// Cache key for speculative results.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct SpeculationKey {
    pub tool_name: String,
    pub params_hash: u64,
}

impl SpeculationKey {
    pub fn new(tool_name: &str, params: &serde_json::Value) -> Self {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        params.to_string().hash(&mut hasher);
        Self {
            tool_name: tool_name.to_string(),
            params_hash: hasher.finish(),
        }
    }
}

/// Cached result from a speculative tool execution.
#[derive(Debug, Clone)]
pub struct SpeculativeResult {
    pub output: String,
    pub metadata: Option<serde_json::Value>,
    pub created_at: std::time::Instant,
}

/// Manages speculative execution of read-only tools.
/// Spawns tokio tasks to pre-fetch results for predicted tool calls.
#[derive(Debug)]
pub struct SpeculationCache {
    cache: Arc<Mutex<HashMap<SpeculationKey, SpeculativeResult>>>,
    max_concurrent: usize,
    active_count: Arc<std::sync::atomic::AtomicUsize>,
}

impl SpeculationCache {
    pub fn new(max_concurrent: usize) -> Self {
        Self {
            cache: Arc::new(Mutex::new(HashMap::new())),
            max_concurrent,
            active_count: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    /// Check if we have a cached result for the given tool + params.
    pub async fn get(
        &self,
        tool_name: &str,
        params: &serde_json::Value,
    ) -> Option<SpeculativeResult> {
        let key = SpeculationKey::new(tool_name, params);
        let cache = self.cache.lock().await;
        cache.get(&key).cloned()
    }

    /// Store a speculative result.
    pub async fn insert(
        &self,
        tool_name: &str,
        params: &serde_json::Value,
        result: SpeculativeResult,
    ) {
        let key = SpeculationKey::new(tool_name, params);
        let mut cache = self.cache.lock().await;
        cache.insert(key, result);
    }

    /// Clear all cached speculative results (called at turn completion).
    pub async fn clear(&self) {
        let mut cache = self.cache.lock().await;
        let count = cache.len();
        cache.clear();
        if count > 0 {
            debug!(cleared = count, "speculation cache cleared");
        }
    }

    /// Number of cached results.
    pub async fn size(&self) -> usize {
        self.cache.lock().await.len()
    }

    /// Whether we can spawn another speculative task.
    pub fn can_speculate(&self) -> bool {
        self.active_count.load(std::sync::atomic::Ordering::Relaxed) < self.max_concurrent
    }

    /// Try to reserve a speculation slot. Returns true if the slot was acquired.
    pub fn start_speculation(&self) -> bool {
        let prev = self
            .active_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if prev >= self.max_concurrent {
            self.active_count
                .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
            false
        } else {
            true
        }
    }

    /// Release a speculation slot.
    pub fn end_speculation(&self) {
        self.active_count
            .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn active_count(&self) -> usize {
        self.active_count.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// Predicts likely next tool calls based on conversation context.
pub struct ToolPredictor {
    min_confidence: f64,
}

impl ToolPredictor {
    pub fn new(min_confidence: f64) -> Self {
        Self { min_confidence }
    }

    /// Analyze recent tool call history to predict likely next calls.
    /// Uses pattern matching on sequential tool usage.
    pub fn predict(
        &self,
        recent_tools: &[String],
        available_tools: &[String],
    ) -> Vec<ToolPrediction> {
        let mut predictions = Vec::new();

        if recent_tools.is_empty() || available_tools.is_empty() {
            return predictions;
        }

        let last_tool = &recent_tools[recent_tools.len() - 1];

        let follow_ups = common_follow_ups(last_tool);
        for (follow_tool, confidence) in follow_ups {
            if confidence >= self.min_confidence && available_tools.contains(&follow_tool) {
                predictions.push(ToolPrediction {
                    tool_name: follow_tool,
                    predicted_params: serde_json::Value::Object(serde_json::Map::new()),
                    confidence,
                });
            }
        }

        // Repeated tool calls raise confidence that the same tool will be called again
        let repeat_count = recent_tools
            .iter()
            .rev()
            .take_while(|t| *t == last_tool)
            .count();
        if repeat_count >= 2 {
            let confidence = 0.6 + (repeat_count as f64 * 0.05).min(0.2);
            if confidence >= self.min_confidence
                && !predictions.iter().any(|p| p.tool_name == *last_tool)
            {
                predictions.push(ToolPrediction {
                    tool_name: last_tool.clone(),
                    predicted_params: serde_json::Value::Object(serde_json::Map::new()),
                    confidence,
                });
            }
        }

        predictions.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        predictions
    }
}

/// Known tool sequences that commonly follow each other.
fn common_follow_ups(tool_name: &str) -> Vec<(String, f64)> {
    match tool_name {
        "file_read" => vec![
            ("file_read".to_string(), 0.7),
            ("memory_search".to_string(), 0.4),
        ],
        "memory_search" => vec![
            ("memory_search".to_string(), 0.5),
            ("file_read".to_string(), 0.3),
        ],
        "http_get" => vec![("http_get".to_string(), 0.6)],
        "list_directory" => vec![
            ("file_read".to_string(), 0.7),
            ("list_directory".to_string(), 0.4),
        ],
        _ => Vec::new(),
    }
}

/// Only `Safe` tools are eligible for speculative pre-fetching — they have
/// no side effects and can be re-executed without consequence.
pub fn is_safe_for_speculation(risk: &ironclad_core::RiskLevel) -> bool {
    matches!(risk, ironclad_core::RiskLevel::Safe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speculation_key_hashing() {
        let key1 = SpeculationKey::new("file_read", &serde_json::json!({"path": "/tmp/a.txt"}));
        let key2 = SpeculationKey::new("file_read", &serde_json::json!({"path": "/tmp/a.txt"}));
        let key3 = SpeculationKey::new("file_read", &serde_json::json!({"path": "/tmp/b.txt"}));

        assert_eq!(key1, key2);
        assert_ne!(key1, key3);
    }

    #[tokio::test]
    async fn cache_insert_and_get() {
        let cache = SpeculationCache::new(4);
        let params = serde_json::json!({"path": "/tmp/test.txt"});

        cache
            .insert(
                "file_read",
                &params,
                SpeculativeResult {
                    output: "file contents".to_string(),
                    metadata: None,
                    created_at: std::time::Instant::now(),
                },
            )
            .await;

        let result = cache.get("file_read", &params).await;
        assert!(result.is_some());
        assert_eq!(result.unwrap().output, "file contents");
    }

    #[tokio::test]
    async fn cache_miss() {
        let cache = SpeculationCache::new(4);
        let params = serde_json::json!({"path": "/tmp/missing.txt"});
        let result = cache.get("file_read", &params).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn cache_clear() {
        let cache = SpeculationCache::new(4);
        let params = serde_json::json!({"key": "value"});
        cache
            .insert(
                "tool1",
                &params,
                SpeculativeResult {
                    output: "result".to_string(),
                    metadata: None,
                    created_at: std::time::Instant::now(),
                },
            )
            .await;

        assert_eq!(cache.size().await, 1);
        cache.clear().await;
        assert_eq!(cache.size().await, 0);
    }

    #[test]
    fn concurrency_limit() {
        let cache = SpeculationCache::new(2);
        assert!(cache.can_speculate());
        assert!(cache.start_speculation());
        assert!(cache.start_speculation());
        assert!(!cache.start_speculation());
        assert_eq!(cache.active_count(), 2);

        cache.end_speculation();
        assert!(cache.can_speculate());
        assert_eq!(cache.active_count(), 1);
    }

    #[test]
    fn predictor_no_history() {
        let predictor = ToolPredictor::new(0.3);
        let predictions = predictor.predict(&[], &["file_read".to_string()]);
        assert!(predictions.is_empty());
    }

    #[test]
    fn predictor_known_sequence() {
        let predictor = ToolPredictor::new(0.3);
        let recent = vec!["list_directory".to_string()];
        let available = vec!["file_read".to_string(), "list_directory".to_string()];
        let predictions = predictor.predict(&recent, &available);
        assert!(!predictions.is_empty());
        assert_eq!(predictions[0].tool_name, "file_read");
        assert!(predictions[0].confidence >= 0.7);
    }

    #[test]
    fn predictor_repeated_tool() {
        let predictor = ToolPredictor::new(0.3);
        let recent = vec![
            "file_read".to_string(),
            "file_read".to_string(),
            "file_read".to_string(),
        ];
        let available = vec!["file_read".to_string(), "memory_search".to_string()];
        let predictions = predictor.predict(&recent, &available);
        assert!(predictions.iter().any(|p| p.tool_name == "file_read"));
    }

    #[test]
    fn predictor_confidence_filter() {
        let predictor = ToolPredictor::new(0.9);
        let recent = vec!["memory_search".to_string()];
        let available = vec!["memory_search".to_string(), "file_read".to_string()];
        let predictions = predictor.predict(&recent, &available);
        assert!(predictions.is_empty() || predictions.iter().all(|p| p.confidence >= 0.9));
    }

    #[test]
    fn predictor_unavailable_tool_filtered() {
        let predictor = ToolPredictor::new(0.3);
        let recent = vec!["list_directory".to_string()];
        let available = vec!["memory_search".to_string()];
        let predictions = predictor.predict(&recent, &available);
        assert!(!predictions.iter().any(|p| p.tool_name == "file_read"));
    }

    #[test]
    fn safe_for_speculation() {
        assert!(is_safe_for_speculation(&ironclad_core::RiskLevel::Safe));
        assert!(!is_safe_for_speculation(&ironclad_core::RiskLevel::Caution));
        assert!(!is_safe_for_speculation(
            &ironclad_core::RiskLevel::Dangerous
        ));
        assert!(!is_safe_for_speculation(
            &ironclad_core::RiskLevel::Forbidden
        ));
    }

    #[test]
    fn predictions_sorted_by_confidence() {
        let predictor = ToolPredictor::new(0.3);
        let recent = vec!["list_directory".to_string()];
        let available = vec!["file_read".to_string(), "list_directory".to_string()];
        let predictions = predictor.predict(&recent, &available);
        for i in 1..predictions.len() {
            assert!(predictions[i - 1].confidence >= predictions[i].confidence);
        }
    }

    #[test]
    fn common_follow_ups_http_get() {
        // http_get follow-ups should predict another http_get
        let predictor = ToolPredictor::new(0.3);
        let recent = vec!["http_get".to_string()];
        let available = vec!["http_get".to_string()];
        let predictions = predictor.predict(&recent, &available);
        assert!(
            predictions.iter().any(|p| p.tool_name == "http_get"),
            "http_get should predict a follow-up http_get"
        );
    }

    #[test]
    fn common_follow_ups_unknown_tool_returns_empty() {
        // Unknown tool names produce no follow-up predictions (only repeat heuristic)
        let predictor = ToolPredictor::new(0.3);
        let recent = vec!["unknown_exotic_tool".to_string()];
        let available = vec!["unknown_exotic_tool".to_string(), "file_read".to_string()];
        let predictions = predictor.predict(&recent, &available);
        // No follow-ups for unknown tool, and only 1 call so no repeat heuristic
        assert!(
            predictions.is_empty(),
            "unknown tool with single call should produce no predictions"
        );
    }

    #[test]
    fn predict_empty_available_tools() {
        let predictor = ToolPredictor::new(0.3);
        let recent = vec!["file_read".to_string()];
        let predictions = predictor.predict(&recent, &[]);
        assert!(
            predictions.is_empty(),
            "no available tools means no predictions"
        );
    }

    #[test]
    fn predict_empty_recent_tools() {
        let predictor = ToolPredictor::new(0.3);
        let available = vec!["file_read".to_string()];
        let predictions = predictor.predict(&[], &available);
        assert!(
            predictions.is_empty(),
            "no recent tools means no predictions"
        );
    }

    #[test]
    fn start_speculation_exhaustion_and_recovery() {
        let cache = SpeculationCache::new(1);
        assert!(cache.start_speculation(), "first slot should succeed");
        assert!(!cache.start_speculation(), "second slot should fail");
        assert_eq!(
            cache.active_count(),
            1,
            "count should remain 1 after failed attempt"
        );
        cache.end_speculation();
        assert_eq!(cache.active_count(), 0);
        assert!(cache.start_speculation(), "slot should be available again");
    }

    #[test]
    fn memory_search_follow_ups() {
        let predictor = ToolPredictor::new(0.3);
        let recent = vec!["memory_search".to_string()];
        let available = vec!["memory_search".to_string(), "file_read".to_string()];
        let predictions = predictor.predict(&recent, &available);
        assert!(
            predictions.iter().any(|p| p.tool_name == "memory_search"),
            "memory_search should predict memory_search follow-up"
        );
    }

    #[test]
    fn repeated_tool_no_duplicate_with_follow_up() {
        // file_read repeated 3 times: follow-up includes file_read (0.7),
        // repeat heuristic should not add a duplicate
        let predictor = ToolPredictor::new(0.3);
        let recent = vec![
            "file_read".to_string(),
            "file_read".to_string(),
            "file_read".to_string(),
        ];
        let available = vec!["file_read".to_string(), "memory_search".to_string()];
        let predictions = predictor.predict(&recent, &available);
        let file_read_count = predictions
            .iter()
            .filter(|p| p.tool_name == "file_read")
            .count();
        assert_eq!(
            file_read_count, 1,
            "file_read should appear exactly once (no duplicate from repeat heuristic)"
        );
    }

    #[tokio::test]
    async fn cache_different_tools_same_params() {
        let cache = SpeculationCache::new(4);
        let params = serde_json::json!({"path": "/tmp/test.txt"});
        cache
            .insert(
                "file_read",
                &params,
                SpeculativeResult {
                    output: "read result".to_string(),
                    metadata: None,
                    created_at: std::time::Instant::now(),
                },
            )
            .await;
        cache
            .insert(
                "file_write",
                &params,
                SpeculativeResult {
                    output: "write result".to_string(),
                    metadata: None,
                    created_at: std::time::Instant::now(),
                },
            )
            .await;
        assert_eq!(cache.size().await, 2);
        let read_result = cache.get("file_read", &params).await.unwrap();
        assert_eq!(read_result.output, "read result");
        let write_result = cache.get("file_write", &params).await.unwrap();
        assert_eq!(write_result.output, "write result");
    }

    #[test]
    fn speculation_key_different_tool_names() {
        let params = serde_json::json!({"key": "value"});
        let key1 = SpeculationKey::new("tool_a", &params);
        let key2 = SpeculationKey::new("tool_b", &params);
        assert_ne!(
            key1, key2,
            "different tool names should produce different keys"
        );
    }

    #[test]
    fn speculative_result_metadata() {
        let result = SpeculativeResult {
            output: "data".to_string(),
            metadata: Some(serde_json::json!({"source": "cache"})),
            created_at: std::time::Instant::now(),
        };
        assert_eq!(result.metadata.unwrap()["source"], "cache");
    }
}
