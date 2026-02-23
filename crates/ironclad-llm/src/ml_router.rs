use std::path::Path;
use tracing::{debug, info};

use crate::router::RouterBackend;

/// Logistic regression classifier for model routing.
/// Trained on preference data: which model produced better answers for given features.
/// Produces a complexity score in [0.0, 1.0] — low means a weak/local model suffices,
/// high means a strong/cloud model is needed.
#[derive(Debug, Clone)]
pub struct LogisticBackend {
    weights: Vec<f64>,
    bias: f64,
}

impl LogisticBackend {
    pub fn new(weights: Vec<f64>, bias: f64) -> Self {
        Self { weights, bias }
    }

    /// Load a trained model from a simple text format.
    /// Format: first line is bias, subsequent lines are weights (one per line).
    pub fn from_file(path: &Path) -> std::io::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let mut lines = content.lines();

        let bias: f64 = lines
            .next()
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, "missing bias line")
            })?
            .trim()
            .parse()
            .map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("invalid bias: {e}"),
                )
            })?;

        let weights: Vec<f64> = lines
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.trim().parse::<f64>())
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("invalid weight: {e}"),
                )
            })?;

        info!(weights = weights.len(), bias, "loaded ML router model");

        Ok(Self { weights, bias })
    }

    /// Serialize the model to a simple text format.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let mut content = format!("{}\n", self.bias);
        for w in &self.weights {
            content.push_str(&format!("{w}\n"));
        }
        std::fs::write(path, content)
    }

    fn sigmoid(x: f64) -> f64 {
        1.0 / (1.0 + (-x).exp())
    }

    fn logit(&self, features: &[f32]) -> f64 {
        let mut z = self.bias;
        for (w, f) in self.weights.iter().zip(features.iter()) {
            z += w * (*f as f64);
        }
        z
    }

    pub fn weights(&self) -> &[f64] {
        &self.weights
    }

    pub fn bias(&self) -> f64 {
        self.bias
    }

    /// Train on a batch of labeled examples using gradient descent.
    /// Each example is (features, label) where label is 0.0 (weak model ok) or 1.0 (strong model needed).
    pub fn train(&mut self, examples: &[(Vec<f32>, f64)], learning_rate: f64, epochs: usize) {
        if examples.is_empty() {
            return;
        }

        let feature_dim = examples[0].0.len();
        if self.weights.len() != feature_dim {
            self.weights = vec![0.0; feature_dim];
        }

        for epoch in 0..epochs {
            let mut total_loss = 0.0;

            for (features, label) in examples {
                let prediction = Self::sigmoid(self.logit(features));
                let error = prediction - label;

                self.bias -= learning_rate * error;
                for (i, f) in features.iter().enumerate() {
                    if i < self.weights.len() {
                        self.weights[i] -= learning_rate * error * (*f as f64);
                    }
                }

                let clamped = prediction.clamp(1e-10, 1.0 - 1e-10);
                total_loss += -label * clamped.ln() - (1.0 - label) * (1.0 - clamped).ln();
            }

            if epoch % 100 == 0 || epoch == epochs - 1 {
                debug!(
                    epoch,
                    avg_loss = total_loss / examples.len() as f64,
                    "training progress"
                );
            }
        }
    }
}

impl RouterBackend for LogisticBackend {
    fn classify_complexity(&self, features: &[f32]) -> f64 {
        let score = Self::sigmoid(self.logit(features));
        debug!(
            features = ?features,
            score,
            "ML complexity classification"
        );
        score
    }
}

/// Preference data point for training: which model won for a given query.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PreferenceRecord {
    pub features: Vec<f32>,
    pub strong_model_won: bool,
}

impl PreferenceRecord {
    pub fn to_training_example(&self) -> (Vec<f32>, f64) {
        (
            self.features.clone(),
            if self.strong_model_won { 1.0 } else { 0.0 },
        )
    }
}

/// Collects preference data for incremental training.
#[derive(Debug, Default)]
pub struct PreferenceCollector {
    records: Vec<PreferenceRecord>,
}

