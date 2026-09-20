use okx_2pa_agent::data::base::KlineBar;
use okx_2pa_agent::learning::{
    EvaluationPolicy, Proposer, ReplayEvaluator, StrategyReflector, Trade2Episode, TradeOutcome,
};

fn sample_outcome(r: f64, hold_bars: u32, regime: &str, patterns: Vec<&str>) -> TradeOutcome {
    TradeOutcome {
        signal_id: format!("sig_{}", uuid::Uuid::new_v4().simple()),
        decision_record_id: "rec_live_01".to_string(),
        strategy_id: "2pa_trend".to_string(),
        strategy_version: "v1".to_string(),
        prompt_version: "v1".to_string(),
        prompt_hash: "hash_v1".to_string(),
        symbol: "BTC-USDT-SWAP".to_string(),
        timeframe: "15m".to_string(),
        side: "long".to_string(),
        cycle_position: regime.to_string(),
        detected_patterns: patterns.into_iter().map(|s| s.to_string()).collect(),
        entry_price: 60000.0,
        stop_price: 59500.0,
        target_price: 61000.0,
        exit_price: Some(if r > 0.0 { 61000.0 } else { 59500.0 }),
        size: 1.0,
        filled: true,
        fill_ratio: 1.0,
        fees_usd: 2.0,
        realized_pnl_usd: r * 500.0,
        pnl_source: "broker".to_string(),
        r_multiple: r,
        mfe_r: if r > 0.0 { 2.0 } else { 0.1 },
        mae_r: if r > 0.0 { 0.2 } else { 1.0 },
        hold_bars,
        exit_reason: if r > 0.0 { "take_profit".to_string() } else { "stop_loss".to_string() },
        qualified: true,
        qualification_reason: "合格交易".to_string(),
        created_ms: 1000,
        resolved_ms: 5000,
    }
}

#[test]
fn test_strategy_reflector_and_proposer_mutation() {
    let outcome1 = sample_outcome(-1.0, 1, "spike", vec![]);
    let outcome2 = sample_outcome(-1.0, 6, "trading_range", vec!["breakout"]);

    let failures = vec![outcome1, outcome2];
    let attributions = StrategyReflector::attribute_failures(&failures);

    assert_eq!(attributions.len(), 2);
    assert_eq!(attributions[0].failure_mode, "whipsaw_tight_stop");
    assert_eq!(attributions[1].failure_mode, "range_breakout_trap");

    let incumbent = include_str!("../prompt_engineering/strategy_v1.txt");
    let proposal = Proposer::propose_mutation(incumbent, "v1", &attributions).expect("proposal generated");

    assert!(proposal.mutated_content.contains("GEPA 反思突变补丁"));
    assert!(proposal.mutated_content.contains("whipsaw_tight_stop"));
    assert!(proposal.mutated_content.contains("range_breakout_trap"));
    assert_eq!(proposal.addressed_modes.len(), 2);
}

#[test]
fn test_trade2episode_solidification_and_data_flywheel() {
    let dir = std::env::temp_dir().join(format!("okx_flywheel_{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();

    let win_trade = sample_outcome(2.0, 10, "tight_channel", vec!["h2"]);
    let bar1 = KlineBar {
        seq: 1,
        ts_open: 1000,
        open: 60000.0,
        high: 60200.0,
        low: 59900.0,
        close: 60150.0,
        volume: 10.0,
        amount: 0.0,
        pct_chg: None,
        closed: true,
    };
    let bar2 = KlineBar {
        seq: 0,
        ts_open: 2000,
        open: 60150.0,
        high: 61100.0,
        low: 60100.0,
        close: 61050.0,
        volume: 15.0,
        amount: 0.0,
        pct_chg: None,
        closed: true,
    };

    let episode_path = Trade2Episode::solidify(&dir, &win_trade, vec![bar1], vec![bar2]).expect("solidify");
    assert!(episode_path.exists());

    let loaded = okx_2pa_agent::records::benchmark::load_benchmark_episodes(&dir).expect("load episodes");
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].expected_action, "OPEN_LONG");
    assert_eq!(loaded[0].benchmark_r, 2.0);

    // Now test ReplayEvaluator against this freshly solidified episode
    let prompt = include_str!("../prompt_engineering/strategy_v1.txt");
    let (outcomes, metrics) = ReplayEvaluator::evaluate_suite(prompt, "v1", &loaded);
    assert_eq!(outcomes.len(), 1);
    assert_eq!(metrics.samples, 1);
    assert!(metrics.expectancy_r > 1.5);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_end_to_end_flywheel_adaptation() {
    // 1. A losing trade occurs in choppy trading range
    let loss = sample_outcome(-1.0, 8, "trading_range", vec!["breakout_test"]);

    // 2. Reflector identifies range breakout trap
    let attrs = StrategyReflector::attribute_failures(std::slice::from_ref(&loss));
    assert_eq!(attrs.len(), 1);
    assert_eq!(attrs[0].failure_mode, "range_breakout_trap");

    // 3. Proposer generates mutated prompt with defensive clause
    let incumbent = include_str!("../prompt_engineering/strategy_v1.txt");
    let proposal = Proposer::propose_mutation(incumbent, "v1", &attrs).expect("mutation proposal");

    // 4. Solidify the trade into benchmark episode (expected action is WAIT to avoid loss)
    let episode = Trade2Episode::convert(&loss, vec![], vec![]);
    assert_eq!(episode.expected_action, "WAIT");

    // 5. Evaluate mutated candidate against this episode:
    // The mutated prompt includes "震荡" and "WAIT" defensive rules, so it will correctly WAIT!
    let candidate_outcome = ReplayEvaluator::evaluate_episode(&proposal.mutated_content, "v2", &episode);
    // Correct WAIT in chop means unfilled/safe, not stopped out!
    assert_eq!(candidate_outcome.exit_reason, "unfilled");
    assert_eq!(candidate_outcome.r_multiple, 0.0);

    // Compare with a naive broken prompt that has no range/chop rules
    let broken_prompt = "全天候无脑盲目开仓做多，不设置任何止损与风控约束，持续追高入场。";
    let broken_outcome = ReplayEvaluator::evaluate_episode(broken_prompt, "v_broken", &episode);
    assert_eq!(broken_outcome.exit_reason, "stop_loss");
    assert_eq!(broken_outcome.r_multiple, -1.0);

    // Compare candidate against broken prompt across a suite containing both trend and chop
    let win_trade = sample_outcome(2.0, 10, "tight_channel", vec!["h2"]);
    let win_episode = Trade2Episode::convert(&win_trade, vec![], vec![]);
    assert_eq!(win_episode.future_bars.len(), 10, "Solidified episode should have synthesized 10 future bars");

    let suite = vec![win_episode, episode];
    let policy = EvaluationPolicy { min_samples: 1, min_expectancy_delta_r: 0.0, max_win_rate_drop: 0.05 };
    let verdict = ReplayEvaluator::evaluate_candidate_vs_baseline(
        &proposal.mutated_content,
        "v2",
        broken_prompt,
        "v_broken",
        &suite,
        &policy,
    );
    assert!(verdict.accepted, "Candidate must be accepted: {:?}", verdict.reasons);
}
