use okx_2pa_agent::ai::typesafe::{TypeSafeClient, TypeSafeQuestion};
use okx_2pa_agent::config::settings::Settings;
use okx_2pa_agent::data::base::{IndicatorBundle, KlineBar, KlineFrame};
use okx_2pa_agent::learning::hooks::{HookPipeline, PreExecutionContext, TypeSafeConfidenceGuardHook};
use okx_2pa_agent::orchestrator::two_stage::TwoStageOrchestrator;
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

#[tokio::test]
async fn test_live_typesafe_api_connectivity() {
    let settings = Settings::load_from_file_and_env("config/settings.json");
    if !settings.typesafe.enabled || settings.typesafe.api_key.trim().is_empty() {
        println!("TypeSafe is not enabled or api_key is empty in .env; skipping live API test.");
        return;
    }

    println!("============================================================");
    println!("1. Testing Live TypeSafe AI API Connectivity & Primitives");
    println!("============================================================");

    let client = TypeSafeClient::new(
        &settings.typesafe.model,
        &settings.typesafe.base_url,
        &settings.typesafe.api_key,
        settings.typesafe.timeout_seconds,
    );

    let state = json!({
        "symbol": "ETH-USDT-SWAP",
        "timeframe": "15m",
        "current_price": 2650.50,
        "ema20": 2640.20,
        "sma170": 2610.00,
        "recent_moves": "Price bounced twice off EMA20 with increasing green volume"
    });

    let mut questions = HashMap::new();
    let mut regime_opts = HashMap::new();
    regime_opts.insert("bull_trend".to_string(), "Price steadily holding above rising EMA20".to_string());
    regime_opts.insert("bear_trend".to_string(), "Price breaking below declining moving averages".to_string());
    regime_opts.insert("range_chop".to_string(), "Price sideways with overlapping bars".to_string());

    questions.insert(
        "regime".to_string(),
        TypeSafeQuestion::choice("Identify the trend regime of ETH", regime_opts),
    );

    questions.insert(
        "h2_confirmed".to_string(),
        TypeSafeQuestion::noul_with_criteria(
            "Does the recent price action confirm a valid H2 bullish pullback setup?",
            "Confirmed second entry with bullish continuation",
            "Unconfirmed or false breakout"
        ),
    );

    questions.insert(
        "signal_bar_score".to_string(),
        TypeSafeQuestion::score(
            "Score the bullish signal bar body conviction",
            vec![
                "0: Weak / doji / long upper shadow".to_string(),
                "1: Moderate green body".to_string(),
                "2: Strong engulfing or decisive close near high".to_string(),
            ],
        ),
    );

    println!("Sending request to {} with model: {}...", client.base_url, client.model);
    let resp = client.evaluate(&state, &questions).await.expect("TypeSafe evaluation failed");

    println!("✅ TypeSafe API responded successfully in {}ms!", resp.latency_ms);
    println!("   Model: {}", resp.model);
    println!("   Tokens: {} input, {} output", resp.usage.input_tokens, resp.usage.output_tokens);

    // Validate Choice answer
    let regime_ans = resp.answers.get("regime").expect("regime answer");
    println!("   [Choice] regime: {:?} (confidence: {:.2})", regime_ans.choice, regime_ans.effective_confidence());
    assert!(regime_ans.choice.is_some());
    assert!(regime_ans.effective_confidence() >= 0.0);

    // Validate Noul answer
    let noul_ans = resp.answers.get("h2_confirmed").expect("h2_confirmed answer");
    println!("   [Noul] h2_confirmed: {:.4} (effective_conf: {:.2})", noul_ans.noul.unwrap_or(0.0), noul_ans.effective_confidence());
    assert!(noul_ans.noul.is_some());

    // Validate Score answer
    let score_ans = resp.answers.get("signal_bar_score").expect("signal_bar_score answer");
    println!("   [Score] signal_bar_score: {:.2} (confidence: {:.2})", score_ans.score.unwrap_or(0.0), score_ans.effective_confidence());
    assert!(score_ans.score.is_some());

    println!("============================================================");
    println!("2. Testing Stage 1 TwoStageOrchestrator Integration");
    println!("============================================================");

    let orchestrator = TwoStageOrchestrator::new(settings.clone(), PathBuf::from("records"));
    assert!(orchestrator.typesafe_client.is_some(), "TwoStageOrchestrator typesafe_client should be initialized");

    // Build synthetic ETH 15m KlineFrame
    let mut bars = Vec::new();
    let base_ts = 1726880000000i64;
    let mut px = 2600.0;
    for i in 0..50 {
        px += if i > 35 { 2.5 } else { 1.0 };
        bars.push(KlineBar {
            seq: i,
            ts_open: base_ts + (i as i64) * 900_000,
            open: px - 1.0,
            high: px + 3.0,
            low: px - 2.0,
            close: px,
            volume: 100.0 + (i as f64) * 5.0,
            amount: 260000.0,
            pct_chg: Some(0.1),
            closed: true,
        });
    }
    bars.reverse(); // bars[0] is newest

    let ema_vec = vec![px - 2.0; 50];
    let sma170_vec = vec![px - 20.0; 50];
    let atr_vec = vec![25.0; 50];

    let frame = KlineFrame {
        symbol: "ETH-USDT-SWAP".to_string(),
        timeframe: "15m".to_string(),
        bars,
        indicators: IndicatorBundle {
            ema20: ema_vec,
            atr14: atr_vec,
            sma14: vec![px - 3.0; 50],
            sma170: sma170_vec,
            sma170_slope: vec![0.5; 50],
            dev170_pct: vec![1.2; 50],
        },
        snapshot_ts_local_ms: 1726880000000,
    };

    let hard_evidence = json!({
        "long": {
            "eligible": true,
            "evidence": {
                "reference_close": px,
                "atr": 25.0,
                "invalidation": px - 35.0,
                "target_bound": px + 55.0
            }
        },
        "short": {
            "eligible": false
        }
    });

    let s1_res = orchestrator.evaluate_typesafe_stage1("2pa_trend", &frame, None, &hard_evidence).await;
    assert!(s1_res.is_some(), "evaluate_typesafe_stage1 should succeed with live API");
    let (diagnosis, reply, _msgs) = s1_res.unwrap();

    println!("✅ Stage 1 TypeSafe diagnosis executed:");
    println!("   Regime: {}", diagnosis["market_regime"]);
    println!("   Confidence: {}", diagnosis["typesafe_confidence"]);
    println!("   Setup Noul: {}", diagnosis["typesafe_setup_noul"]);
    println!("   Gate Result: {}", diagnosis["gate_result"]);
    println!("   Reasoning: {}", diagnosis["reasoning"]);
    println!("   Latency: {}ms", reply.latency_ms);

    println!("============================================================");
    println!("3. Testing Reef TypeSafeConfidenceGuardHook Lifecycle");
    println!("============================================================");

    let guard = Arc::new(TypeSafeConfidenceGuardHook::new(settings.typesafe.min_confidence));
    let mut pipeline = HookPipeline::new();
    pipeline.add_hook(guard.clone());

    let ctx = PreExecutionContext {
        symbol: "ETH-USDT-SWAP".to_string(),
        timeframe: "15m".to_string(),
        trading_system: "2pa_trend".to_string(),
        decision: json!({
            "action": "OPEN",
            "typesafe_confidence": diagnosis["typesafe_confidence"]
        }),
        account_balance_usd: 10000.0,
        is_shadow_mode: false,
    };

    let hook_res = pipeline.run_pre_execution(&ctx);
    let conf_val = diagnosis["typesafe_confidence"].as_f64().unwrap_or(0.0);
    if conf_val >= settings.typesafe.min_confidence {
        assert!(hook_res.is_ok(), "High confidence decision should be approved by hook");
        println!("✅ Hook passed: confidence ({:.2}) >= threshold ({:.2})", conf_val, settings.typesafe.min_confidence);
    } else {
        assert!(hook_res.is_err(), "Low confidence decision should be blocked by hook");
        println!("✅ Hook intercepted: confidence ({:.2}) < threshold ({:.2}) (Risk limit guarded)", conf_val, settings.typesafe.min_confidence);
    }

    println!("============================================================");
    println!("4. Testing Backtest Engine Candidate Gating with TypeSafe");
    println!("============================================================");

    let mut btc_cfg = okx_2pa_agent::backtest::types::BacktestConfig::default();
    btc_cfg.use_typesafe = true;
    btc_cfg.strategy_id = "2pa_trend".to_string();
    btc_cfg.symbol = "ETH-USDT-SWAP".to_string();
    btc_cfg.timeframe = "15m".to_string();

    let cache = okx_2pa_agent::backtest::cache::DecisionCache::new(None);
    let engine = okx_2pa_agent::backtest::engine::BacktestEngine::new(btc_cfg, cache, None)
        .with_typesafe_client(Arc::new(client));

    let candidate_decision = engine.produce_candidate_decision(&frame, None, &hard_evidence, true).await;
    println!("✅ Backtest candidate decision evaluated via TypeSafe:");
    println!("   Action: {}", candidate_decision["decision"]["action"]);
    println!("   Order Type: {}", candidate_decision["decision"]["order_type"]);
    println!("   Entry: {}", candidate_decision["decision"]["entry_price"]);
    println!("   SL: {}", candidate_decision["decision"]["stop_loss_price"]);
    println!("   TP: {}", candidate_decision["decision"]["take_profit_price"]);
    println!("   Trade Confidence: {}", candidate_decision["decision"]["trade_confidence"]);
    println!("   TypeSafe Confidence: {:?}", candidate_decision["decision"]["typesafe_confidence"]);
    println!("   Reasoning: {}", candidate_decision["decision"]["reasoning"]);

    println!("============================================================");
    println!("🎉 All TypeSafe AI live tests successfully verified!");
    println!("============================================================");
}
