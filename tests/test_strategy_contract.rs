mod support;
use okx_2pa_agent::{data::base::*, okx::trading::OKXTradeExecutor, strategies::*};
use serde_json::{json, Value};

fn set(f: &mut KlineFrame, i: usize, o: f64, h: f64, l: f64, c: f64) {
    let b = &mut f.bars[i];
    b.open = o;
    b.high = h;
    b.low = l;
    b.close = c;
}
fn fixture(id: &str) -> (KlineFrame, KlineFrame) {
    let mut f = KlineFrame {
        symbol: "TEST-USDT-SWAP".into(),
        timeframe: "15m".into(),
        snapshot_ts_local_ms: 90_000_000,
        bars: (0..20)
            .map(|i| KlineBar {
                seq: i + 1,
                ts_open: 89_100_000 - i as i64 * 900_000,
                open: 99.5,
                high: 100.0,
                low: 99.0,
                close: 99.7,
                volume: 10.0,
                amount: 0.0,
                pct_chg: None,
                closed: true,
            })
            .collect(),
        indicators: IndicatorBundle {
            ema20: (0..20).map(|i| 100.0 - i as f64 * 0.04).collect(),
            atr14: vec![1.0; 20],
            sma14: vec![100.0; 20],
            sma170: (0..20).map(|i| 100.0 - i as f64 * 0.03).collect(),
            sma170_slope: vec![],
            dev170_pct: vec![],
        },
    };
    set(&mut f, 0, 100.2, 101.0, 100.0, 100.8);
    set(&mut f, 1, 100.1, 100.3, 99.9, 100.0);
    set(&mut f, 2, 99.8, 100.7, 99.5, 100.5);
    set(&mut f, 12, 100.0, 106.0, 99.0, 100.0);
    let mut h = f.clone();
    h.timeframe = "1h".into();
    for (i, b) in h.bars.iter_mut().enumerate() {
        b.ts_open = 86_400_000 - i as i64 * 3_600_000;
    }
    if id == "dog_reversion" {
        f.indicators.sma170 = vec![106.0; 20];
        f.indicators.sma14[0] = 100.5;
        set(&mut f, 1, 100.0, 100.3, 99.0, 99.9);
        set(&mut f, 3, 99.9, 100.0, 99.0, 99.8);
        set(&mut f, 5, 100.0, 100.6, 99.4, 100.5);
        h.indicators.ema20 = vec![100.8; 20];
    }
    (f, h)
}
fn mirror(f: &mut KlineFrame) {
    for b in &mut f.bars {
        let high = b.high;
        b.open = 200.0 - b.open;
        b.high = 200.0 - b.low;
        b.low = 200.0 - high;
        b.close = 200.0 - b.close;
    }
    for v in [
        &mut f.indicators.ema20,
        &mut f.indicators.sma14,
        &mut f.indicators.sma170,
    ] {
        for x in v {
            *x = 200.0 - *x;
        }
    }
}
fn proposal(e: &Evidence) -> Value {
    let sign = if e.direction == "做多" { 1.0 } else { -1.0 };
    json!({"action":"OPEN","order_type":"限价单","order_direction":e.direction,"entry_price":e.reference_close,
        "stop_loss_price":e.invalidation-sign*0.3*e.atr,"take_profit_price":e.target_bound,
        "trade_confidence":80,"estimated_win_rate":99,"traders_equation_passes":true})
}

#[test]
fn all_three_strategies_accept_confirmed_symmetric_setups() {
    for id in ["2pa_trend", "dog_reversion", "dog_trend"] {
        for sign in [1.0, -1.0] {
            let (mut f, mut h) = fixture(id);
            if sign < 0.0 {
                mirror(&mut f);
                mirror(&mut h);
            }
            let e = evidence(id, &f, Some(&h), sign).unwrap();
            let mut w = json!({"decision":proposal(&e),"terminal":{"outcome":"trade"}});
            enforce(
                id,
                &f,
                Some(&h),
                &json!({"gate_result":"proceed"}),
                &mut w,
                None,
            );
            assert_eq!(w["program_validation"]["passed"], true, "{id} {sign}: {w}");
            assert_eq!(w["decision"]["strategy_id"], id);
            assert!(w["decision"]["estimated_win_rate"].is_null());
            assert!(w["decision"]["risk_reward_ratio"].as_f64().unwrap() >= MIN_NET_RR);
        }
    }
}

