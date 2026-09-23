use okx_2pa_agent::ai::typesafe::TypeSafeClient;
use okx_2pa_agent::backtest::cache::DecisionCache;
use okx_2pa_agent::backtest::data::{fetch_candles_okx, generate_synthetic_candles_for_strategy, timeframe_to_ms};
use okx_2pa_agent::backtest::engine::BacktestEngine;
use okx_2pa_agent::backtest::types::BacktestConfig;
use okx_2pa_agent::config::settings::Settings;
use okx_2pa_agent::data::snapshot::build_analysis_frame;
use okx_2pa_agent::okx::client::OKXClient;
use std::sync::Arc;

#[tokio::test]
async fn test_backtest_eth_15m_1000_bars() {
    let settings = Settings::load_from_file_and_env("config/settings.json");

    println!("============================================================");
    println!("🧪 Testing Backtest: ETH-USDT-SWAP, 15m, 1000 Bars");
    println!("============================================================");

    let symbol = "ETH-USDT-SWAP".to_string();
    let timeframe = "15m".to_string();
    let total_bars = 1000usize;
    let interval_ms = timeframe_to_ms(&timeframe);

    let creds = if settings.is_okx_configured() {
        Some(okx_2pa_agent::okx::client::OKXCredentials::new(
            &settings.okx.api_key,
            &settings.okx.secret_key,
            &settings.okx.passphrase,
        ))
    } else {
        None
    };

    let okx_client = OKXClient::new(
        &settings.okx.base_url,
        creds,
        settings.okx.demo_trading,
        15,
    );

    println!("Attempting to fetch {} historical 15m bars for {} from OKX...", total_bars, symbol);
    let bars = match fetch_candles_okx(&okx_client, &symbol, &timeframe, total_bars, None).await {
        Ok(b) if b.len() >= 250 => {
            println!("✅ Successfully retrieved {} real historical candles from OKX!", b.len());
            b
        }
        Ok(b) => {
            println!("⚠️ Retrieved only {} bars from OKX (less than required warmup); using 1000 synthetic ETH bars", b.len());
            let start_ts = chrono::Utc::now().timestamp_millis() - (total_bars as i64) * interval_ms;
            generate_synthetic_candles_for_strategy("2pa_trend", total_bars, 2650.0, interval_ms, start_ts)
        }
        Err(e) => {
            println!("⚠️ OKX historical fetch failed or timed out ({:?}). Using 1000 calibrated synthetic ETH bars (Base Price: $2650).", e);
            let start_ts = chrono::Utc::now().timestamp_millis() - (total_bars as i64) * interval_ms;
            generate_synthetic_candles_for_strategy("2pa_trend", total_bars, 2650.0, interval_ms, start_ts)
        }
    };

    println!("Total K-line bars available for backtest: {}", bars.len());
    let (t_first, t_last) = (bars.first().unwrap().ts_open, bars.last().unwrap().ts_open);
    println!("Time range: {} -> {} (approx {:.1} days of 15m data)", 
        chrono::DateTime::from_timestamp_millis(t_first).map(|t| t.to_rfc3339()).unwrap_or_default(),
        chrono::DateTime::from_timestamp_millis(t_last).map(|t| t.to_rfc3339()).unwrap_or_default(),
        (t_last - t_first) as f64 / 86_400_000.0
    );

    // 1. Detailed Strategy Diagnostics Scan across all 1000 bars
    println!("\n🔍 Strategy Diagnostics Screening over 1000 bars:");
    for strat in ["2pa_source", "2pa_trend", "dog_trend", "dog_reversion"] {
        let mut eligible_longs = 0;
        let mut eligible_shorts = 0;
        let mut sample_details = Vec::new();

        let mut rejection_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

        for i in 220..bars.len() {
            let slice = &bars[..=i];
            let mut desc = slice.to_vec();
            desc.reverse();

            if let Some(frame) = build_analysis_frame(&desc, 50, &symbol, &timeframe, None) {
                let htf_closed = BacktestEngine::resample_closed_htf(&bars, &timeframe, "1H", i);
                let htf_frame_opt = if htf_closed.len() >= 25 {
                    let mut htf_desc = htf_closed;
                    htf_desc.reverse();
                    build_analysis_frame(&htf_desc, 20, &symbol, "1H", None)
                } else {
                    None
                };

                let diag = okx_2pa_agent::strategies::diagnostics(strat, &frame, htf_frame_opt.as_ref());
                let is_long = diag["long"]["eligible"] == true;
                let is_short = diag["short"]["eligible"] == true;
                if is_long {
                    eligible_longs += 1;
                    if sample_details.len() < 3 {
                        sample_details.push(format!("Bar #{} Long: ref={:.1}, invalidation={:.1}", i, diag["long"]["evidence"]["reference_close"], diag["long"]["evidence"]["invalidation"]));
                    }
                } else if let Some(r) = diag["long"]["reason"].as_str() {
                    *rejection_counts.entry(r.to_string()).or_insert(0) += 1;
                }
                if is_short {
                    eligible_shorts += 1;
                    if sample_details.len() < 3 {
                        sample_details.push(format!("Bar #{} Short: ref={:.1}, invalidation={:.1}", i, diag["short"]["evidence"]["reference_close"], diag["short"]["evidence"]["invalidation"]));
                    }
                } else if let Some(r) = diag["short"]["reason"].as_str() {
                    *rejection_counts.entry(format!("[Short] {}", r)).or_insert(0) += 1;
                }
            }
        }
        println!("   - [{}] Eligible Longs: {}, Eligible Shorts: {} (Total Candidates: {})", strat, eligible_longs, eligible_shorts, eligible_longs + eligible_shorts);
        for s in sample_details {
            println!("       ↳ {}", s);
        }
        println!("       ↳ Top Rejection Reasons ({}):", strat);
        let mut sorted_rejections: Vec<_> = rejection_counts.into_iter().collect();
        sorted_rejections.sort_by(|a, b| b.1.cmp(&a.1));
        for (reason, count) in sorted_rejections.iter().take(5) {
            println!("           • {}: {} bars ({:.1}%)", reason, count, (*count as f64 / 780.0) * 100.0);
        }
    }

    // Diagnostic inspection of Bar #670 and #732
    {
        let i = 670;
        let slice = &bars[..=i];
        let mut desc = slice.to_vec();
        desc.reverse();
        if let Some(frame) = build_analysis_frame(&desc, 50, &symbol, &timeframe, None) {
            let htf_closed = BacktestEngine::resample_closed_htf(&bars, &timeframe, "1H", i);
            let htf_frame_opt = if htf_closed.len() >= 25 {
                let mut htf_desc = htf_closed;
                htf_desc.reverse();
                build_analysis_frame(&htf_desc, 20, &symbol, "1H", None)
            } else {
                None
            };
            let diag = okx_2pa_agent::strategies::diagnostics("2pa_trend", &frame, htf_frame_opt.as_ref());
            let mut cfg = BacktestConfig::default();
            cfg.symbol = symbol.clone();
            cfg.ct_val = 0.1;
            let engine = BacktestEngine::new(cfg.clone(), DecisionCache::new(None), None);
            let synthesized = engine.produce_candidate_decision(&frame, htf_frame_opt.as_ref(), &diag, true).await;
            println!("\n🔍 Bar #670 Deep Inspection:");
            println!("   Synthesized Decision: {}", synthesized["decision"]);
            let mut wrapper = synthesized.clone();
            let stage1_mock = serde_json::json!({
                "gate_result": "proceed",
                "program_candidates": diag
            });
            okx_2pa_agent::strategies::enforce("2pa_trend", &frame, htf_frame_opt.as_ref(), &stage1_mock, &mut wrapper, None);
            println!("   Enforced Decision: {}", wrapper["decision"]);
            println!("   Program Validation: {}", wrapper["program_validation"]);

            let account = okx_2pa_agent::backtest::account::VirtualAccount::new(&cfg);
            let lots = account.calculate_order_size(
                wrapper["decision"]["entry_price"].as_f64().unwrap_or(0.0),
                wrapper["decision"]["stop_loss_price"].as_f64().unwrap_or(0.0),
                1.0,
                50.0,
            );
            println!("   Lots calculated: {} (min lot_sz: {})", lots, account.lot_sz);
        }

        let i = 732;
        let slice = &bars[..=i];
        let mut desc = slice.to_vec();
        desc.reverse();
        if let Some(frame) = build_analysis_frame(&desc, 50, &symbol, &timeframe, None) {
            let htf_closed = BacktestEngine::resample_closed_htf(&bars, &timeframe, "1H", i);
            let htf_frame_opt = if htf_closed.len() >= 25 {
                let mut htf_desc = htf_closed;
                htf_desc.reverse();
                build_analysis_frame(&htf_desc, 20, &symbol, "1H", None)
            } else {
                None
            };
            let diag = okx_2pa_agent::strategies::diagnostics("2pa_trend", &frame, htf_frame_opt.as_ref());
            let mut cfg = BacktestConfig::default();
            cfg.symbol = symbol.clone();
            cfg.ct_val = 0.1;
            let engine = BacktestEngine::new(cfg.clone(), DecisionCache::new(None), None);
            let synthesized = engine.produce_candidate_decision(&frame, htf_frame_opt.as_ref(), &diag, true).await;
            println!("\n🔍 Bar #732 Deep Inspection:");
            println!("   Synthesized Decision: {}", synthesized["decision"]);
            let mut wrapper = synthesized.clone();
            let stage1_mock = serde_json::json!({
                "gate_result": "proceed",
                "program_candidates": diag
            });
            okx_2pa_agent::strategies::enforce("2pa_trend", &frame, htf_frame_opt.as_ref(), &stage1_mock, &mut wrapper, None);
            println!("   Enforced Decision: {}", wrapper["decision"]);
            println!("   Program Validation: {}", wrapper["program_validation"]);

            let account = okx_2pa_agent::backtest::account::VirtualAccount::new(&cfg);
            let lots = account.calculate_order_size(
                wrapper["decision"]["entry_price"].as_f64().unwrap_or(0.0),
                wrapper["decision"]["stop_loss_price"].as_f64().unwrap_or(0.0),
                1.0,
                50.0,
            );
            println!("   Lots calculated: {} (min lot_sz: {})", lots, account.lot_sz);
        }
    }

    // 2. Prepare TypeSafe client and AI client
    let typesafe_client = if settings.typesafe.enabled && !settings.typesafe.api_key.trim().is_empty() {
        println!("\nTypeSafe AI client initialized (model: {})", settings.typesafe.model);
        Some(Arc::new(TypeSafeClient::new(
            &settings.typesafe.model,
            &settings.typesafe.base_url,
            &settings.typesafe.api_key,
            settings.typesafe.timeout_seconds,
        )))
    } else {
        None
    };

    let _ai_client = if !settings.provider.api_key.trim().is_empty() {
        println!("\nAI client initialized (model: {}, base_url: {})", settings.provider.model, settings.provider.base_url);
        Some(Arc::new(okx_2pa_agent::ai::client::AIClient::new(
            &settings.provider.model,
            &settings.provider.base_url,
            &settings.provider.api_key,
            settings.provider.thinking,
            &settings.provider.reasoning_effort,
            settings.provider.stage_timeout_seconds,
        )))
    } else {
        None
    };

    // 3. Run backtest across strategies and compare
    let strategies = ["2pa_source", "2pa_trend", "dog_trend", "dog_reversion"];
    for strat in strategies {
        for use_ts in [false, true] {
            if use_ts && typesafe_client.is_none() {
                continue;
            }

            let mut config = BacktestConfig::default();
            config.symbol = symbol.clone();
            config.timeframe = timeframe.clone();
            config.strategy_id = strat.to_string();
            config.initial_capital = 10000.0;
            config.risk_percent = 1.0;
            config.leverage = 5.0;
            config.ct_val = 0.1; // OKX contract value: 0.1 ETH per contract
            config.lot_sz = 1.0;
            config.max_bars = total_bars;
            config.use_mechanical_exit = true;
            config.use_cache = true;
            config.use_typesafe = use_ts;
            config.allow_llm_calls = false;

            let cache = DecisionCache::new(None);
            let mut engine = BacktestEngine::new(config.clone(), cache, None);
            if use_ts {
                if let Some(ref tc) = typesafe_client {
                    engine = engine.with_typesafe_client(tc.clone());
                }
            }

            let job_id = format!("JOB-ETH-{}-ts{}", strat, use_ts);
            let report = engine
                .run(&job_id, &bars, None)
                .await
                .expect("Backtest run must succeed");

            let mode_label = if use_ts { "TypeSafe Jev Gated" } else { "Programmatic Pure Gating" };
            println!("\n============================================================");
            println!("📊 Report: ETH 15m | Strategy: {} | Mode: {}", strat, mode_label);
            println!("============================================================");
            println!("   Total Bars:            {} (Skipped chop: {} / {:.1}%)", 
                report.metrics.total_bars_processed, 
                report.metrics.gated_bars_skipped, 
                (report.metrics.gated_bars_skipped as f64 / report.metrics.total_bars_processed.max(1) as f64) * 100.0
            );
            println!("   Total Trades:          {}", report.metrics.total_trades);
            println!("   Win / Loss / BE:       {} / {} / {}", report.metrics.winning_trades, report.metrics.losing_trades, report.metrics.break_even_trades);
            println!("   Win Rate:              {:.1}%", report.metrics.win_rate);
            println!("   Profit Factor:         {:.2}", report.metrics.profit_factor);
            println!("   Expectancy (R):        {:+.2}R", report.metrics.expectancy_r);
            println!("   Net Profit:            ${:+.2} ({:+.2}%)", report.metrics.net_profit, report.metrics.net_profit_pct);
            println!("   Max Drawdown:          ${:.2} ({:.2}%)", report.metrics.max_drawdown_amount, report.metrics.max_drawdown_pct);
            println!("   Sharpe Ratio:          {:.2}", report.metrics.sharpe_ratio);
            println!("   Sortino Ratio:         {:.2}", report.metrics.sortino_ratio);
            println!("   Fees Paid:             ${:.2}", report.metrics.total_fees_paid);
            println!("   Execution Duration:    {}ms", report.execution_duration_ms);

            if !report.trades.is_empty() {
                println!("   Executed Trades Details (first 5 of {}):", report.trades.len());
                for (idx, trade) in report.trades.iter().take(5).enumerate() {
                    println!(
                        "     [{}] {} {} @ {:.2} -> exit @ {:.2} ({}) | PnL: ${:+.2} ({:+.2}R) | Hold: {} bars",
                        idx + 1,
                        trade.direction,
                        trade.strategy_id,
                        trade.entry_price,
                        trade.exit_price,
                        trade.exit_reason,
                        trade.net_pnl,
                        trade.pnl_r,
                        trade.hold_bars
                    );
                }
            }
        }
    }

    // 4. Trending Market Benchmark Test (1000 ETH bars with active 2PA trend setups)
    println!("\n============================================================");
    println!("📈 Running Trend-Benchmark Backtest: ETH 15m 1000 Bars (2PA Trend Setups)");
    println!("============================================================");
    let start_ts = chrono::Utc::now().timestamp_millis() - (total_bars as i64) * interval_ms;
    let trend_bars = generate_synthetic_candles_for_strategy("2pa_trend", total_bars, 2650.0, interval_ms, start_ts);

    let mut trend_config = BacktestConfig::default();
    trend_config.symbol = symbol.clone();
    trend_config.timeframe = timeframe.clone();
    trend_config.strategy_id = "2pa_trend".to_string();
    trend_config.initial_capital = 10000.0;
    trend_config.risk_percent = 1.0;
    trend_config.leverage = 5.0;
    trend_config.ct_val = 0.1;
    trend_config.lot_sz = 1.0;
    trend_config.max_bars = total_bars;
    trend_config.use_mechanical_exit = true;
    trend_config.use_cache = false;
    trend_config.use_typesafe = false;
    trend_config.allow_llm_calls = false;

    let trend_engine = BacktestEngine::new(trend_config.clone(), DecisionCache::new(None), None);
    let trend_report = trend_engine
        .run("JOB-ETH-TREND-BENCH", &trend_bars, None)
        .await
        .expect("Trend backtest must succeed");

    println!("   Total Bars Processed:  {}", trend_report.metrics.total_bars_processed);
    println!("   Executed Trades:       {}", trend_report.metrics.total_trades);
    println!("   Win Rate:              {:.1}%", trend_report.metrics.win_rate);
    println!("   Profit Factor:         {:.2}", trend_report.metrics.profit_factor);
    println!("   Expectancy (R):        {:+.2}R", trend_report.metrics.expectancy_r);
    println!("   Net Profit:            ${:+.2} ({:+.2}%)", trend_report.metrics.net_profit, trend_report.metrics.net_profit_pct);
    println!("   Max Drawdown:          ${:.2} ({:.2}%)", trend_report.metrics.max_drawdown_amount, trend_report.metrics.max_drawdown_pct);
    println!("   Sharpe Ratio:          {:.2}", trend_report.metrics.sharpe_ratio);

    if !trend_report.trades.is_empty() {
        println!("   Sample Executed Trades:");
        for (idx, trade) in trend_report.trades.iter().take(5).enumerate() {
            println!(
                "     [{}] {} {} @ {:.2} -> exit @ {:.2} ({}) | PnL: ${:+.2} ({:+.2}R) | Hold: {} bars",
                idx + 1,
                trade.direction,
                trade.strategy_id,
                trade.entry_price,
                trade.exit_price,
                trade.exit_reason,
                trade.net_pnl,
                trade.pnl_r,
                trade.hold_bars
            );
        }
    }

    assert!(trend_report.metrics.total_trades > 0, "Trend benchmark must generate trades");

    // 5. Source 2PA Benchmark Test (1000 ETH bars)
    println!("\n============================================================");
    println!("📈 Running Source 2PA Benchmark Backtest: ETH 15m 1000 Bars (2PA Source)");
    println!("============================================================");
    let mut source_config = BacktestConfig::default();
    source_config.symbol = symbol.clone();
    source_config.timeframe = timeframe.clone();
    source_config.strategy_id = "2pa_source".to_string();
    source_config.initial_capital = 10000.0;
    source_config.risk_percent = 1.0;
    source_config.leverage = 5.0;
    source_config.ct_val = 0.1;
    source_config.lot_sz = 1.0;
    source_config.max_bars = total_bars;
    source_config.use_mechanical_exit = true;
    source_config.use_cache = false;
    source_config.use_typesafe = false;
    source_config.allow_llm_calls = false;

    let source_bars = generate_synthetic_candles_for_strategy("2pa_source", total_bars, 2650.0, interval_ms, start_ts);
    let source_engine = BacktestEngine::new(source_config.clone(), DecisionCache::new(None), None);
    let source_report = source_engine
        .run("JOB-ETH-2PA-SOURCE-BENCH", &source_bars, None)
        .await
        .expect("2PA source benchmark must succeed");

    println!("   Total Bars Processed:  {}", source_report.metrics.total_bars_processed);
    println!("   Executed Trades:       {}", source_report.metrics.total_trades);
    println!("   Win Rate:              {:.1}%", source_report.metrics.win_rate);
    println!("   Profit Factor:         {:.2}", source_report.metrics.profit_factor);
    println!("   Expectancy (R):        {:+.2}R", source_report.metrics.expectancy_r);
    println!("   Net Profit:            ${:+.2} ({:+.2}%)", source_report.metrics.net_profit, source_report.metrics.net_profit_pct);
    println!("   Max Drawdown:          ${:.2} ({:.2}%)", source_report.metrics.max_drawdown_amount, source_report.metrics.max_drawdown_pct);
    println!("   Sharpe Ratio:          {:.2}", source_report.metrics.sharpe_ratio);

    if !source_report.trades.is_empty() {
        println!("   Sample Executed Trades (2PA Source):");
        for (idx, trade) in source_report.trades.iter().take(5).enumerate() {
            println!(
                "     [{}] {} {} @ {:.2} -> exit @ {:.2} ({}) | PnL: ${:+.2} ({:+.2}R) | Hold: {} bars",
                idx + 1,
                trade.direction,
                trade.strategy_id,
                trade.entry_price,
                trade.exit_price,
                trade.exit_reason,
                trade.net_pnl,
                trade.pnl_r,
                trade.hold_bars
            );
        }
    }

    assert!(source_report.metrics.total_trades > 0, "2PA Source benchmark must generate trades");

    // 6. Dog Reversion Benchmark Test (1000 ETH bars)
    println!("\n============================================================");
    println!("📈 Running Dog Reversion Benchmark Backtest: ETH 15m 1000 Bars (Dog Reversion)");
    println!("============================================================");
    let mut dog_rev_config = BacktestConfig::default();
    dog_rev_config.symbol = symbol.clone();
    dog_rev_config.timeframe = timeframe.clone();
    dog_rev_config.strategy_id = "dog_reversion".to_string();
    dog_rev_config.initial_capital = 10000.0;
    dog_rev_config.risk_percent = 1.0;
    dog_rev_config.leverage = 5.0;
    dog_rev_config.ct_val = 0.1;
    dog_rev_config.lot_sz = 1.0;
    dog_rev_config.max_bars = total_bars;
    dog_rev_config.use_mechanical_exit = true;
    dog_rev_config.use_cache = false;
    dog_rev_config.use_typesafe = false;
    dog_rev_config.allow_llm_calls = false;

    let dog_rev_bars = generate_synthetic_candles_for_strategy("dog_reversion", total_bars, 2650.0, interval_ms, start_ts);
    let dog_rev_engine = BacktestEngine::new(dog_rev_config.clone(), DecisionCache::new(None), None);
    let dog_rev_report = dog_rev_engine
        .run("JOB-ETH-DOG-REV-BENCH", &dog_rev_bars, None)
        .await
        .expect("Dog reversion benchmark must succeed");

    println!("   Total Bars Processed:  {}", dog_rev_report.metrics.total_bars_processed);
    println!("   Executed Trades:       {}", dog_rev_report.metrics.total_trades);
    println!("   Win Rate:              {:.1}%", dog_rev_report.metrics.win_rate);
    println!("   Profit Factor:         {:.2}", dog_rev_report.metrics.profit_factor);
    println!("   Expectancy (R):        {:+.2}R", dog_rev_report.metrics.expectancy_r);
    println!("   Net Profit:            ${:+.2} ({:+.2}%)", dog_rev_report.metrics.net_profit, dog_rev_report.metrics.net_profit_pct);
    println!("   Max Drawdown:          ${:.2} ({:.2}%)", dog_rev_report.metrics.max_drawdown_amount, dog_rev_report.metrics.max_drawdown_pct);
    println!("   Sharpe Ratio:          {:.2}", dog_rev_report.metrics.sharpe_ratio);

    if !dog_rev_report.trades.is_empty() {
        println!("   Sample Executed Trades (Dog Reversion):");
        for (idx, trade) in dog_rev_report.trades.iter().take(5).enumerate() {
            println!(
                "     [{}] {} {} @ {:.2} -> exit @ {:.2} ({}) | PnL: ${:+.2} ({:+.2}R) | Hold: {} bars",
                idx + 1,
                trade.direction,
                trade.strategy_id,
                trade.entry_price,
                trade.exit_price,
                trade.exit_reason,
                trade.net_pnl,
                trade.pnl_r,
                trade.hold_bars
            );
        }
    }

    assert!(dog_rev_report.metrics.total_trades > 0, "Dog reversion benchmark must generate trades");

    // 7. Dog Trend Benchmark Test (1000 ETH bars)
    println!("\n============================================================");
    println!("📈 Running Dog Trend Benchmark Backtest: ETH 15m 1000 Bars (Dog Trend)");
    println!("============================================================");
    let mut dog_trend_config = BacktestConfig::default();
    dog_trend_config.symbol = symbol.clone();
    dog_trend_config.timeframe = timeframe.clone();
    dog_trend_config.strategy_id = "dog_trend".to_string();
    dog_trend_config.initial_capital = 10000.0;
    dog_trend_config.risk_percent = 1.0;
    dog_trend_config.leverage = 5.0;
    dog_trend_config.ct_val = 0.1;
    dog_trend_config.lot_sz = 1.0;
    dog_trend_config.max_bars = total_bars;
    dog_trend_config.use_mechanical_exit = true;
    dog_trend_config.use_cache = false;
    dog_trend_config.use_typesafe = false;
    dog_trend_config.allow_llm_calls = false;

    let dog_trend_bars = generate_synthetic_candles_for_strategy("dog_trend", total_bars, 2650.0, interval_ms, start_ts);
    let dog_trend_engine = BacktestEngine::new(dog_trend_config.clone(), DecisionCache::new(None), None);
    let dog_trend_report = dog_trend_engine
        .run("JOB-ETH-DOG-TREND-BENCH", &dog_trend_bars, None)
        .await
        .expect("Dog trend benchmark must succeed");

    println!("   Total Bars Processed:  {}", dog_trend_report.metrics.total_bars_processed);
    println!("   Executed Trades:       {}", dog_trend_report.metrics.total_trades);
    println!("   Win Rate:              {:.1}%", dog_trend_report.metrics.win_rate);
    println!("   Profit Factor:         {:.2}", dog_trend_report.metrics.profit_factor);
    println!("   Expectancy (R):        {:+.2}R", dog_trend_report.metrics.expectancy_r);
    println!("   Net Profit:            ${:+.2} ({:+.2}%)", dog_trend_report.metrics.net_profit, dog_trend_report.metrics.net_profit_pct);
    println!("   Max Drawdown:          ${:.2} ({:.2}%)", dog_trend_report.metrics.max_drawdown_amount, dog_trend_report.metrics.max_drawdown_pct);
    println!("   Sharpe Ratio:          {:.2}", dog_trend_report.metrics.sharpe_ratio);

    if !dog_trend_report.trades.is_empty() {
        println!("   Sample Executed Trades (Dog Trend):");
        for (idx, trade) in dog_trend_report.trades.iter().take(5).enumerate() {
            println!(
                "     [{}] {} {} @ {:.2} -> exit @ {:.2} ({}) | PnL: ${:+.2} ({:+.2}R) | Hold: {} bars",
                idx + 1,
                trade.direction,
                trade.strategy_id,
                trade.entry_price,
                trade.exit_price,
                trade.exit_reason,
                trade.net_pnl,
                trade.pnl_r,
                trade.hold_bars
            );
        }
    }

    assert!(dog_trend_report.metrics.total_trades > 0, "Dog trend benchmark must generate trades");

    println!("============================================================");
    println!("🎉 All ETH 15m 1000-candle backtests completed successfully!");
    println!("============================================================");
}
