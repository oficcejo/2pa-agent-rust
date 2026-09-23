use crate::ai::client::AIClient;
use crate::ai::prompt_assembler::{
    build_stage1_prompt_for_system, build_stage2_prompt_with_strategy,
    resolve_strategy_prompt_for_system, Stage2PromptRequest,
};
use crate::config::settings::Settings;
use crate::data::base::{KlineFrame, PositionContext};
use crate::orchestrator::validation_retry::{call_and_validate_stage1, call_and_validate_stage2};
use crate::records::history::save_record;
use crate::records::schema::{AnalysisRecord, RecordMeta};
use crate::util::mask::mask_secret;
use crate::util::timefmt::{now_local_iso, now_local_ms};
use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use std::path::PathBuf;
use tracing::{info, warn};

#[derive(Debug, Clone)]
pub struct TwoStageOrchestrator {
    pub ai_client: AIClient,
    pub typesafe_client: Option<crate::ai::typesafe::TypeSafeClient>,
    pub prompt_dir: Option<PathBuf>,
    pub experience_dir: Option<PathBuf>,
    pub records_dir: PathBuf,
    pub settings: Settings,
}

impl TwoStageOrchestrator {
    pub fn new(settings: Settings, records_dir: PathBuf) -> Self {
        let ai_client = AIClient::new(
            &settings.provider.model,
            &settings.provider.base_url,
            &settings.provider.api_key,
            settings.provider.thinking,
            &settings.provider.reasoning_effort,
            settings.provider.stage_timeout_seconds,
        );

        let typesafe_client = if settings.is_typesafe_configured() {
            Some(crate::ai::typesafe::TypeSafeClient::new(
                &settings.typesafe.model,
                &settings.typesafe.base_url,
                &settings.typesafe.api_key,
                settings.typesafe.timeout_seconds,
            ))
        } else {
            None
        };

        Self {
            ai_client,
            typesafe_client,
            prompt_dir: Some(PathBuf::from("prompt_engineering")),
            experience_dir: Some(PathBuf::from("experience")),
            records_dir,
            settings,
        }
    }

    /// Experience injection is opt-in and gated by the master switch, so
    /// enabling reconciliation alone never changes what the model sees.
    pub fn experience_max_entries(&self) -> usize {
        let learning = &self.settings.learning;
        if !learning.enabled || !learning.read_experience {
            0
        } else {
            learning.experience_max_entries
        }
    }

    pub async fn run_analysis(&self, frame: &KlineFrame) -> Result<AnalysisRecord> {
        let system = self.settings.general.trading_system.clone();
        self.run_analysis_with_system_and_pos(frame, &system, None, None).await
    }

    pub async fn run_analysis_with_system(&self, frame: &KlineFrame, system: &str) -> Result<AnalysisRecord> {
        self.run_analysis_with_system_and_pos(frame, system, None, None).await
    }

    pub async fn run_analysis_with_system_and_pos(
        &self,
        frame: &KlineFrame,
        system: &str,
        pos_ctx: Option<&PositionContext>,
        htf_context: Option<&str>,
    ) -> Result<AnalysisRecord> {
        self.run_analysis_with_market_context(frame, system, pos_ctx, htf_context, None).await
    }

