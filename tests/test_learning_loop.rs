//! End-to-end test of the continual-learning loop against a mock venue.
//!
//! Verifies the chain that previously did not exist: a submitted order is
//! reconciled into a structured outcome (R, MFE/MAE, hold time, exit reason),
//! the qualified outcome is written into the experience library, and the
//! library can be read back by the same reader the prompt assembler uses.

use okx_2pa_agent::learning::{
    evaluate_versions, metrics_for, ExperienceWriter, EvaluationPolicy, OutcomeStore,
    QualificationPolicy, Reconciler,
};
use okx_2pa_agent::okx::trading::AuditEntry;
use okx_2pa_agent::records::experience::ExperienceReader;
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;

mod support;

const ENTRY_TS: i64 = 1_700_000_000_000;
const SIGNAL_ID: &str = "sig-abc123";

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("okx-loop-{tag}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn candle_row(ts: i64, o: &str, h: &str, l: &str, c: &str) -> serde_json::Value {
    json!([ts.to_string(), o, h, l, c, "10", "0", "0", "1"])
}

/// Bars newest-first, as OKX returns them. The oldest bar after entry reaches
/// the 102.0 target, so the trade resolves as a +2R win on the first bar.
fn candle_rows() -> Vec<serde_json::Value> {
    vec![
        candle_row(ENTRY_TS + 2_700_000, "102", "102.6", "101.8", "102.5"),
        candle_row(ENTRY_TS + 1_800_000, "101", "102.8", "100.9", "102.2"),
        candle_row(ENTRY_TS + 900_000, "100", "102.4", "99.6", "102.1"),
    ]
}

fn audit_entry() -> AuditEntry {
    AuditEntry {
        strategy_id: "2pa_trend".into(),
        strategy_version: "2026-09-v1".into(),
        decision_record_id: "rec-1".into(),
        prompt_version: "v1".into(),
        prompt_hash: "hash-v1".into(),
        cycle_position: "trending_tr".into(),
        detected_patterns: vec!["h2".into(), "breakout_test".into()],
        id: "audit-1".into(),
        timestamp_ms: ENTRY_TS,
        submitted: true,
        signal_id: SIGNAL_ID.into(),
        instrument: "TEST-USDT-SWAP".into(),
        timeframe: "15m".into(),
        direction: "做多".into(),
        order_type: "限价单".into(),
        confidence: Some(json!(70)),
        size: Some(json!(1.0)),
        price: Some(json!(100.0)),
        stop_loss_price: Some(json!(99.0)),
        take_profit_price: Some(json!(102.0)),
        order_id: "ord-1".into(),
        reason: String::new(),
        error_code: String::new(),
        broker_tag: "tag".into(),
        deleted: false,
    }
}

fn mock_routes(order_state: &str, acc_fill: &str, avg_px: &str) -> HashMap<String, serde_json::Value> {
    HashMap::from([
        (
            "/api/v5/trade/order".to_string(),
            support::ok(json!([{
                "state": order_state,
                "accFillSz": acc_fill,
                "sz": "1",
                "avgPx": avg_px,
            }])),
        ),
        (
            "/api/v5/market/candles".to_string(),
            support::ok(serde_json::Value::Array(candle_rows())),
        ),
    ])
}

fn policy() -> QualificationPolicy {
    QualificationPolicy { require_filled: true, min_hold_bars: 1, max_abs_r: 25.0 }
}

/// Float comparison for values derived from decimal price strings.
fn approx(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() < 1e-9
}

