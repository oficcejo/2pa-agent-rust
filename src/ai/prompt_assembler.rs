use crate::ai::decision_stance::build_decision_stance_guidance;
use crate::data::base::KlineFrame;
use crate::data::geometry::compute_kline_geometry_features;
use crate::records::experience::ExperienceReader;
use crate::records::schema::ExperienceEntry;
use serde_json::Value;
use std::path::Path;


pub const LANGUAGE_ZH_RULE: &str = "\
## 语言要求（阶段一、阶段二均必须遵守）

- **通俗易懂（最重要）**：面向用户的解释文本必须通俗易懂，像给刚入门的新手讲解一样，用日常语言描述市场发生了什么、为什么这样判断。
- **思考过程**：扩展思考、内部推理及 JSON 说明，全程使用简体中文。
- **最终输出**：JSON 中所有面向用户的字符串一律使用简体中文。
- **仅允许英文**：JSON 字段名（schema 键名）、规定的枚举取值（如 `proceed`、`wait`、`bullish`、`bearish`）、K 线序号（如 `K1`、`K42-K1`）。";

pub const STAGE1_SYSTEM_PROMPT: &str = "\
你是一个专业的 Price Action (PA) 价格行为分析师。
你的任务是对提供的 K 线数据及技术指标进行【阶段一：市场诊断】。

你必须严格输出符合规范的纯 JSON 格式（不得输出额外的 Markdown 文本或前后解释）。

JSON 格式要求包含以下核心字段：
- `cycle_position`: 市场周期形态 (spike / tight_channel / broad_channel / trading_range / trending_tr 等)
- `dominant_force`: 当前多空主导力量 (bulls / bears / neutral)
- `trend_state`: 趋势状态描述
- `key_levels`: 关键支撑与阻力位列表
- `detected_patterns`: 识别出的 PA 形态列表 (英文 key，如 wedge, h2, l2, breakout_test, barbwire 等)
- `gate_result`: 阶段一闸门裁定 (proceed / wait / unknown)
- `gate_trace`: 闸门逐项检查追踪列表
- `diagnosis_summary`: 阶段一诊断通俗总结文本
- `reasoning`: 诊断思考与逻辑说明";

pub const STAGE2_SYSTEM_PROMPT: &str = include_str!("../../prompt_engineering/strategy_v1.txt");

pub const DOG_WALKING_STAGE1_SYSTEM_PROMPT: &str = STAGE1_SYSTEM_PROMPT;
pub const DOG_WALKING_STAGE2_SYSTEM_PROMPT: &str = include_str!("../../prompt_engineering/strategy_v1.txt");
pub const ADAPTIVE_STAGE1_SYSTEM_PROMPT: &str = STAGE1_SYSTEM_PROMPT;
pub const ADAPTIVE_STAGE2_SYSTEM_PROMPT: &str = DOG_WALKING_STAGE2_SYSTEM_PROMPT;

fn format_bar_time(ts: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ts)
        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| ts.to_string())
}