#[test]
fn trend_retest_has_no_reversion_distance_requirement() {
    let (f, h) = fixture("dog_trend");
    assert!(evidence("dog_trend", &f, Some(&h), 1.0).is_ok());
    assert!(evidence("dog_reversion", &f, Some(&h), 1.0).is_err());
}

#[test]
fn no_confirmation_no_entry_even_with_model_claims() {
    for id in ["2pa_trend", "dog_reversion", "dog_trend"] {
        let (mut f, h) = fixture(id);
        let e = evidence(id, &f, Some(&h), 1.0).unwrap();
        let mut w = json!({"decision":proposal(&e),"terminal":{"outcome":"trade"}});
        f.bars[0].close = f.bars[1].close;
        enforce(
            id,
            &f,
            Some(&h),
            &json!({"gate_result":"proceed","detected_patterns":["h2","break_above_sma14"]}),
            &mut w,
            None,
        );
        assert_eq!(w["decision"]["action"], "WAIT");
        assert!(w["rejected_proposal"].is_object());
    }
}

#[test]
fn missing_stale_future_or_opposing_htf_blocks_entries() {
    let (f, h) = fixture("dog_trend");
    assert!(evidence("dog_trend", &f, None, 1.0).is_err());
    for offset in [-3_600_000, 3_600_000] {
        let mut bad = h.clone();
        for b in &mut bad.bars {
            b.ts_open += offset;
        }
        assert!(evidence("dog_trend", &f, Some(&bad), 1.0).is_err());
    }
    let mut bad = h.clone();
    mirror(&mut bad);
    assert!(evidence("dog_trend", &f, Some(&bad), 1.0).is_err());
    let mut bad = f.clone();
    bad.bars[0].closed = false;
    assert!(evidence("dog_trend", &bad, Some(&h), 1.0).is_err());
}

#[test]
fn stage1_wait_terminal_wait_and_existing_position_override_open() {
    let (f, h) = fixture("dog_trend");
    let e = evidence("dog_trend", &f, Some(&h), 1.0).unwrap();
    for mode in ["stage1", "terminal", "equation", "position", "action"] {
        let mut w = json!({"decision":proposal(&e),"terminal":{"outcome":"trade"}});
        if mode == "terminal" {
            w["terminal"]["outcome"] = json!("wait");
        }
        if mode == "equation" {
            w["decision"]["traders_equation_passes"] = json!(false);
        }
        if mode == "action" {
            w["decision"]["action"] = json!("WAIT");
        }
        let pos = PositionContext {
            has_position: true,
            ..Default::default()
        };
        enforce(
            "dog_trend",
            &f,
            Some(&h),
            &json!({"gate_result":if mode=="stage1"{"wait"}else{"proceed"}}),
            &mut w,
            if mode == "position" { Some(&pos) } else { None },
        );
        assert_ne!(w["decision"]["action"], "OPEN", "{mode}");
        assert_eq!(w["program_validation"]["passed"], false);
    }
}

#[test]
fn aliases_preserve_attribution_and_adaptive_is_observation_only() {
    assert_eq!(canonical("2pa"), Some("2pa_trend"));
    assert_eq!(canonical("dog_walking"), Some("dog_reversion"));
    assert_eq!(canonical("alpha_pilot"), None);
    assert_eq!(canonical("unknown"), None);
    let (f, h) = fixture("2pa_trend");
    assert!(evidence("adaptive", &f, Some(&h), 1.0).is_err());
    assert!(evidence("alpha_pilot", &f, Some(&h), 1.0).is_err());
}

#[test]
fn structure_drift_costs_and_remaining_space_are_hard_gates() {
    let (f, h) = fixture("dog_trend");
    let e = evidence("dog_trend", &f, Some(&h), 1.0).unwrap();
    for (key, value) in [
        ("entry_price", 101.2),
        ("stop_loss_price", 99.4),
        ("stop_loss_price", 95.0),
        ("take_profit_price", 107.0),
        ("take_profit_price", 102.0),
    ] {
        let mut d = proposal(&e);
        d[key] = json!(value);
        assert!(validate_entry(&f.symbol, &d, &e).is_err(), "{key} {value}");
    }
    // Gross 1.6 looks acceptable, but both winning and losing exits incur costs.
    assert!(net_rr("TEST-USDT-SWAP", 100.0, 99.0, 101.6) < 1.5);
}

