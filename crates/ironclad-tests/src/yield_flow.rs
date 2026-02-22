use ironclad_core::config::YieldConfig;
use ironclad_wallet::YieldEngine;

fn enabled_engine() -> YieldEngine {
    YieldEngine::new(&YieldConfig {
        enabled: true,
        protocol: "aave".into(),
        chain: "base".into(),
        min_deposit: 50.0,
        withdrawal_threshold: 30.0,
        ..Default::default()
    })
}

fn disabled_engine() -> YieldEngine {
    YieldEngine::new(&YieldConfig {
        enabled: false,
        protocol: "aave".into(),
        chain: "base".into(),
        min_deposit: 50.0,
        withdrawal_threshold: 30.0,
        ..Default::default()
    })
}

#[test]
fn excess_calculation_at_various_balances() {
    let engine = enabled_engine();
    let reserve = 100.0;

    let excess = engine.calculate_excess(200.0, reserve);
    assert!((excess - 90.0).abs() < f64::EPSILON);

    let excess = engine.calculate_excess(110.0, reserve);
    assert!((excess - 0.0).abs() < f64::EPSILON);

    let excess = engine.calculate_excess(105.0, reserve);
    assert!((excess - 0.0).abs() < f64::EPSILON);

    let excess = engine.calculate_excess(50.0, reserve);
    assert!((excess - 0.0).abs() < f64::EPSILON);

    let excess = engine.calculate_excess(500.0, reserve);
    assert!((excess - 390.0).abs() < f64::EPSILON);
}

#[test]
fn deposit_decision_logic() {
    let engine = enabled_engine();

    assert!(engine.should_deposit(50.01));
    assert!(!engine.should_deposit(50.0));
    assert!(!engine.should_deposit(49.99));
    assert!(engine.should_deposit(1000.0));
}

#[test]
fn withdrawal_decision_logic() {
    let engine = enabled_engine();

    assert!(engine.should_withdraw(29.99));
    assert!(!engine.should_withdraw(30.0));
    assert!(!engine.should_withdraw(100.0));
    assert!(engine.should_withdraw(0.0));
}

#[test]
fn disabled_engine_never_recommends() {
    let engine = disabled_engine();

    assert!(!engine.should_deposit(10_000.0));
    assert!(!engine.should_withdraw(0.0));
}

#[tokio::test]
async fn deposit_and_withdraw_produce_tx_hashes() {
    let engine = enabled_engine();

    let deposit_tx = engine.deposit(1.0, None, None).await.unwrap();
    assert!(deposit_tx.starts_with("0x"));
    assert!(deposit_tx.len() > 10);

    let withdraw_tx = engine.withdraw(0.5, None, None).await.unwrap();
    assert!(withdraw_tx.starts_with("0x"));
    assert!(withdraw_tx.len() > 10);

    // Different amounts encode differently in the tx hash (amount * 1e18 as the first u64)
    let deposit_tx_2 = engine.deposit(2.0, None, None).await.unwrap();
    assert_ne!(deposit_tx, deposit_tx_2);
}

#[tokio::test]
async fn disabled_engine_rejects_operations() {
    let engine = disabled_engine();

    assert!(engine.deposit(100.0, None, None).await.is_err());
    assert!(engine.withdraw(50.0, None, None).await.is_err());
}

#[test]
fn full_yield_decision_flow() {
    let engine = enabled_engine();
    let reserve = 100.0;

    let balance_high = 250.0;
    let excess = engine.calculate_excess(balance_high, reserve);
    assert!((excess - 140.0).abs() < f64::EPSILON);
    assert!(engine.should_deposit(excess));

    let balance_ok = 150.0;
    let excess = engine.calculate_excess(balance_ok, reserve);
    assert!((excess - 40.0).abs() < f64::EPSILON);
    assert!(!engine.should_deposit(excess));

    let balance_low = 25.0;
    assert!(engine.should_withdraw(balance_low));

    let balance_safe = 50.0;
    assert!(!engine.should_withdraw(balance_safe));
}