    pub async fn run_analysis_with_market_context(
        &self, frame: &KlineFrame, system: &str, pos_ctx: Option<&PositionContext>,
        htf_context: Option<&str>, htf_frame: Option<&KlineFrame>,
    ) -> Result<AnalysisRecord> {
        let system = crate::strategies::canonical(system).ok_or_else(|| anyhow!("未知交易系统"))?;

        info!("Starting Stage 1 analysis for {} ({}) using system [{}]...", frame.symbol, frame.timeframe, system);

        // One stable id per decision; the reconciler uses it to attach the
        // eventual outcome to the analysis that caused it.
        let record_id = uuid::Uuid::new_v4().simple().to_string();
        let strategy_prompt = resolve_strategy_prompt_for_system(system, self.prompt_dir.as_deref());

        let hard_evidence = crate::strategies::diagnostics(system, frame, htf_frame);
        let context = format!("{}\n程序候选证据（不得篡改）：{}", htf_context.unwrap_or(""), hard_evidence);
        let htf_context = Some(context.as_str());
        let (mut stage1_diagnosis, stage1_reply, stage1_messages) = if self.typesafe_client.is_some() {
            if let Some(res) = self.evaluate_typesafe_stage1(system, frame, htf_frame, &hard_evidence).await {
                res
            } else if self.settings.typesafe.fail_closed {
                warn!(
                    "TypeSafe evaluation unavailable and fail_closed=true; forcing WAIT for {} {}",
                    frame.symbol, frame.timeframe
                );
                let diagnosis = serde_json::json!({
                    "market_regime": "unknown",
                    "typesafe_evaluated": true,
                    "typesafe_error": true,
                    "gate_result": "wait",
                    "reasoning": "TypeSafe 评估失败且 fail_closed=true，本次观望，不回落主模型",
                    "program_candidates": hard_evidence
                });
                let reply = crate::ai::client::LLMReply {
                    content: diagnosis.to_string(),
                    reasoning_content: Some("TypeSafe fail_closed".to_string()),
                    usage: crate::ai::client::Usage::default(),
                    latency_ms: 0,
                };
                let messages = vec![
                    crate::ai::client::ChatMessage {
                        role: "system".to_string(),
                        content: "TypeSafe fail_closed".to_string(),
                    },
                    crate::ai::client::ChatMessage {
                        role: "assistant".to_string(),
                        content: reply.content.clone(),
                    },
                ];
                (diagnosis, reply, messages)
            } else {
                info!("TypeSafe evaluation fallback to standard LLM Stage 1 diagnosis");
                let stage1_prompt = build_stage1_prompt_for_system(system, frame, self.prompt_dir.as_deref(), htf_context);
                call_and_validate_stage1(
                    &self.ai_client,
                    &stage1_prompt,
                    self.settings.validation.retry_max,
                ).await.context("第一阶段市场分析失败")?
            }
        } else {
            let stage1_prompt = build_stage1_prompt_for_system(system, frame, self.prompt_dir.as_deref(), htf_context);
            call_and_validate_stage1(
                &self.ai_client,
                &stage1_prompt,
                self.settings.validation.retry_max,
            ).await.context("第一阶段市场分析失败")?
        };

        stage1_diagnosis["program_candidates"] = hard_evidence;
        if stage1_diagnosis["program_candidates"]["long"]["eligible"] != true
            && stage1_diagnosis["program_candidates"]["short"]["eligible"] != true {
            stage1_diagnosis["gate_result"] = serde_json::json!("wait");
        }
        info!("Stage 1 diagnosis complete. Starting Stage 2 decision for system [{}]...", system);

        let (stage2_prompt, strategies_used, experiences_loaded) = build_stage2_prompt_with_strategy(
            &Stage2PromptRequest {
                system,
                frame,
                stage1_diagnosis: &stage1_diagnosis,
                decision_stance: &self.settings.general.decision_stance,
                strategy_prompt: &strategy_prompt.content,
                experience_dir: self.experience_dir.as_deref(),
                experience_max_entries: self.experience_max_entries(),
                position_context: pos_ctx,
                htf_context,
            },
        );

        let (mut stage2_decision, stage2_reply, stage2_messages) = call_and_validate_stage2(
            &self.ai_client,
            &stage2_prompt,
            self.settings.validation.retry_max,
            Some(&stage1_diagnosis),
        ).await.context("第二阶段交易决策失败")?;

        crate::strategies::enforce(system, frame, htf_frame, &stage1_diagnosis, &mut stage2_decision, pos_ctx);
        info!("Stage 2 decision complete. Building AnalysisRecord...");

        let kline_json = serde_json::to_value(&frame.bars).unwrap_or(Value::Array(Vec::new()));
        let kline_data = match kline_json {
            Value::Array(arr) => arr,
            _ => Vec::new(),
        };

        let stage1_msg_values: Vec<Value> = stage1_messages
            .into_iter()
            .map(|m| serde_json::json!({ "role": m.role, "content": m.content }))
            .collect();

        let stage2_msg_values: Vec<Value> = stage2_messages
            .into_iter()
            .map(|m| serde_json::json!({ "role": m.role, "content": m.content }))
            .collect();

        let exp_loaded_values: Vec<Value> = experiences_loaded
            .into_iter()
            .map(|e| serde_json::json!({
                "filename": e.filename,
                "case_type": e.case_type,
                "cycle_position": e.cycle_position,
                "content": e.content
            }))
            .collect();

        let total_prompt = stage1_reply.usage.prompt_tokens + stage2_reply.usage.prompt_tokens;
        let total_completion = stage1_reply.usage.completion_tokens + stage2_reply.usage.completion_tokens;

        let pos_val = pos_ctx.and_then(|p| serde_json::to_value(p).ok());

        let record = AnalysisRecord {
            meta: RecordMeta {
                timestamp_local_iso: now_local_iso(),
                timestamp_local_ms: now_local_ms(),
                symbol: frame.symbol.clone(),
                timeframe: frame.timeframe.clone(),
                bar_count: frame.bars.len(),
                ai_provider: serde_json::json!({
                    "model": self.settings.provider.model,
                    "base_url": self.settings.provider.base_url,
                    "api_key": mask_secret(&self.settings.provider.api_key),
                }),
                decision_stance: self.settings.general.decision_stance.clone(),
                trading_system: system.to_string(),
                record_id: record_id.clone(),
                prompt_version: strategy_prompt.version.clone(),
                prompt_hash: strategy_prompt.hash.clone(),
            },
            kline_data,
            htf_text: context,
            stage1_messages: stage1_msg_values,
            stage1_response: Some(serde_json::json!({
                "content": stage1_reply.content,
                "reasoning_content": stage1_reply.reasoning_content,
                "usage": stage1_reply.usage,
                "latency_ms": stage1_reply.latency_ms,
            })),
            stage1_diagnosis: Some(stage1_diagnosis),
            stage2_messages: stage2_msg_values,
            stage2_response: Some(serde_json::json!({
                "content": stage2_reply.content,
                "reasoning_content": stage2_reply.reasoning_content,
                "usage": stage2_reply.usage,
                "latency_ms": stage2_reply.latency_ms,
            })),
            stage2_decision: Some(stage2_decision),
            strategy_files_used: strategies_used,
            experience_loaded: exp_loaded_values,
            position_context: pos_val,
            exception: None,
            usage_total: serde_json::json!({
                "prompt_tokens": total_prompt,
                "completion_tokens": total_completion,
                "total_tokens": total_prompt + total_completion,
            }),
        };

        save_record(&self.records_dir, &record).context("保存决策记录失败，停止执行")?;
        Ok(record)
    }