#[tokio::test]
async fn filled_order_becomes_a_qualified_outcome_and_an_experience_case() {
    let mock = support::Mock::start(mock_routes("filled", "1", "100.0")).await;
    let root = temp_dir("filled");
    let store = OutcomeStore::new(root.join("outcomes"));
    let writer = ExperienceWriter::new(root.join("experience"));

    let reconciler = Reconciler::new(mock.client.clone(), store.clone(), writer.clone());
    let report = reconciler
        .reconcile(&[audit_entry()], &policy(), 96, 0, true)
        .await;

    assert_eq!(report.checked, 1);
    assert_eq!(report.resolved, 1, "errors: {:?}", report.errors);
    assert_eq!(report.experiences_written, 1);

    let outcome = store.load(SIGNAL_ID).expect("outcome must be persisted");
    assert!(outcome.filled);
    assert_eq!(outcome.side, "long", "Chinese direction must normalise");
    assert_eq!(outcome.exit_reason, "take_profit");
    assert!(approx(outcome.r_multiple, 2.0), "r={}", outcome.r_multiple);
    assert!(approx(outcome.mfe_r, 2.4), "mfe={}", outcome.mfe_r);
    assert!(approx(outcome.mae_r, 0.4), "mae={}", outcome.mae_r);
    assert_eq!(outcome.hold_bars, 1);
    assert!(outcome.qualified, "{}", outcome.qualification_reason);
    assert_eq!(outcome.pnl_source, "model_estimate");
    assert_eq!(outcome.decision_record_id, "rec-1");
    assert_eq!(outcome.prompt_version, "v1");

    // The library the prompt assembler reads must see it, with the fields the
    // reader scores on.
    let reader = ExperienceReader::new(root.join("experience"));
    let cases = reader.read_for_stage2("trending_tr", "long", &["h2".to_string()], 5);
    assert_eq!(cases.len(), 1);
    assert_eq!(cases[0].case_type, "success");
    assert_eq!(cases[0].content["direction"], "long");
    assert_eq!(cases[0].content["outcome"]["r_multiple"], 2.0);

    // Re-running must be idempotent.
    let second = reconciler
        .reconcile(&[audit_entry()], &policy(), 96, 0, true)
        .await;
    assert_eq!(second.skipped_existing, 1);
    assert_eq!(second.experiences_written, 0);

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn losing_trade_is_recorded_as_a_failure_case() {
    // Price never reaches the target; the stop at 99 is taken instead.
    let rows = vec![
        candle_row(ENTRY_TS + 1_800_000, "99", "99.4", "98.2", "98.6"),
        candle_row(ENTRY_TS + 900_000, "100", "100.4", "98.9", "99.1"),
    ];
    let routes = HashMap::from([
        (
            "/api/v5/trade/order".to_string(),
            support::ok(json!([{"state":"filled","accFillSz":"1","sz":"1","avgPx":"100.0"}])),
        ),
        (
            "/api/v5/market/candles".to_string(),
            support::ok(serde_json::Value::Array(rows)),
        ),
    ]);
    let mock = support::Mock::start(routes).await;
    let root = temp_dir("loss");
    let store = OutcomeStore::new(root.join("outcomes"));
    let writer = ExperienceWriter::new(root.join("experience"));

    let reconciler = Reconciler::new(mock.client.clone(), store.clone(), writer.clone());
    let report = reconciler.reconcile(&[audit_entry()], &policy(), 96, 0, true).await;
    assert_eq!(report.resolved, 1, "errors: {:?}", report.errors);

    let outcome = store.load(SIGNAL_ID).unwrap();
    assert_eq!(outcome.exit_reason, "stop_loss");
    assert!(approx(outcome.r_multiple, -1.0), "r={}", outcome.r_multiple);
    assert!(outcome.qualified);

    let cases = ExperienceReader::new(root.join("experience"))
        .read_for_stage2("trending_tr", "long", &["h2".to_string()], 5);
    assert_eq!(cases.len(), 1);
    assert_eq!(cases[0].case_type, "failure");
    assert!(cases[0].content["lesson"].as_str().unwrap().contains("做多"));

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn cancelled_order_is_recorded_but_never_learned_from() {
    let mock = support::Mock::start(mock_routes("canceled", "0", "")).await;
    let root = temp_dir("canceled");
    let store = OutcomeStore::new(root.join("outcomes"));
    let writer = ExperienceWriter::new(root.join("experience"));

    let reconciler = Reconciler::new(mock.client.clone(), store.clone(), writer.clone());
    let report = reconciler.reconcile(&[audit_entry()], &policy(), 96, 0, true).await;

    assert_eq!(report.unfilled, 1, "errors: {:?}", report.errors);
    assert_eq!(report.resolved, 0);
    assert_eq!(report.experiences_written, 0);

    let outcome = store.load(SIGNAL_ID).expect("unfilled orders stay auditable");
    assert!(!outcome.filled);
    assert!(!outcome.qualified);
    assert_eq!(outcome.exit_reason, "unfilled");
    assert!(outcome.qualification_reason.contains("未成交"));

    assert!(reader_is_empty(&root));
    let _ = std::fs::remove_dir_all(&root);
}

fn reader_is_empty(root: &std::path::Path) -> bool {
    ExperienceReader::new(root.join("experience"))
        .read_for_stage2("trending_tr", "long", &["h2".to_string()], 5)
        .is_empty()
}

#[tokio::test]
async fn still_live_order_is_not_resolved_prematurely() {
    let mock = support::Mock::start(mock_routes("live", "0", "")).await;
    let root = temp_dir("live");
    let store = OutcomeStore::new(root.join("outcomes"));
    let writer = ExperienceWriter::new(root.join("experience"));

    let reconciler = Reconciler::new(mock.client.clone(), store.clone(), writer.clone());
    let report = reconciler.reconcile(&[audit_entry()], &policy(), 96, 0, true).await;

    assert_eq!(report.still_open, 1);
    assert_eq!(report.resolved, 0);
    assert!(!store.exists(SIGNAL_ID), "an open order must not be written as a result");
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn partial_fill_is_measured_against_filled_quantity() {
    let mock = support::Mock::start(mock_routes("partially_filled", "0.5", "100.0")).await;
    let root = temp_dir("partial");
    let store = OutcomeStore::new(root.join("outcomes"));
    let writer = ExperienceWriter::new(root.join("experience"));

    let reconciler = Reconciler::new(mock.client.clone(), store.clone(), writer.clone());
    let report = reconciler.reconcile(&[audit_entry()], &policy(), 96, 0, true).await;
    assert_eq!(report.resolved, 1, "errors: {:?}", report.errors);

    let outcome = store.load(SIGNAL_ID).unwrap();
    assert!(outcome.filled);
    assert_eq!(outcome.fill_ratio, 0.5);
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn evaluation_gates_a_candidate_against_the_incumbent() {
    let root = temp_dir("eval");
    let store = OutcomeStore::new(root.join("outcomes"));
    let writer = ExperienceWriter::new(root.join("experience"));
    let mock = support::Mock::start(mock_routes("filled", "1", "100.0")).await;

    // Two independent signals so we get one v1 sample and one v2 sample.
    let mut first = audit_entry();
    first.signal_id = "sig-v1".into();
    first.prompt_version = "v1".into();
    let mut second = audit_entry();
    second.signal_id = "sig-v2".into();
    second.prompt_version = "v2".into();
    second.order_id = "ord-2".into();

    let reconciler = Reconciler::new(mock.client.clone(), store.clone(), writer.clone());
    let report = reconciler.reconcile(&[first, second], &policy(), 96, 0, true).await;
    assert_eq!(report.resolved, 2, "errors: {:?}", report.errors);

    let outcomes = store.list(10);
    assert_eq!(outcomes.len(), 2);
    assert_eq!(metrics_for(&outcomes).samples, 2);

    // Both versions produced the same +2R result, so a demanding policy that
    // needs more samples must refuse to promote the candidate.
    let strict = EvaluationPolicy { min_samples: 20, min_expectancy_delta_r: 0.0, max_win_rate_drop: 0.05 };
    let verdicts = evaluate_versions(&outcomes, "v1", &strict);
    assert!(!verdicts["v2"].accepted);
    assert!(verdicts["v2"].reasons.iter().any(|r| r.contains("样本不足")));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn reconciliation_requires_the_receipt_fields_to_be_present() {
    // Guards the linkage contract: without these the loop silently stops
    // working, so the fields are part of the audit schema.
    let entry = audit_entry();
    assert!(!entry.decision_record_id.is_empty());
    assert!(!entry.prompt_version.is_empty());
    assert!(!entry.prompt_hash.is_empty());
    assert!(!entry.cycle_position.is_empty());
    assert!(!entry.signal_id.is_empty());
}

#[test]
fn receipt_fields_must_not_change_the_signal_id() {
    // `signal_id` is simultaneously the duplicate-submission guard and the
    // receipt the reconciler joins on. Stamping a decision with loop metadata
    // before execution must therefore leave it untouched, or reconciliation
    // would look up an id the audit row never recorded.
    use okx_2pa_agent::okx::trading::OKXTradeExecutor;

    let decision = json!({
        "order_direction": "做多",
        "order_type": "限价单",
        "entry_price": 100.0,
        "stop_loss_price": 99.0,
        "take_profit_price": 102.0,
    });

    let mut stamped = decision.clone();
    stamped["decision_record_id"] = json!("rec-1");
    stamped["prompt_version"] = json!("v1");
    stamped["prompt_hash"] = json!("hash-v1");
    stamped["cycle_position"] = json!("trending_tr");
    stamped["detected_patterns"] = json!(["h2", "breakout_test"]);

    let plain = OKXTradeExecutor::generate_signal_id(
        "TEST-USDT-SWAP", "15m", ENTRY_TS, &decision,
    );
    let with_receipt = OKXTradeExecutor::generate_signal_id(
        "TEST-USDT-SWAP", "15m", ENTRY_TS, &stamped,
    );
    assert_eq!(plain, with_receipt, "receipt stamping must not perturb the idempotency key");
    assert!(!plain.is_empty());
}
