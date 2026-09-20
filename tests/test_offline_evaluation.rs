use okx_2pa_agent::config::settings::Settings;
use okx_2pa_agent::learning::{EvaluationPolicy, ReplayEvaluator, STRATEGY_PROMPT_NAME};
use okx_2pa_agent::records::benchmark::load_benchmark_episodes;
use okx_2pa_agent::web::service::WebTradingService;
use std::path::Path;

#[tokio::test]
async fn test_benchmark_episodes_loaded_successfully() {
    let dir = Path::new("records/benchmark_episodes");
    let episodes = load_benchmark_episodes(dir).expect("load benchmark episodes");
    assert!(!episodes.is_empty(), "benchmark episodes should be present");
    assert!(episodes.len() >= 5);

    let spike_ep = episodes.iter().find(|e| e.episode_id == "ep01_btc_spike_long");
    assert!(spike_ep.is_some());
    assert_eq!(spike_ep.unwrap().expected_action, "OPEN_LONG");

    let range_wait = episodes.iter().find(|e| e.episode_id == "ep03_btc_trading_range_wait");
    assert!(range_wait.is_some());
    assert_eq!(range_wait.unwrap().expected_action, "WAIT");
}

#[tokio::test]
async fn test_replay_evaluator_breaks_activation_deadlock() {
    let dir = Path::new("records/benchmark_episodes");
    let episodes = load_benchmark_episodes(dir).expect("load episodes");

    let baseline = include_str!("../prompt_engineering/strategy_v1.txt");
    let candidate = format!("{}\n## 优化规则：强化窄通道与强趋势入场确定性，严格过滤震荡中轴杂乱信号。\n", baseline);

    let policy = EvaluationPolicy {
        min_samples: 20,
        min_expectancy_delta_r: 0.0,
        max_win_rate_drop: 0.05,
    };

    let verdict = ReplayEvaluator::evaluate_candidate_vs_baseline(
        &candidate,
        "v2",
        baseline,
        "v1",
        &episodes,
        &policy,
    );

    assert!(verdict.accepted, "Candidate should pass benchmark replay evaluation: {:?}", verdict.reasons);
    assert!(verdict.expectancy_delta_r >= 0.0);
}

#[tokio::test]
async fn test_service_activate_with_offline_benchmark_fallback() {
    let mut settings = Settings::default();
    settings.learning.enabled = true;
    settings.web_auth_token = "test_token_secret_1234567890".to_string();

    let service = WebTradingService::new(settings);

    // Baseline v1 is already seeded
    let baseline_content = include_str!("../prompt_engineering/strategy_v1.txt");
    let candidate_content = format!("{}\n## 改进补丁：针对趋势行情强化保护止损，严格过滤震荡区间中轴开仓。\n", baseline_content);

    let v2 = service
        .prompt_store
        .publish(STRATEGY_PROMPT_NAME, &candidate_content, "Candidate for deadlock test")
        .expect("publish v2");

    // Activating v2 without force would previously fail due to 0/20 live samples.
    // Now it falls back to ReplayEvaluator on benchmark episodes and succeeds!
    let res = service.activate_prompt_version(&v2, false).expect("activation must succeed via benchmark replay");

    assert_eq!(res["active"].as_str(), Some(v2.as_str()));
    assert_eq!(res["forced"].as_bool(), Some(false));
    assert_eq!(res["verdict"]["evaluation_mode"].as_str(), Some("benchmark_replay"));
}

#[tokio::test]
async fn test_replay_evaluator_handles_broken_baseline_and_chop_suite() {
    let dir = Path::new("records/benchmark_episodes");
    let episodes = load_benchmark_episodes(dir).expect("load episodes");

    // Baseline is broken, taking 6 losing trades (-1.0R each)
    let broken_baseline = "短暂测试 prompt";
    let candidate = include_str!("../prompt_engineering/strategy_v1.txt");

    let policy = EvaluationPolicy {
        min_samples: 20,
        min_expectancy_delta_r: 0.0,
        max_win_rate_drop: 0.05,
    };

    let verdict = ReplayEvaluator::evaluate_candidate_vs_baseline(
        candidate,
        "v2",
        broken_baseline,
        "v_broken",
        &episodes,
        &policy,
    );

    assert!(verdict.accepted, "Candidate should pass against broken baseline: {:?}", verdict.reasons);
    assert!(verdict.candidate.samples > 0);
    assert!(verdict.baseline.expectancy_r < 0.0);
    assert!(verdict.expectancy_delta_r > 2.0);

    // Also test a defensive suite containing only WAIT episodes
    let chop_episodes: Vec<_> = episodes.into_iter().filter(|e| e.expected_action == "WAIT").collect();
    assert!(!chop_episodes.is_empty());
    let defensive_verdict = ReplayEvaluator::evaluate_candidate_vs_baseline(
        candidate,
        "v2",
        broken_baseline,
        "v_broken",
        &chop_episodes,
        &policy,
    );
    assert!(defensive_verdict.accepted, "Candidate should pass when avoiding losses in chop suite: {:?}", defensive_verdict.reasons);
    assert_eq!(defensive_verdict.candidate.samples, 0);
    assert!(defensive_verdict.baseline.samples > 0);
}