pub fn render_kline_table(frame: &KlineFrame) -> String {
    const RECENT_CUTOFF: usize = 30;
    let mut s = String::new();
    let n = frame.bars.len();

    if n > RECENT_CUTOFF {
        let older = &frame.bars[RECENT_CUTOFF..];
        let max_high = older.iter().map(|b| b.high).fold(f64::NEG_INFINITY, f64::max);
        let min_low = older.iter().map(|b| b.low).fold(f64::INFINITY, f64::min);
        let start_time = older.last().map(|b| format_bar_time(b.ts_open)).unwrap_or_default();
        let end_time = older.first().map(|b| format_bar_time(b.ts_open)).unwrap_or_default();
        let oldest_idx = n - 1;
        let start_ema = if oldest_idx < frame.indicators.ema20.len() { format!("{:.2}", frame.indicators.ema20[oldest_idx]) } else { "-".to_string() };
        let end_ema = if RECENT_CUTOFF < frame.indicators.ema20.len() { format!("{:.2}", frame.indicators.ema20[RECENT_CUTOFF]) } else { "-".to_string() };

        s.push_str(&format!(
            "#### 🌐 远端宏观窗口摘要 (K{} ~ K{}, 共 {} 根 K 线)\n\
             - **时间跨度**: {} ~ {} (UTC)\n\
             - **区间极值**: 最高 {:.2} | 最低 {:.2} (总波幅: {:.2} 点)\n\
             - **结构演变**: 开盘 {:.2} -> 收盘 {:.2} (EMA20: {} -> {})\n\n\
             #### 🔍 近端高保真即时窗口明细 (K{} ~ K1)\n",
            n, RECENT_CUTOFF + 1, older.len(),
            start_time, end_time,
            max_high, min_low, max_high - min_low,
            older.last().map(|b| b.open).unwrap_or(0.0),
            older.first().map(|b| b.close).unwrap_or(0.0),
            start_ema, end_ema,
            RECENT_CUTOFF.min(n)
        ));
    }

    s.push_str("| K线序号 | 开盘时间 (UTC) | 开盘价 | 最高价 | 最低价 | 收盘价 | 成交量 | EMA20 | ATR14 |\n|---|---|---|---|---|---|---|---|---|\n");
    let display_bars = if n > RECENT_CUTOFF { &frame.bars[..RECENT_CUTOFF] } else { &frame.bars[..] };
    for (i, bar) in display_bars.iter().enumerate() {
        let ema = if i < frame.indicators.ema20.len() {
            format!("{:.4}", frame.indicators.ema20[i])
        } else {
            "-".to_string()
        };
        let atr = if i < frame.indicators.atr14.len() {
            format!("{:.4}", frame.indicators.atr14[i])
        } else {
            "-".to_string()
        };
        let time_str = format_bar_time(bar.ts_open);

        s.push_str(&format!(
            "| K{} | {} | {:.4} | {:.4} | {:.4} | {:.4} | {:.2} | {} | {} |\n",
            bar.seq, time_str, bar.open, bar.high, bar.low, bar.close, bar.volume, ema, atr
        ));
    }
    s
}

