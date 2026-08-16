use crate::ai::decision_stance::build_decision_stance_guidance;
use crate::ai::pattern_routing::{
    route_strategy_files, STAGE1_DETECTED_PATTERNS_GUIDE, STAGE1_PATTERN_BRIEFS_BLOCK,
};
use crate::ai::prompts::get_prompt_file;
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

pub const STAGE2_SYSTEM_PROMPT: &str = "\
你是一个专业的 Price Action (PA) 交易决策执行引擎。
你的任务是在【阶段一：市场诊断】的基础上，结合价格行为策略库与历史经验，给出明确的【阶段二：交易决策】。

你必须严格输出符合规范的纯 JSON 格式。

JSON 格式要求包含以下核心字段：
- `decision`: {
    \"order_type\": \"限价单\" | \"突破单\" | \"市价单\" | \"不下单\",
    \"order_direction\": \"做多\" | \"做空\" | \"neutral\" | null,
    \"entry_price\": 数字或 null,
    \"stop_loss_price\": 数字或 null,
    \"take_profit_price\": 数字或 null,
    \"trade_confidence\": 0-100 整数,
    \"estimated_win_rate\": 0-100 整数或 null,
    \"estimated_win_rate_reasoning\": 胜率估算说明,
    \"risk_reward_ratio\": 盈亏比数字或 null,
    \"traders_equation_passes\": true | false,
    \"reasoning\": 通俗易懂的交易决策理由
  }
- `decision_trace`: 二元决策树节点执行追踪
- `terminal`: { \"outcome\": \"trade\" | \"wait\" | \"reject\", \"label\": \"总结标签\", \"node_id\": \"最终节点\" }
- `watch_points`: 观察要点列表
- `invalidation_condition`: 方案失效条件描述";

pub fn render_kline_table(frame: &KlineFrame) -> String {
    let mut s = String::from("| K线序号 | 开盘时间 (UTC) | 开盘价 | 最高价 | 最低价 | 收盘价 | 成交量 | EMA20 | ATR14 |\n|---|---|---|---|---|---|---|---|---|\n");
    for (i, bar) in frame.bars.iter().enumerate() {
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
        let time_str = chrono::DateTime::from_timestamp_millis(bar.ts_open)
            .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| bar.ts_open.to_string());

        s.push_str(&format!(
            "| K{} | {} | {:.4} | {:.4} | {:.4} | {:.4} | {:.2} | {} | {} |\n",
            bar.seq, time_str, bar.open, bar.high, bar.low, bar.close, bar.volume, ema, atr
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

pub fn build_stage1_prompt(frame: &KlineFrame, prompt_dir: Option<&Path>) -> String {
    let diagnosis_framework = get_prompt_file("市场诊断框架.txt", prompt_dir);
    let binary_decision = get_prompt_file("二元决策.txt", prompt_dir);

    let kline_table = render_kline_table(frame);
    let geometry_table = render_geometry_features_table(frame);

    format!(
        "{}\n\n{}\n\n{}\n\n{}\n\n{}\n\n{}\n\n## 当前分析标的与 K 线数据\n- 标的: {}\n- 周期: {}\n- K 线根数: {}\n\n### K 线及基础指标表\n{}\n\n### 最近 K 线几何特征表\n{}\n\n请严格基于上述数据与规则，输出【阶段一：市场诊断】纯 JSON 格式。",
        STAGE1_SYSTEM_PROMPT,
        LANGUAGE_ZH_RULE,
        STAGE1_DETECTED_PATTERNS_GUIDE,
        STAGE1_PATTERN_BRIEFS_BLOCK,
        diagnosis_framework,
        binary_decision,
        frame.symbol,
        frame.timeframe,
        frame.bars.len(),
        kline_table,
        geometry_table
    )
}

pub fn build_stage2_prompt(
    frame: &KlineFrame,
    stage1_diagnosis: &Value,
    decision_stance: &str,
    load_all_strategies: bool,
    prompt_dir: Option<&Path>,
    experience_dir: Option<&Path>,
) -> (String, Vec<String>, Vec<ExperienceEntry>) {
    let cycle_pos = stage1_diagnosis.get("cycle_position").and_then(|v| v.as_str()).unwrap_or("unknown");
    let detected_patterns: Vec<String> = stage1_diagnosis.get("detected_patterns")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();

    let strategy_files = route_strategy_files(cycle_pos, &detected_patterns, load_all_strategies);

    let mut strategy_contents = String::new();
    for fname in &strategy_files {
        let content = get_prompt_file(fname, prompt_dir);
        if !content.is_empty() {
            strategy_contents.push_str(&format!("\n\n### 策略文档: {}\n{}", fname, content));
        }
    }

    let stance_guidance = build_decision_stance_guidance(decision_stance);

    // Read experiences
    let exp_reader = ExperienceReader::new(experience_dir.unwrap_or_else(|| Path::new("experience")));
    let dominant_force = stage1_diagnosis.get("dominant_force").and_then(|v| v.as_str()).unwrap_or("");
    let experiences = exp_reader.read_for_stage2(cycle_pos, dominant_force, &detected_patterns, 3);

    let mut experience_text = String::new();
    if !experiences.is_empty() {
        experience_text.push_str("\n\n## 历史类似交易经验参考\n");
        for exp in &experiences {
            experience_text.push_str(&format!(
                "- [{}] {}: {}\n",
                exp.case_type, exp.filename, serde_json::to_string(&exp.content).unwrap_or_default()
            ));
        }
    }

    let kline_table = render_kline_table(frame);

    let prompt = format!(
        "{}\n\n{}\n\n{}\n\n## 阶段一诊断结果\n```json\n{}\n```\n\n## 适用策略库规则{}\n{}\n\n## 最新 K 线数据\n{}\n\n请根据阶段一诊断、策略库及交易倾向，输出【阶段二：交易决策】纯 JSON 格式。",
        STAGE2_SYSTEM_PROMPT,
        LANGUAGE_ZH_RULE,
        stance_guidance,
        serde_json::to_string_pretty(stage1_diagnosis).unwrap_or_default(),
        strategy_contents,
        experience_text,
        kline_table
    );

    (prompt, strategy_files, experiences)
}