    /// Evaluates market condition using TypeSafe System One (Jev model).
    /// Returns Stage 1 diagnosis, mock LLM reply, and synthetic messages.
    pub async fn evaluate_typesafe_stage1(
        &self,
        system: &str,
        frame: &KlineFrame,
        _htf_frame: Option<&KlineFrame>,
        hard_evidence: &Value,
    ) -> Option<(Value, crate::ai::client::LLMReply, Vec<crate::ai::client::ChatMessage>)> {
        let client = self.typesafe_client.as_ref()?;

        let last_bar = frame.bars.first()?;
        let ema20 = frame.indicators.ema20.first().copied().unwrap_or(last_bar.close);
        let sma170 = frame.indicators.sma170.first().copied().unwrap_or(last_bar.close);
        let atr = frame.indicators.atr14.first().copied().unwrap_or(last_bar.close * 0.01);

        let recent_bars: Vec<serde_json::Value> = frame
            .bars
            .iter()
            .take(5)
            .rev()
            .map(|b| {
                serde_json::json!({
                    "open": b.open,
                    "high": b.high,
                    "low": b.low,
                    "close": b.close,
                    "volume": b.volume,
                    "bullish": b.close >= b.open
                })
            })
            .collect();

        let state = serde_json::json!({
            "symbol": frame.symbol,
            "timeframe": frame.timeframe,
            "strategy": system,
            "latest_bar": {
                "open": last_bar.open,
                "high": last_bar.high,
                "low": last_bar.low,
                "close": last_bar.close,
                "volume": last_bar.volume
            },
            "indicators": {
                "ema20": ema20,
                "sma170": sma170,
                "atr": atr,
                "dist_to_ema20_atr": (last_bar.close - ema20) / (atr.max(1e-6))
            },
            "recent_bars": recent_bars,
            "program_candidates": hard_evidence
        });

        let mut questions = std::collections::HashMap::new();

        let mut regime_opts = std::collections::HashMap::new();
        regime_opts.insert(
            "bull_trend".to_string(),
            "Price above EMA20 with rising moving averages and higher highs".to_string(),
        );
        regime_opts.insert(
            "bear_trend".to_string(),
            "Price below EMA20 with declining moving averages and lower lows".to_string(),
        );
        regime_opts.insert(
            "range_chop".to_string(),
            "Price oscillating around flat moving averages without directional follow-through".to_string(),
        );
        questions.insert(
            "market_regime".to_string(),
            crate::ai::typesafe::TypeSafeQuestion::choice(
                "Classify the prevailing market trend structure.",
                regime_opts,
            ),
        );

        questions.insert(
            "setup_valid".to_string(),
            crate::ai::typesafe::TypeSafeQuestion::noul_with_criteria(
                format!(
                    "Does the recent price action confirm an actionable {} trade setup without immediate false breakout risk?",
                    system
                ),
                "Setup is confirmed with high conviction directional momentum",
                "Setup is unconfirmed, overlapping chop, or showing rejection against the trade",
            ),
        );

        questions.insert(
            "bar_quality".to_string(),
            crate::ai::typesafe::TypeSafeQuestion::score(
                "Score the signal bar body saturation and momentum quality.",
                vec![
                    "0: Weak doji or opposing tail".to_string(),
                    "1: Moderate body with directional close".to_string(),
                    "2: Strong trend bar closing decisively near extreme".to_string(),
                ],
            ),
        );

        match client.evaluate(&state, &questions).await {
            Ok(resp) => {
                let regime_ans = resp.answers.get("market_regime");
                let setup_ans = resp.answers.get("setup_valid");
                let bar_ans = resp.answers.get("bar_quality");

                let choice_str = regime_ans
                    .and_then(|a| a.choice.clone())
                    .unwrap_or_else(|| "range_chop".to_string());
                let confidence = regime_ans.map(|a| a.effective_confidence()).unwrap_or(0.5);
                let setup_noul = setup_ans.and_then(|a| a.noul).unwrap_or(0.0);
                let bar_score = bar_ans.and_then(|a| a.score).unwrap_or(1.0);

                let min_conf = self.settings.typesafe.min_confidence;
                let min_noul = self.settings.typesafe.min_noul_threshold;

                let is_proceed = confidence >= min_conf && setup_noul >= min_noul;

                let diagnosis = serde_json::json!({
                    "market_regime": choice_str,
                    "typesafe_evaluated": true,
                    "typesafe_confidence": confidence,
                    "typesafe_setup_noul": setup_noul,
                    "typesafe_bar_quality_score": bar_score,
                    "gate_result": if is_proceed { "proceed" } else { "wait" },
                    "reasoning": format!(
                        "TypeSafe Jev System 1: regime={}, confidence={:.2} (min={:.2}), setup_noul={:.2} (min={:.2}), bar_score={:.2} => gate={}",
                        choice_str, confidence, min_conf, setup_noul, min_noul, bar_score, if is_proceed { "proceed" } else { "wait" }
                    ),
                    "program_candidates": hard_evidence
                });

                let reply = crate::ai::client::LLMReply {
                    content: diagnosis.to_string(),
                    reasoning_content: Some(format!("TypeSafe System One Jev (latency: {}ms)", resp.latency_ms)),
                    usage: crate::ai::client::Usage {
                        prompt_tokens: resp.usage.input_tokens,
                        completion_tokens: resp.usage.output_tokens,
                        total_tokens: resp.usage.input_tokens + resp.usage.output_tokens,
                    },
                    latency_ms: resp.latency_ms,
                };

                let messages = vec![
                    crate::ai::client::ChatMessage {
                        role: "system".to_string(),
                        content: "TypeSafe AI System One Evaluation".to_string(),
                    },
                    crate::ai::client::ChatMessage {
                        role: "assistant".to_string(),
                        content: reply.content.clone(),
                    },
                ];

                info!(
                    "TypeSafe Stage 1 evaluation complete: regime={}, conf={:.2}, noul={:.2} in {}ms",
                    choice_str, confidence, setup_noul, resp.latency_ms
                );

                Some((diagnosis, reply, messages))
            }
            Err(e) => {
                tracing::warn!("TypeSafe evaluation error: {}", e);
                None
            }
        }
    }
}