pub fn render_dog_walking_kline_table(frame: &KlineFrame) -> String {
    const RECENT_CUTOFF: usize = 30;
    let mut s = String::new();
    let n = frame.bars.len();

    if n > RECENT_CUTOFF {
        let older = &frame.bars[RECENT_CUTOFF..];
        let max_high = older.iter().map(|b| b.high).fold(f64::NEG_INFINITY, f64::max);
        let min_low = older.iter().map(|b| b.low).fold(f64::INFINITY, f64::min);
        let start_time = older.last().map(|b| format_bar_time(b.ts_open)).unwrap_or_default();
        let end_time = older.first().map(|b| format_bar_time(b.ts_open)).unwrap_or_default();
        let oldest_idx = n - 1;
        let start_170 = if oldest_idx < frame.indicators.sma170.len() { format!("{:.2}", frame.indicators.sma170[oldest_idx]) } else { "-".to_string() };
        let end_170 = if RECENT_CUTOFF < frame.indicators.sma170.len() { format!("{:.2}", frame.indicators.sma170[RECENT_CUTOFF]) } else { "-".to_string() };

        s.push_str(&format!(
            "#### 🐕 远端宏观偏离回溯摘要 (K{} ~ K{}, 共 {} 根 K 线)\n\
             - **时间跨度**: {} ~ {} (UTC)\n\
             - **价格极值**: 最高 {:.2} | 最低 {:.2} (总波幅: {:.2} 点)\n\
             - **主人均线演变**: 开盘 {:.2} -> 收盘 {:.2} (SMA170: {} -> {})\n\n\
             #### 🔍 近端高保真即时偏离明细 (K{} ~ K1)\n",
            n, RECENT_CUTOFF + 1, older.len(),
            start_time, end_time,
            max_high, min_low, max_high - min_low,
            older.last().map(|b| b.open).unwrap_or(0.0),
            older.first().map(|b| b.close).unwrap_or(0.0),
            start_170, end_170,
            RECENT_CUTOFF.min(n)
        ));
    }

    s.push_str("| K线序号 | 开盘时间 (UTC) | 开盘价 | 最高价 | 最低价 | 收盘价 | 成交量 | SMA14 (狗绳) | SMA170 (主人) | 偏离度(%) | ATR14 |\n|---|---|---|---|---|---|---|---|---|---|---|\n");
    let display_bars = if n > RECENT_CUTOFF { &frame.bars[..RECENT_CUTOFF] } else { &frame.bars[..] };
    for (i, bar) in display_bars.iter().enumerate() {
        let sma14_str = if i < frame.indicators.sma14.len() && !frame.indicators.sma14[i].is_nan() {
            format!("{:.4}", frame.indicators.sma14[i])
        } else {
            "-".to_string()
        };
        let sma170_str = if i < frame.indicators.sma170.len() && !frame.indicators.sma170[i].is_nan() {
            format!("{:.4}", frame.indicators.sma170[i])
        } else {
            "-".to_string()
        };
        let dev_str = if i < frame.indicators.dev170_pct.len() && !frame.indicators.dev170_pct[i].is_nan() {
            format!("{:+.2}%", frame.indicators.dev170_pct[i])
        } else {
            "-".to_string()
        };
        let atr = if i < frame.indicators.atr14.len() && !frame.indicators.atr14[i].is_nan() {
            format!("{:.4}", frame.indicators.atr14[i])
        } else {
            "-".to_string()
        };
        let time_str = format_bar_time(bar.ts_open);

        s.push_str(&format!(
            "| K{} | {} | {:.4} | {:.4} | {:.4} | {:.4} | {:.2} | {} | {} | {} | {} |\n",
            bar.seq, time_str, bar.open, bar.high, bar.low, bar.close, bar.volume, sma14_str, sma170_str, dev_str, atr
        ));
    }
    s
}

pub fn render_geometry_features_table(frame: &KlineFrame) -> String {
    let features = compute_kline_geometry_features(frame, Some(20));
    let mut s = String::from("| K线序号 | 类型 | 实体比 | 上影比 | 下影比 | 收盘位置 | 范围/ATR | EMA关系 | 重叠比 | 内部序列 | 双顶底 | 缺口棒 |\n|---|---|---|---|---|---|---|---|---|---|---|---|\n");
    for f in features {
        s.push_str(&format!(
            "| K{} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            f.seq,
            f.bar_type,
            f.body_ratio.map(|v| format!("{:.2}", v)).unwrap_or_else(|| "-".to_string()),
            f.upper_wick_ratio.map(|v| format!("{:.2}", v)).unwrap_or_else(|| "-".to_string()),
            f.lower_wick_ratio.map(|v| format!("{:.2}", v)).unwrap_or_else(|| "-".to_string()),
            f.close_position.map(|v| format!("{:.2}", v)).unwrap_or_else(|| "-".to_string()),
            f.range_atr_ratio.map(|v| format!("{:.2}", v)).unwrap_or_else(|| "-".to_string()),
            f.ema_relation,
            f.overlap_prev_ratio.map(|v| format!("{:.2}", v)).unwrap_or_else(|| "-".to_string()),
            f.inside_sequence,
            f.micro_double,
            f.gap_bar,
        ));
    }
    s
}

use crate::data::base::PositionContext;

