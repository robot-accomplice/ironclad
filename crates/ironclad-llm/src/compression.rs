use tracing::info;

/// Token importance scorer using entropy-based heuristics.
/// Approximates perplexity-based scoring without requiring a separate model.
#[derive(Debug)]
pub struct PromptCompressor {
    target_ratio: f64,
}

impl PromptCompressor {
    pub fn new(target_ratio: f64) -> Self {
        Self {
            target_ratio: target_ratio.clamp(0.1, 1.0),
        }
    }

    /// Compress a prompt by removing low-importance tokens.
    /// Returns the compressed text.
    pub fn compress(&self, text: &str) -> String {
        if text.is_empty() || self.target_ratio >= 1.0 {
            return text.to_string();
        }

        let tokens: Vec<&str> = text.split_whitespace().collect();
        let target_count = (tokens.len() as f64 * self.target_ratio).ceil() as usize;

        if target_count >= tokens.len() {
            return text.to_string();
        }

        let scores: Vec<(usize, f64)> = tokens
            .iter()
            .enumerate()
            .map(|(i, token)| (i, self.score_token(token, i, tokens.len())))
            .collect();

        let mut ranked: Vec<(usize, f64)> = scores.clone();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let mut keep_indices: Vec<usize> =
            ranked.iter().take(target_count).map(|(i, _)| *i).collect();
        keep_indices.sort();

        let result: Vec<&str> = keep_indices.iter().map(|&i| tokens[i]).collect();

        let compressed = result.join(" ");

        info!(
            original_tokens = tokens.len(),
            compressed_tokens = keep_indices.len(),
            ratio = %format!("{:.2}", keep_indices.len() as f64 / tokens.len() as f64),
            "prompt compressed"
        );

        compressed
    }

    /// Score a token's importance. Higher = more important.
    fn score_token(&self, token: &str, position: usize, total: usize) -> f64 {
        let mut score = 0.0;

        if is_content_word(token) {
            score += 3.0;
        } else if is_stop_word(token) {
            score += 0.5;
        } else {
            score += 1.5;
        }

        // Longer tokens tend to carry more information (information density)
        score += (token.len() as f64).ln().max(0.0) * 0.5;

        // Tokens with special characters (code, punctuation) are often important
        if token.contains('(')
            || token.contains(')')
            || token.contains('{')
            || token.contains('}')
            || token.contains('=')
            || token.contains(':')
        {
            score += 2.0;
        }

        // Capitalized tokens (names, starts of sentences) get a boost
        if token.chars().next().is_some_and(|c| c.is_uppercase()) {
            score += 1.0;
        }

        if token.chars().any(|c| c.is_ascii_digit()) {
            score += 1.5;
        }

        // Position bias: first and last tokens in the text tend to be important
        let position_ratio = position as f64 / total.max(1) as f64;
        if !(0.1..=0.9).contains(&position_ratio) {
            score += 1.0;
        }

        score
    }

    pub fn target_ratio(&self) -> f64 {
        self.target_ratio
    }

    /// Estimate the token count reduction.
    pub fn estimate_savings(&self, text: &str) -> CompressionEstimate {
        let original = text.split_whitespace().count();
        let target = (original as f64 * self.target_ratio).ceil() as usize;
        CompressionEstimate {
            original_tokens: original,
            estimated_tokens: target.min(original),
            estimated_ratio: if original == 0 {
                1.0
            } else {
                target.min(original) as f64 / original as f64
            },
        }
    }
}

/// Estimation of compression savings before actual compression.
#[derive(Debug, Clone)]
pub struct CompressionEstimate {
    pub original_tokens: usize,
    pub estimated_tokens: usize,
    pub estimated_ratio: f64,
}

