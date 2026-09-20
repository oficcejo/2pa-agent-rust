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
        let stage1_prompt = build_stage1_prompt_for_system(system, frame, self.prompt_dir.as_deref(), htf_context);
        let (mut stage1_diagnosis, stage1_reply, stage1_messages) = call_and_validate_stage1(
            &self.ai_client,
            &stage1_prompt,
            self.settings.validation.retry_max,
        ).await.context("第一阶段市场分析失败")?;

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

}