pub fn render_position_context_section(pos: Option<&PositionContext>) -> String {
    match pos {
        Some(p) if p.has_position => {
            let side_zh = if p.pos_side.eq_ignore_ascii_case("long") { "做多 (Long)" } else { "做空 (Short)" };
            let pnl_str = match (p.unrealized_pnl, p.unrealized_pnl_ratio) {
                (Some(pnl), Some(ratio)) => format!("{:+.4} USDT ({:+.2}%)", pnl, ratio),
                _ => "未知".to_string(),
            };
            let open_px_str = p.open_avg_px.map(|v| format!("{:.2}", v)).unwrap_or_else(|| "未知".to_string());
            let mark_px_str = p.mark_px.map(|v| format!("{:.2}", v)).unwrap_or_else(|| "未知".to_string());
            let sl_str = p.current_sl.map(|v| format!("{:.2}", v)).unwrap_or_else(|| "未设置".to_string());
            let tp_str = p.current_tp.map(|v| format!("{:.2}", v)).unwrap_or_else(|| "未设置".to_string());

            format!(
                "## 🛡️ 【当前账户持仓与实时风控状态】\n\
                 - **当前持仓状态**：【持仓中】\n\
                 - **持仓品种与方向**：{} | 方向：{}\n\
                 - **持仓数量**：{} 张/币\n\
                 - **开仓均价**：{} ──> **当前标记价**：{}\n\
                 - **未实现浮动盈亏**：{}\n\
                 - **当前生效中的止损价 (SL)**：{}\n\
                 - **当前生效中的止盈价 (TP)**：{}\n\n\
                 ⚠️ **【持仓生命周期管理规则】**：\n\
                 1. 若行情正沿预期发展且已累积安全浮盈（>= 1.5 ATR），请评估是否输出 `action: \"MOVE_STOP_LOSS\"` 将止损向有利方向移动（提损保本/锁定利润，多单只能上移，空单只能下移，严禁反向扩大止损！）；\n\
                 2. 若行情顺畅且未达到移损/平仓条件，输出 `action: \"HOLD\"` 继续持有；\n\
                 3. 若原开仓逻辑被重大反向信号彻底破坏，请输出 `action: \"CLOSE_EARLY\"` 主动平仓规避风险；\n\
                 4. 已有持仓时，禁止下达同向或反向的新开仓订单。",
                p.symbol, side_zh, p.pos_size, open_px_str, mark_px_str, pnl_str, sl_str, tp_str
            )
        },
        _ => {
            "## 🛡️ 【当前账户持仓与实时风控状态】\n\
             - **当前持仓状态**：【空仓 (No Open Position)】\n\
             - **操作指引**：当前无任何持仓，可正常评估市场并决策是否输出 `action: \"OPEN\"`（开仓）或 `action: \"WAIT\"`（观望）。".to_string()
        }
    }
}

pub fn build_stage1_prompt(frame: &KlineFrame, prompt_dir: Option<&Path>) -> String {
    build_stage1_prompt_for_system("2pa", frame, prompt_dir, None)
}

pub fn build_stage2_prompt(
    frame: &KlineFrame,
    stage1_diagnosis: &Value,
    decision_stance: &str,
    load_all_strategies: bool,
    prompt_dir: Option<&Path>,
    experience_dir: Option<&Path>,
    position_context: Option<&PositionContext>,
) -> (String, Vec<String>, Vec<ExperienceEntry>) {
    build_stage2_prompt_for_system(
        "2pa",
        frame,
        stage1_diagnosis,
        decision_stance,
        load_all_strategies,
        prompt_dir,
        experience_dir,
        position_context,
        None,
    )
}

/// The strategy protocol compiled into the binary.
///
/// This is now only a *fallback*: the authoritative copy lives in the versioned
/// artifact store so a threshold change no longer requires a rebuild.
pub const STRATEGY_PROMPT_FALLBACK: &str = include_str!("../../prompt_engineering/strategy_v1.txt");

/// A resolved strategy prompt together with its provenance.
#[derive(Debug, Clone)]
pub struct StrategyPrompt {
    pub content: String,
    pub version: String,
    pub hash: String,
    /// `artifact` when read from the versioned store, `embedded` otherwise.
    pub source: &'static str,
}

