use crate::ai::decision_stance::build_decision_stance_guidance;
use crate::data::base::KlineFrame;
use crate::data::geometry::compute_kline_geometry_features;
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

pub fn build_stage1_prompt_for_system(
    system: &str, frame: &KlineFrame, _prompt_dir: Option<&Path>, htf_context: Option<&str>,
) -> String {
    format!("{}\n{}\n策略：{}\n{}\n{}\n高时间框架：{}\n阶段一：仅输出市场诊断 JSON。",
        LANGUAGE_ZH_RULE, include_str!("../../prompt_engineering/strategy_v1.txt"),
        crate::strategies::canonical(system).unwrap_or("unknown"),
        render_kline_table(frame), render_dog_walking_kline_table(frame), htf_context.unwrap_or("高周期数据缺失"))
}

pub fn build_stage2_prompt_for_system(
    system: &str, frame: &KlineFrame, stage1_diagnosis: &Value, decision_stance: &str,
    _load_all_strategies: bool, _prompt_dir: Option<&Path>, _experience_dir: Option<&Path>,
    position_context: Option<&PositionContext>, htf_context: Option<&str>,
) -> (String, Vec<String>, Vec<ExperienceEntry>) {
    let prompt = format!("{}\n{}\n策略：{}\n{}\n阶段一：{}\n持仓：{}\n{}\n{}\n{}\n阶段二：仅输出交易决策 JSON。",
        LANGUAGE_ZH_RULE, include_str!("../../prompt_engineering/strategy_v1.txt"),
        crate::strategies::canonical(system).unwrap_or("unknown"),
        build_decision_stance_guidance(decision_stance), stage1_diagnosis,
        serde_json::to_string(&position_context).unwrap_or_default(),
        render_kline_table(frame), render_dog_walking_kline_table(frame), htf_context.unwrap_or("高周期数据缺失"));
    (prompt, vec!["strategy_v1.txt".into()], vec![])
}