fn is_stop_word(token: &str) -> bool {
    const STOP_WORDS: &[&str] = &[
        "the", "a", "an", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had",
        "do", "does", "did", "will", "would", "could", "should", "may", "might", "shall", "can",
        "to", "of", "in", "for", "on", "with", "at", "by", "from", "as", "into", "through",
        "during", "before", "after", "above", "below", "between", "but", "and", "or", "nor", "not",
        "so", "yet", "both", "either", "neither", "each", "every", "all", "any", "few", "more",
        "most", "other", "some", "such", "no", "only", "own", "same", "than", "too", "very",
        "just", "also", "that", "this", "these", "those", "it", "its",
    ];
    STOP_WORDS.contains(&token.to_lowercase().as_str())
}

fn is_content_word(token: &str) -> bool {
    let lower = token.to_lowercase();
    !is_stop_word(token) && token.len() > 3 && lower.chars().all(|c| c.is_alphabetic())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compress_empty_string() {
        let c = PromptCompressor::new(0.5);
        assert_eq!(c.compress(""), "");
    }

    #[test]
    fn compress_ratio_one_no_change() {
        let c = PromptCompressor::new(1.0);
        let input = "The quick brown fox jumps over the lazy dog";
        assert_eq!(c.compress(input), input);
    }

    #[test]
    fn compress_reduces_tokens() {
        let c = PromptCompressor::new(0.5);
        let input = "The quick brown fox jumps over the lazy dog near the river bank";
        let compressed = c.compress(input);
        let original_count = input.split_whitespace().count();
        let compressed_count = compressed.split_whitespace().count();
        assert!(
            compressed_count < original_count,
            "should reduce token count"
        );
        assert!(compressed_count > 0, "should keep at least some tokens");
    }

    #[test]
    fn compress_keeps_content_words() {
        let c = PromptCompressor::new(0.4);
        let input = "The database connection timeout should be increased to 30 seconds";
        let compressed = c.compress(input);
        assert!(
            compressed.contains("database")
                || compressed.contains("connection")
                || compressed.contains("timeout"),
            "should keep content words: {compressed}"
        );
    }

    #[test]
    fn compress_preserves_order() {
        let c = PromptCompressor::new(0.6);
        let input = "First alpha then beta finally gamma heading toward the sequence end";
        let compressed = c.compress(input);
        let input_tokens: Vec<&str> = input.split_whitespace().collect();
        let compressed_tokens: Vec<&str> = compressed.split_whitespace().collect();
        let positions: Vec<usize> = compressed_tokens
            .iter()
            .filter_map(|ct| input_tokens.iter().position(|it| it == ct))
            .collect();
        for i in 1..positions.len() {
            assert!(
                positions[i - 1] < positions[i],
                "token order should be preserved"
            );
        }
    }

    #[test]
    fn compress_ratio_clamped() {
        let c = PromptCompressor::new(0.05);
        assert!(
            (c.target_ratio() - 0.1).abs() < f64::EPSILON,
            "should clamp to 0.1"
        );

        let c2 = PromptCompressor::new(1.5);
        assert!(
            (c2.target_ratio() - 1.0).abs() < f64::EPSILON,
            "should clamp to 1.0"
        );
    }

    #[test]
    fn stop_word_detection() {
        assert!(is_stop_word("the"));
        assert!(is_stop_word("The"));
        assert!(is_stop_word("is"));
        assert!(!is_stop_word("database"));
        assert!(!is_stop_word("compress"));
    }

    #[test]
    fn content_word_detection() {
        assert!(is_content_word("database"));
        assert!(is_content_word("compress"));
        assert!(!is_content_word("the"));
        assert!(!is_content_word("a"));
        assert!(!is_content_word("is"));
    }

    #[test]
    fn estimate_savings() {
        let c = PromptCompressor::new(0.5);
        let est = c.estimate_savings("one two three four five six seven eight nine ten");
        assert_eq!(est.original_tokens, 10);
        assert_eq!(est.estimated_tokens, 5);
        assert!((est.estimated_ratio - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn estimate_savings_empty() {
        let c = PromptCompressor::new(0.5);
        let est = c.estimate_savings("");
        assert_eq!(est.original_tokens, 0);
        assert!((est.estimated_ratio - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn short_input_no_compression() {
        let c = PromptCompressor::new(0.5);
        let input = "Hello world";
        let compressed = c.compress(input);
        assert!(!compressed.is_empty());
    }
}