impl StrategyPrompt {
    pub fn embedded() -> Self {
        Self {
            content: STRATEGY_PROMPT_FALLBACK.to_string(),
            version: "embedded".to_string(),
            hash: crate::learning::artifact::content_hash(STRATEGY_PROMPT_FALLBACK),
            source: "embedded",
        }
    }
}

/// Load the active strategy prompt from the artifact store.
///
/// Falls back to the compiled-in copy when no directory is configured or the
/// store cannot be read, so a damaged artifact folder degrades to the shipped
/// baseline instead of an empty prompt.
pub fn resolve_strategy_prompt(prompt_dir: Option<&Path>) -> StrategyPrompt {
    let Some(dir) = prompt_dir else { return StrategyPrompt::embedded() };
    let store = crate::learning::PromptArtifactStore::new(dir.join("artifacts"));
    match store.ensure_seeded(crate::learning::STRATEGY_PROMPT_NAME, STRATEGY_PROMPT_FALLBACK) {
        Ok(active) => StrategyPrompt {
            content: active.content,
            version: active.version,
            hash: active.hash,
            source: "artifact",
        },
        Err(e) => {
            tracing::warn!("读取 prompt artifact 失败，回退到内置版本: {e:#}");
            StrategyPrompt::embedded()
        }
    }
}

/// Derive the experience-library query from a stage-1 diagnosis.
///
/// `dominant_force` is `bulls`/`bears`; case files key on `long`/`short`.
pub fn experience_query(stage1_diagnosis: &Value) -> (String, String, Vec<String>) {
    let cycle = stage1_diagnosis
        .get("cycle_position")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let direction = match stage1_diagnosis
        .get("dominant_force")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_lowercase()
        .as_str()
    {
        "bulls" | "bull" | "long" => "long".to_string(),
        "bears" | "bear" | "short" => "short".to_string(),
        other => other.to_string(),
    };
    let patterns = stage1_diagnosis
        .get("detected_patterns")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|p| p.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();
    (cycle, direction, patterns)
}

fn load_experiences(
    experience_dir: Option<&Path>,
    cycle_position: &str,
    direction: &str,
    patterns: &[String],
    max_entries: usize,
) -> Vec<ExperienceEntry> {
    if max_entries == 0 || cycle_position.trim().is_empty() {
        return Vec::new();
    }
    let Some(dir) = experience_dir else { return Vec::new() };
    let reader = ExperienceReader::new(dir);
    reader.read_for_stage2(cycle_position, direction, patterns, max_entries)
}

/// Render retrieved cases into the prompt.
///
/// Framed as reference material, not as evidence of edge: the project already
/// refuses to present a model's confidence as a measured win rate, and the same
/// rule applies to these samples.
fn render_experience_section(entries: &[ExperienceEntry]) -> String {
    if entries.is_empty() {
        return String::new();
    }
    let mut s = String::from(
        "\n## 历史同类案例（由程序从已结算交易自动抽取，仅作结构参考；样本有限，不得当作实测胜率或收益承诺，也不得据此放宽任何阈值）\n",
    );
    for entry in entries {
        let kind = if entry.case_type == "success" { "成功" } else { "失败" };
        let lesson = entry
            .content
            .get("lesson")
            .and_then(|v| v.as_str())
            .unwrap_or("（无复盘说明）");
        let outcome = entry.content.get("outcome").cloned().unwrap_or(Value::Null);
        s.push_str(&format!(
            "- 【{}·{}】{} 程序记录结果：{}\n",
            kind, entry.cycle_position, lesson, outcome
        ));
    }
    s
}

/// Inputs for building a stage-2 prompt against an explicit prompt body.
pub struct Stage2PromptRequest<'a> {
    pub system: &'a str,
    pub frame: &'a KlineFrame,
    pub stage1_diagnosis: &'a Value,
    pub decision_stance: &'a str,
    /// Strategy protocol text, normally from `resolve_strategy_prompt`.
    pub strategy_prompt: &'a str,
    pub experience_dir: Option<&'a Path>,
    pub experience_max_entries: usize,
    pub position_context: Option<&'a PositionContext>,
    pub htf_context: Option<&'a str>,
}