#[test]
fn management_preserves_targets_and_requires_cost_covered_break_even() {
    assert!(validate_management(
        &json!({"action":"HOLD","order_type":"平仓"}),
        None,
        "TEST-USDT-SWAP"
    )
    .is_err());
    for sign in [1.0, -1.0] {
        let p = PositionContext {
            has_position: true,
            pos_side: if sign > 0.0 { "long" } else { "short" }.into(),
            open_avg_px: Some(100.0),
            current_sl: Some(100.0 - sign * 2.0),
            current_tp: Some(100.0 + sign * 5.0),
            ..Default::default()
        };
        assert!(validate_management(
            &json!({"action":"MOVE_TAKE_PROFIT","new_take_profit_price":100.0+sign*6.0}),
            Some(&p),
            "TEST-USDT-SWAP"
        )
        .is_err());
        assert!(validate_management(
            &json!({"action":"MOVE_STOP_LOSS","new_stop_loss_price":100.0}),
            Some(&p),
            "TEST-USDT-SWAP"
        )
        .is_err());
        assert!(validate_management(
            &json!({"action":"MOVE_STOP_LOSS","new_stop_loss_price":100.0+sign*0.2}),
            Some(&p),
            "TEST-USDT-SWAP"
        )
        .is_ok());
        assert!(
            validate_management(&json!({"action":"CLOSE_EARLY"}), Some(&p), "TEST-USDT-SWAP")
                .is_ok()
        );
    }
}

#[test]
fn pa_second_entry_requires_two_attempts_and_trend() {
    for sign in [1.0, -1.0] {
        let (mut f, mut h) = fixture("2pa_trend");
        set(&mut f, 5, 100.0, 102.0, 99.5, 100.1);
        set(&mut f, 4, 100.0, 100.2, 99.0, 99.8);
        set(&mut f, 3, 100.0, 100.6, 99.3, 100.3);
        set(&mut f, 2, 100.0, 100.4, 99.5, 100.1);
        set(&mut f, 1, 100.0, 100.3, 99.4, 100.0);
        if sign < 0.0 {
            mirror(&mut f);
            mirror(&mut h);
        }
        let e = evidence("2pa_trend", &f, Some(&h), sign).unwrap();
        assert_eq!(e.setup, "second_entry");
        // Without the first failed attempt this must not be counted as H2/L2.
        if sign > 0.0 {
            f.bars[3].high = 100.1;
            f.bars[3].close = 100.0;
        } else {
            f.bars[3].low = 99.9;
            f.bars[3].close = 100.0;
        }
        assert!(evidence("2pa_trend", &f, Some(&h), sign).is_err());
    }
}

#[tokio::test]
async fn tick_rounding_cannot_cross_structural_stop_buffer() {
    let (f, h) = fixture("dog_trend");
    let mut e = evidence("dog_trend", &f, Some(&h), 1.0).unwrap();
    e.signal_ts_ms = chrono::Utc::now().timestamp_millis() - 900_000;
    let mut d = proposal(&e);
    d["stop_loss_price"] = json!(99.3);
    assert!(validate_entry(&f.symbol, &d, &e).is_ok());
    d["strategy_id"] = json!("dog_trend");
    d["strategy_evidence"] = json!(e);
    let mut routes = support::routes();
    routes.get_mut("/api/v5/public/instruments").unwrap()["data"][0]["tickSz"] = json!("0.5");
    let mock = support::Mock::start(routes).await;
    let r = executor(&mock)
        .execute(&f.symbol, "15m", e.signal_ts_ms, &d)
        .await;
    assert!(!r.submitted);
    assert!(r.reason.contains("结构失效"), "{}", r.reason);
    assert!(mock
        .calls
        .lock()
        .unwrap()
        .iter()
        .all(|(m, _, _)| m == "GET"));
}

