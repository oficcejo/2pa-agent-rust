use crate::ai::client::AIClient;
use crate::ai::prompt_assembler::{build_stage1_prompt_for_system, build_stage2_prompt_for_system};
use crate::config::settings::Settings;
use crate::data::base::{KlineFrame, PositionContext};
use crate::indicators::alpha_pilot::AlphaPilotEngine;
use crate::orchestrator::validation_retry::{call_and_validate_stage1, call_and_validate_stage2};
use crate::records::history::save_record;
use crate::records::schema::{AnalysisRecord, RecordMeta};
use crate::util::mask::mask_secret;
use crate::util::timefmt::{now_local_iso, now_local_ms};
use anyhow::{anyhow, Result};
use serde_json::Value;
use std::path::PathBuf;
use tracing::info;

#[derive(Debug, Clone)]
pub struct TwoStageOrchestrator {
    pub ai_client: AIClient,
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

        Self {
            ai_client,
            prompt_dir: Some(PathBuf::from("prompt_engineering")),
            experience_dir: Some(PathBuf::from("experience")),
            records_dir,
            settings,
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
        if system == "alpha_pilot" {
            info!("Executing Native AlphaPilot Quant Engine for {} ({}) without LLM calls...", frame.symbol, frame.timeframe);
            let record = self.run_alpha_pilot_analysis(frame, pos_ctx)?;
            let _ = save_record(&self.records_dir, &record);
            return Ok(record);
        }

        info!("Starting Stage 1 analysis for {} ({}) using system [{}]...", frame.symbol, frame.timeframe, system);

        let hard_evidence = crate::strategies::diagnostics(system, frame, htf_frame);
        let context = format!("{}\n程序候选证据（不得篡改）：{}", htf_context.unwrap_or(""), hard_evidence);
        let htf_context = Some(context.as_str());
        let stage1_prompt = build_stage1_prompt_for_system(system, frame, self.prompt_dir.as_deref(), htf_context);
        let (mut stage1_diagnosis, stage1_reply, stage1_messages) = call_and_validate_stage1(
            &self.ai_client,
            &stage1_prompt,
            self.settings.validation.retry_max,
        ).await?;

        stage1_diagnosis["program_candidates"] = hard_evidence;
        if stage1_diagnosis["program_candidates"]["long"]["eligible"] != true
            && stage1_diagnosis["program_candidates"]["short"]["eligible"] != true {
            stage1_diagnosis["gate_result"] = serde_json::json!("wait");
        }
        info!("Stage 1 diagnosis complete. Starting Stage 2 decision for system [{}]...", system);

        let (stage2_prompt, strategies_used, experiences_loaded) = build_stage2_prompt_for_system(
            system,
            frame,
            &stage1_diagnosis,
            &self.settings.general.decision_stance,
            self.settings.prompt.stage2_load_full_strategy_library,
            self.prompt_dir.as_deref(),
            self.experience_dir.as_deref(),
            pos_ctx,
            htf_context,
        );

        let (mut stage2_decision, stage2_reply, stage2_messages) = call_and_validate_stage2(
            &self.ai_client,
            &stage2_prompt,
            self.settings.validation.retry_max,
            Some(&stage1_diagnosis),
        ).await?;

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

        let _ = save_record(&self.records_dir, &record);
        Ok(record)
    }

