use ironclad_core::SurvivalTier;
use ironclad_schedule::heartbeat::build_tick_context;

#[test]
fn heartbeat_tick_context_builds_correctly() {
    let ctx = build_tick_context(50.0, 100.0);
    assert_eq!(ctx.credit_balance, 50.0);
    assert_eq!(ctx.usdc_balance, 100.0);
}

#[test]
fn survival_tier_from_combined_balance() {
    assert_eq!(SurvivalTier::from_balance(10.0, 0.0), SurvivalTier::High);
    assert_eq!(SurvivalTier::from_balance(4.99, 0.0), SurvivalTier::Normal);
    assert_eq!(
        SurvivalTier::from_balance(0.49, 0.0),
        SurvivalTier::LowCompute
    );
    assert_eq!(
        SurvivalTier::from_balance(0.09, 0.0),
        SurvivalTier::Critical
    );
}

#[test]
fn cron_schedule_evaluation_basics() {
    use ironclad_schedule::scheduler::DurableScheduler;
    // evaluate_interval with no last_run should always be due
    let now = "2026-02-21T12:00:00";
    let result = DurableScheduler::evaluate_interval(None, 60_000, now);
    assert!(result, "no last_run should mean job is due");

    // evaluate_interval with recent last_run should not be due
    let result = DurableScheduler::evaluate_interval(Some("2026-02-21T11:59:30"), 60_000, now);
    assert!(!result, "30s ago with 60s interval should not be due");

    // evaluate_interval with old last_run should be due
    let result = DurableScheduler::evaluate_interval(Some("2026-02-21T11:58:00"), 60_000, now);
    assert!(result, "2 min ago with 60s interval should be due");
}