pub fn build_stage2_prompt_with_strategy(
    req: &Stage2PromptRequest<'_>,
) -> (String, Vec<String>, Vec<ExperienceEntry>) {
    let (cycle, direction, patterns) = experience_query(req.stage1_diagnosis);
    let experiences = load_experiences(
        req.experience_dir,
        &cycle,
        &direction,
        &patterns,
        req.experience_max_entries,
    );
    let experience_section = render_experience_section(&experiences);

    let prompt = format!(
        "{}\n{}\n策略：{}\n{}\n阶段一：{}\n持仓：{}\n{}\n{}\n{}\n{}\n阶段二：仅输出交易决策 JSON。",
        LANGUAGE_ZH_RULE,
        req.strategy_prompt,
        crate::strategies::canonical(req.system).unwrap_or("unknown"),
        build_decision_stance_guidance(req.decision_stance),
        req.stage1_diagnosis,
        serde_json::to_string(&req.position_context).unwrap_or_default(),
        render_kline_table(req.frame),
        render_dog_walking_kline_table(req.frame),
        req.htf_context.unwrap_or("高周期数据缺失"),
        experience_section
    );

    let mut files_used = vec!["strategy_v1.txt".to_string()];
    if !experiences.is_empty() {
        files_used.push(format!("experience:{}", cycle));
    }
    (prompt, files_used, experiences)
}

pub fn build_stage1_prompt_for_system(
    system: &str, frame: &KlineFrame, prompt_dir: Option<&Path>, htf_context: Option<&str>,
) -> String {
    let strategy = resolve_strategy_prompt(prompt_dir);
    format!("{}\n{}\n策略：{}\n{}\n{}\n高时间框架：{}\n阶段一：仅输出市场诊断 JSON。",
        LANGUAGE_ZH_RULE, strategy.content,
        crate::strategies::canonical(system).unwrap_or("unknown"),
        render_kline_table(frame), render_dog_walking_kline_table(frame), htf_context.unwrap_or("高周期数据缺失"))
}