impl PreferenceCollector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, features: Vec<f32>, strong_model_won: bool) {
        self.records.push(PreferenceRecord {
            features,
            strong_model_won,
        });
    }

    pub fn examples(&self) -> Vec<(Vec<f32>, f64)> {
        self.records
            .iter()
            .map(|r| r.to_training_example())
            .collect()
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn clear(&mut self) {
        self.records.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn sigmoid_bounds() {
        assert!((LogisticBackend::sigmoid(0.0) - 0.5).abs() < f64::EPSILON);
        assert!(LogisticBackend::sigmoid(10.0) > 0.99);
        assert!(LogisticBackend::sigmoid(-10.0) < 0.01);
    }

    #[test]
    fn classify_with_zero_weights() {
        let backend = LogisticBackend::new(vec![0.0, 0.0, 0.0], 0.0);
        let score = backend.classify_complexity(&[100.0, 5.0, 10.0]);
        assert!(
            (score - 0.5).abs() < f64::EPSILON,
            "zero weights + zero bias = 0.5"
        );
    }

    #[test]
    fn classify_positive_bias() {
        let backend = LogisticBackend::new(vec![0.0, 0.0, 0.0], 5.0);
        let score = backend.classify_complexity(&[0.0, 0.0, 0.0]);
        assert!(score > 0.9, "positive bias should push score high: {score}");
    }

    #[test]
    fn classify_feature_sensitive() {
        let backend = LogisticBackend::new(vec![0.01, 0.5, 0.3], -2.0);
        let simple = backend.classify_complexity(&[10.0, 0.0, 1.0]);
        let complex = backend.classify_complexity(&[500.0, 5.0, 10.0]);
        assert!(
            complex > simple,
            "complex features should score higher: simple={simple}, complex={complex}"
        );
    }

    #[test]
    fn train_learns_separation() {
        let mut backend = LogisticBackend::new(vec![0.0, 0.0, 0.0], 0.0);

        let examples = vec![
            (vec![10.0_f32, 0.0, 1.0], 0.0),
            (vec![20.0, 0.0, 2.0], 0.0),
            (vec![500.0, 5.0, 10.0], 1.0),
            (vec![1000.0, 8.0, 15.0], 1.0),
        ];

        backend.train(&examples, 0.01, 500);

        let simple_score = backend.classify_complexity(&[15.0, 0.0, 1.0]);
        let complex_score = backend.classify_complexity(&[800.0, 7.0, 12.0]);

        assert!(
            simple_score < 0.5,
            "trained model should classify simple as low: {simple_score}"
        );
        assert!(
            complex_score > 0.5,
            "trained model should classify complex as high: {complex_score}"
        );
    }

    #[test]
    fn train_empty_examples() {
        let mut backend = LogisticBackend::new(vec![1.0, 2.0], 0.5);
        backend.train(&[], 0.01, 100);
        assert!(
            (backend.bias() - 0.5).abs() < f64::EPSILON,
            "should not change with no data"
        );
    }

    #[test]
    fn save_and_load() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("model.txt");

        let original = LogisticBackend::new(vec![0.1, -0.3, 0.5], -1.2);
        original.save(&path).unwrap();

        let loaded = LogisticBackend::from_file(&path).unwrap();
        assert!((loaded.bias() - original.bias()).abs() < 1e-10);
        assert_eq!(loaded.weights().len(), original.weights().len());
        for (a, b) in loaded.weights().iter().zip(original.weights()) {
            assert!((a - b).abs() < 1e-10);
        }
    }

    #[test]
    fn load_invalid_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("bad_model.txt");
        std::fs::write(&path, "not_a_number\n").unwrap();
        assert!(LogisticBackend::from_file(&path).is_err());
    }

    #[test]
    fn load_missing_file() {
        let result = LogisticBackend::from_file(Path::new("/nonexistent/model.txt"));
        assert!(result.is_err());
    }

    #[test]
    fn preference_collector() {
        let mut collector = PreferenceCollector::new();
        assert!(collector.is_empty());

        collector.record(vec![10.0, 0.0, 1.0], false);
        collector.record(vec![500.0, 5.0, 10.0], true);

        assert_eq!(collector.len(), 2);
        let examples = collector.examples();
        assert_eq!(examples.len(), 2);
        assert!((examples[0].1 - 0.0).abs() < f64::EPSILON);
        assert!((examples[1].1 - 1.0).abs() < f64::EPSILON);

        collector.clear();
        assert!(collector.is_empty());
    }

    #[test]
    fn preference_record_conversion() {
        let record = PreferenceRecord {
            features: vec![1.0, 2.0, 3.0],
            strong_model_won: true,
        };
        let (feats, label) = record.to_training_example();
        assert_eq!(feats, vec![1.0, 2.0, 3.0]);
        assert!((label - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn weights_accessor() {
        let backend = LogisticBackend::new(vec![0.1, 0.2, 0.3], 0.5);
        assert_eq!(backend.weights(), &[0.1, 0.2, 0.3]);
        assert!((backend.bias() - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn backend_trait_implementation() {
        let backend = LogisticBackend::new(vec![0.01, 0.5, 0.3], -2.0);
        let score = RouterBackend::classify_complexity(&backend, &[100.0, 3.0, 5.0]);
        assert!((0.0..=1.0).contains(&score));
    }
}