fn executor(mock: &support::Mock) -> OKXTradeExecutor {
    OKXTradeExecutor::new(
        mock.client.clone(),
        1.0,
        "cross",
        "net",
        3.0,
        true,
        40,
        120,
        3,
        None,
        true,
        0.5,
        25.0,
    )
}
#[tokio::test]
async fn execution_rechecks_market_price_and_requires_evidence() {
    let (f, h) = fixture("dog_trend");
    let mut e = evidence("dog_trend", &f, Some(&h), 1.0).unwrap();
    e.signal_ts_ms = chrono::Utc::now().timestamp_millis() - 900_000;
    for mode in ["valid", "missing", "market_drift", "mismatch"] {
        let mock = support::Mock::start(support::routes()).await;
        let mut d = proposal(&e);
        d["strategy_id"] = json!("dog_trend");
        d["strategy_evidence"] = json!(e);
        if mode == "missing" {
            d.as_object_mut().unwrap().remove("strategy_evidence");
        }
        if mode == "market_drift" {
            d["order_type"] = json!("市价单");
        } // mock ticker=100 vs signal=100.8
        if mode == "mismatch" {
            d["strategy_evidence"]["timeframe"] = json!("5m");
        }
        let r = executor(&mock)
            .execute(&f.symbol, "15m", e.signal_ts_ms, &d)
            .await;
        assert_eq!(r.submitted, mode == "valid", "{mode}: {}", r.reason);
        let submitted = mock
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|(m, p, _)| m == "POST" && p.starts_with("/api/v5/trade/order?"));
        assert_eq!(submitted, mode == "valid");
    }
}

#[test]
fn all_stances_use_only_new_contract() {
    let (f, _) = fixture("dog_trend");
    for id in ["2pa", "dog_walking", "2pa_trend", "dog_trend", "adaptive"] {
        for stance in [
            "conservative",
            "balanced",
            "aggressive",
            "extreme_aggressive",
        ] {
            let (p, files, _) = okx_2pa_agent::ai::prompt_assembler::build_stage2_prompt_for_system(
                id,
                &f,
                &json!({}),
                stance,
                true,
                None,
                None,
                None,
                None,
            );
            assert_eq!(files, vec!["strategy_v1.txt"]);
            assert!(!p.contains("自动向外扩"));
            assert!(!p.contains("68–75"));
            assert!(p.contains("estimated_win_rate 必须 null"));
        }
    }
}

#[tokio::test]
async fn full_two_stage_analysis_records_program_verdict_and_canonical_strategy() {
    use okx_2pa_agent::{
        config::settings::Settings, orchestrator::two_stage::TwoStageOrchestrator,
    };
    let (f, h) = fixture("dog_trend");
    let e = evidence("dog_trend", &f, Some(&h), 1.0).unwrap();
    // Both calls receive a schema-valid response. The model deliberately claims
    // proceed even when the second run has no HTF; the program must override it.
    let reply = json!({"cycle_position":"trend","dominant_force":"bulls","gate_result":"proceed",
        "decision":proposal(&e),"terminal":{"outcome":"trade"}});
    let mock=support::Mock::start(std::collections::HashMap::from([("/v1/chat/completions".into(),
        json!({"choices":[{"message":{"content":reply.to_string()}}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}))])).await;
    let mut settings = Settings::default();
    settings.provider.base_url = mock.url.clone();
    settings.provider.api_key = "test".into();
    settings.validation.retry_max = 0;
    let dir = std::env::temp_dir().join(format!("strategy-test-{}", uuid::Uuid::new_v4()));
    let orch = TwoStageOrchestrator::new(settings, dir.clone());
    for with_htf in [true, false] {
        let record = orch
            .run_analysis_with_market_context(
                &f,
                "dog_trend",
                None,
                Some("test context"),
                if with_htf { Some(&h) } else { None },
            )
            .await
            .unwrap();
        let w = record.stage2_decision.unwrap();
        assert_eq!(
            w["decision"]["action"],
            if with_htf { "OPEN" } else { "WAIT" }
        );
        assert_eq!(record.meta.trading_system, "dog_trend");
        assert_eq!(w["program_validation"]["passed"], with_htf);
        assert!(record.htf_text.contains("test context"));
        assert_eq!(record.strategy_files_used, vec!["strategy_v1.txt"]);
        assert!(record.stage1_diagnosis.unwrap()["program_candidates"].is_object());
    }
    assert!(std::fs::read_dir(&dir).unwrap().next().is_some());
    std::fs::remove_dir_all(dir).unwrap();
}