pub fn build_stage2_prompt_for_system(
    system: &str, frame: &KlineFrame, stage1_diagnosis: &Value, decision_stance: &str,
    _load_all_strategies: bool, prompt_dir: Option<&Path>, experience_dir: Option<&Path>,
    position_context: Option<&PositionContext>, htf_context: Option<&str>,
) -> (String, Vec<String>, Vec<ExperienceEntry>) {
    let strategy = resolve_strategy_prompt(prompt_dir);
    build_stage2_prompt_with_strategy(&Stage2PromptRequest {
        system,
        frame,
        stage1_diagnosis,
        decision_stance,
        strategy_prompt: &strategy.content,
        experience_dir,
        experience_max_entries: 5,
        position_context,
        htf_context,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::base::{KlineBar, KlineFrame, IndicatorBundle};

    fn frame() -> KlineFrame {
        let bars = (0..40)
            .map(|i| KlineBar {
                seq: i + 1,
                ts_open: 1_700_000_000_000 + (i as i64) * 900_000,
                open: 100.0,
                high: 101.0,
                low: 99.0,
                close: 100.5,
                volume: 1.0,
                amount: 0.0,
                pct_chg: None,
                closed: true,
            })
            .collect();
        KlineFrame {
            symbol: "TEST-USDT-SWAP".into(),
            timeframe: "15m".into(),
            bars,
            indicators: IndicatorBundle {
                ema20: vec![100.0; 40],
                atr14: vec![1.0; 40],
                sma14: vec![100.0; 40],
                sma170: vec![100.0; 40],
                sma170_slope: vec![0.0; 40],
                dev170_pct: vec![0.0; 40],
            },
            snapshot_ts_local_ms: 1,
        }
    }

    fn diagnosis() -> Value {
        serde_json::json!({
            "cycle_position": "trending_tr",
            "dominant_force": "bulls",
            "detected_patterns": ["h2", "breakout_test"],
        })
    }

    #[test]
    fn embedded_prompt_is_used_without_a_directory() {
        let sp = resolve_strategy_prompt(None);
        assert_eq!(sp.source, "embedded");
        assert_eq!(sp.version, "embedded");
        assert!(!sp.content.is_empty());
    }

    #[test]
    fn experience_query_maps_dominant_force_to_side() {
        let (cycle, dir, patterns) = experience_query(&diagnosis());
        assert_eq!(cycle, "trending_tr");
        assert_eq!(dir, "long");
        assert_eq!(patterns, vec!["h2", "breakout_test"]);

        let bearish = serde_json::json!({"cycle_position": "spike", "dominant_force": "bears"});
        assert_eq!(experience_query(&bearish).1, "short");
    }

    #[test]
    fn no_experience_means_no_injection() {
        let f = frame();
        let (prompt, files, loaded) = build_stage2_prompt_for_system(
            "2pa_trend", &f, &diagnosis(), "balanced", false, None, None, None, None,
        );
        assert!(loaded.is_empty());
        assert!(!prompt.contains("历史同类案例"));
        assert_eq!(files, vec!["strategy_v1.txt"]);
    }

    #[test]
    fn experience_cases_are_injected_and_attributed() {
        let dir = std::env::temp_dir().join(format!("okx-pa-{}", uuid::Uuid::new_v4()));
        let cases = dir.join("trending_tr").join("success_cases");
        std::fs::create_dir_all(&cases).unwrap();
        let case = serde_json::json!({
            "signal_id": "s1",
            "direction": "long",
            "detected_patterns": ["h2"],
            "outcome": { "r_multiple": 2.0, "exit_reason": "take_profit" },
            "lesson": "同类 H2 二次入场按计划兑现。",
        });
        std::fs::write(
            cases.join("2026-09-01_10-00-00_s1.json"),
            serde_json::to_string_pretty(&case).unwrap(),
        ).unwrap();

        let f = frame();
        let (prompt, files, loaded) = build_stage2_prompt_with_strategy(&Stage2PromptRequest {
            system: "2pa_trend",
            frame: &f,
            stage1_diagnosis: &diagnosis(),
            decision_stance: "balanced",
            strategy_prompt: STRATEGY_PROMPT_FALLBACK,
            experience_dir: Some(&dir),
            experience_max_entries: 3,
            position_context: None,
            htf_context: None,
        });

        assert_eq!(loaded.len(), 1);
        assert!(prompt.contains("历史同类案例"));
        assert!(prompt.contains("同类 H2 二次入场按计划兑现"));
        assert!(prompt.contains("不得当作实测胜率"));
        assert!(files.iter().any(|f| f.starts_with("experience:")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn zero_experience_budget_disables_lookup() {
        let dir = std::env::temp_dir().join(format!("okx-pa0-{}", uuid::Uuid::new_v4()));
        let cases = dir.join("trending_tr").join("success_cases");
        std::fs::create_dir_all(&cases).unwrap();
        std::fs::write(
            cases.join("2026-09-01_10-00-00_s1.json"),
            r#"{"direction":"long","detected_patterns":["h2"],"outcome":{},"lesson":"x"}"#,
        ).unwrap();

        let f = frame();
        let (_, _, loaded) = build_stage2_prompt_with_strategy(&Stage2PromptRequest {
            system: "2pa_trend",
            frame: &f,
            stage1_diagnosis: &diagnosis(),
            decision_stance: "balanced",
            strategy_prompt: STRATEGY_PROMPT_FALLBACK,
            experience_dir: Some(&dir),
            experience_max_entries: 0,
            position_context: None,
            htf_context: None,
        });
        assert!(loaded.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