    pub fn run_alpha_pilot_analysis(
        &self,
        frame: &KlineFrame,
        pos_ctx: Option<&PositionContext>,
    ) -> Result<AnalysisRecord> {
        let alpha_res = AlphaPilotEngine::evaluate(&frame.bars)
            .ok_or_else(|| anyhow!("AlphaPilot 因子评估失败：K 线数据不足"))?;

        let kline_json = serde_json::to_value(&frame.bars).unwrap_or(Value::Array(Vec::new()));
        let kline_data = match kline_json {
            Value::Array(arr) => arr,
            _ => Vec::new(),
        };

        let stage1_diagnosis = serde_json::json!({
            "trading_system": "alpha_pilot",
            "cycle_position": if alpha_res.supertrend_dir > 0.0 { "bullish_trend" } else { "bearish_trend" },
            "alpha_raw_factor": alpha_res.raw_factor,
            "alpha_z_score": alpha_res.z_score,
            "alpha_target_position": alpha_res.target_position,
            "rs_vol_norm": alpha_res.rs_vol_norm,
            "is_vol_jump": alpha_res.is_vol_jump,
            "supertrend_dir": alpha_res.supertrend_dir,
            "supertrend_upper": alpha_res.supertrend_upper,
            "supertrend_lower": alpha_res.supertrend_lower,
            "diagnosis_summary": alpha_res.rationale,
            "gate_result": if alpha_res.target_position.abs() > 0.05 { "proceed" } else { "wait" }
        });

        let (mut action, mut order_type) = match alpha_res.order_action.as_str() {
            "OPEN" => ("OPEN", "市价单"),
            "CLOSE_EARLY" => ("CLOSE_EARLY", "平仓"),
            _ => ("WAIT", "不下单"),
        };

        if let Some(pos) = pos_ctx.filter(|p| p.has_position) {
            let same_side = (pos.pos_side == "long" && alpha_res.target_position > 0.05)
                || (pos.pos_side == "short" && alpha_res.target_position < -0.05);
            (action, order_type) = if same_side { ("HOLD", "持有") } else { ("CLOSE_EARLY", "平仓") };
        }

        let stage2_decision = serde_json::json!({
            "trading_system": "alpha_pilot",
            "decision": {
                "action": action,
                "order_type": order_type,
                "order_direction": alpha_res.order_direction,
                "entry_price": alpha_res.entry_price,
                "stop_loss_price": alpha_res.stop_loss_price,
                "take_profit_price": alpha_res.take_profit_price,
                "trade_confidence": (alpha_res.confidence * 100.0).round() as u32,
                "confidence": (alpha_res.confidence * 100.0).round() as u32,
                "target_position": alpha_res.target_position,
                "position_scale": alpha_res.target_position.abs(),
                "estimated_win_rate": null,
                "estimated_win_rate_reasoning": "信号强度不是经历史成交校准的胜率",
                "risk_reward_ratio": 2.0,
                "reasoning": alpha_res.rationale,
            },
            "decision_trace": ["alpha_pilot_symbolic_evaluation", "neutral_band_stateless_filter"]
        });

        let pos_val = pos_ctx.and_then(|p| serde_json::to_value(p).ok());

        Ok(AnalysisRecord {
            meta: RecordMeta {
                timestamp_local_iso: now_local_iso(),
                timestamp_local_ms: now_local_ms(),
                symbol: frame.symbol.clone(),
                timeframe: frame.timeframe.clone(),
                bar_count: frame.bars.len(),
                ai_provider: serde_json::json!({
                    "model": "AlphaPilot-Quant-V1",
                    "base_url": "native://rust",
                    "api_key": "None (Pure Quant)",
                }),
                decision_stance: self.settings.general.decision_stance.clone(),
                trading_system: "alpha_pilot".to_string(),
            },
            kline_data,
            htf_text: String::new(),
            stage1_messages: Vec::new(),
            stage1_response: Some(serde_json::json!({
                "content": alpha_res.rationale.clone(),
                "usage": { "prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0 }
            })),
            stage1_diagnosis: Some(stage1_diagnosis),
            stage2_messages: Vec::new(),
            stage2_response: Some(serde_json::json!({
                "content": serde_json::to_string_pretty(&stage2_decision).unwrap_or_default(),
                "usage": { "prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0 }
            })),
            stage2_decision: Some(stage2_decision),
            strategy_files_used: vec!["best_ETH-USDT-SWAP_15m.json".to_string()],
            experience_loaded: Vec::new(),
            position_context: pos_val,
            exception: None,
            usage_total: serde_json::json!({
                "prompt_tokens": 0,
                "completion_tokens": 0,
                "total_tokens": 0
            }),
        })
    }
}


