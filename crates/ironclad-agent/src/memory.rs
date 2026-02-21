use ironclad_core::config::MemoryConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryBudgets {
    pub working: usize,
    pub episodic: usize,
    pub semantic: usize,
    pub procedural: usize,
    pub relationship: usize,
}

pub struct MemoryBudgetManager {
    config: MemoryConfig,
}

impl MemoryBudgetManager {
    pub fn new(config: MemoryConfig) -> Self {
        Self { config }
    }

    /// Distributes `total_tokens` across the five memory tiers based on config percentages.
    /// Any remainder from rounding is added to the working memory tier.
    pub fn allocate_budgets(&self, total_tokens: usize) -> MemoryBudgets {
        let working = pct(total_tokens, self.config.working_budget_pct);
        let episodic = pct(total_tokens, self.config.episodic_budget_pct);
        let semantic = pct(total_tokens, self.config.semantic_budget_pct);
        let procedural = pct(total_tokens, self.config.procedural_budget_pct);
        let relationship = pct(total_tokens, self.config.relationship_budget_pct);

        let allocated = working + episodic + semantic + procedural + relationship;
        let rollover = total_tokens.saturating_sub(allocated);

        MemoryBudgets {
            working: working + rollover,
            episodic,
            semantic,
            procedural,
            relationship,
        }
    }
}

fn pct(total: usize, percent: f64) -> usize {
    ((total as f64) * percent / 100.0).floor() as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> MemoryConfig {
        MemoryConfig {
            working_budget_pct: 30.0,
            episodic_budget_pct: 25.0,
            semantic_budget_pct: 20.0,
            procedural_budget_pct: 15.0,
            relationship_budget_pct: 10.0,
            embedding_provider: None,
            embedding_model: None,
            hybrid_weight: 0.5,
        }
    }

    #[test]
    fn budget_allocation_matches_percentages() {
        let mgr = MemoryBudgetManager::new(default_config());
        let budgets = mgr.allocate_budgets(10_000);

        assert_eq!(budgets.working, 3_000);
        assert_eq!(budgets.episodic, 2_500);
        assert_eq!(budgets.semantic, 2_000);
        assert_eq!(budgets.procedural, 1_500);
        assert_eq!(budgets.relationship, 1_000);

        let sum = budgets.working
            + budgets.episodic
            + budgets.semantic
            + budgets.procedural
            + budgets.relationship;
        assert_eq!(sum, 10_000);
    }

    #[test]
    fn rollover_goes_to_working() {
        let mgr = MemoryBudgetManager::new(default_config());
        let budgets = mgr.allocate_budgets(99);

        let sum = budgets.working
            + budgets.episodic
            + budgets.semantic
            + budgets.procedural
            + budgets.relationship;
        assert_eq!(sum, 99, "all tokens must be distributed");
        assert!(budgets.working >= pct(99, 30.0));
    }

    #[test]
    fn zero_total_tokens() {
        let mgr = MemoryBudgetManager::new(default_config());
        let budgets = mgr.allocate_budgets(0);

        assert_eq!(
            budgets,
            MemoryBudgets {
                working: 0,
                episodic: 0,
                semantic: 0,
                procedural: 0,
                relationship: 0,
            }
        );
    }
}
