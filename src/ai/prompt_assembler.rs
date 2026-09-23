use crate::ai::decision_stance::build_decision_stance_guidance;
use crate::data::base::{KlineFrame, PositionContext};
use crate::data::geometry::compute_kline_geometry_features;
use crate::records::experience::ExperienceReader;
use crate::records::schema::ExperienceEntry;
use serde_json::Value;
use std::path::{Path, PathBuf};
use crate::ai::client::ChatMessage;

pub const LANGUAGE_ZH_RULE: &str = "\
## 语言要求（阶段一、阶段二均必须遵守）

- **通俗易懂（最重要）**：面向用户的解释文本必须通俗易懂，像给刚入门的新手讲解一样，用日常语言描述市场发生了什么、为什么这样判断。
- **思考过程**：扩展思考、内部推理及 JSON 说明，全程使用简体中文。
- **最终输出**：JSON 中所有面向用户的字符串一律使用简体中文。
- **仅允许英文**：JSON 字段名（schema 键名）、规定的枚举取值（如 `proceed`、`wait`、`bullish`、`bearish`）、K 线序号（如 `K1`、`K42-K1`）。";

pub const PA_TERMINOLOGY_ZH: &str = "\
## 术语使用规范（面向用户时必须使用中文）

| 术语 | 含义 / 用法提示 |
|------|----------------|
| 信号棒 | 触发入场计划的 K 线；极点外 1 跳动设止损/突破单 |
| 入场棒 | 实际触发入场的 K 线；须在信号棒之后 |
| 确认棒 / 跟随 | 信号或入场后 1–2 根同向延续；无跟随则信号易失败 |
| 突破 | 价格越过结构位、通道线、区间边界或信号棒极点 |
| 假突破 | 突破后快速回到原结构内；区间中常见 |
| 突破回踩 / 回测 | 突破后回撤测试被突破位再延续（勿与「历史回测」混淆） |
| 外包棒 | 高低点完全包含前一根；方向未定时勿追两端 |
| 内包棒 | 完全在前一根范围内；ii/iii 为连续内包 |
| 流星线 | 长上影、小实体，常作顶部拒绝 |
| 锤子线 | 长下影、小实体，常作底部拒绝 |
| 十字星 | 开收接近、多空犹豫 |
| 趋势棒 | 实体大、收盘近极点、影线短 |
| 铁丝网 | 极窄重叠区间，默认少交易 |
| 被套 | 突破方向上的交易者被迫止损离场 |
| 磁力位 | 失败信号棒/入场棒极点吸引价格回测 |

英文缩写须加中文解释（不可单独使用英文）：SB/EB→信号棒/入场棒、OB/IB→外包棒/内包棒、H1/H2→第一次/第二次顺势回调、L1/L2→第一次/第二次逆势回调、MTR→主要趋势反转、AIL/AIS→持续看多/持续看空、20GB→约20根K线未触及均线、TR→交易区间、MM→测量移动（等距目标位）、SPS→尖峰顺势突破、SCS→尖峰连续尖峰。";

pub const OPENCLAW_AGENT_NO_TOOLS_RULE: &str = "\
## PA Agent × QClaw 任务模式（硬约束）

你正在接收 **PA Agent 程序化 K 线分析**请求，不是通用编程/运维助手会话。

**禁止调用任何工具**，包括但不限于：`exec`、运行 Python/shell、读/写/编辑文件、浏览器、联网搜索、在本机目录写中间 `.md`/`.json` 等。

- K 线表、EMA/ATR、几何特征、阶段一诊断（若有）**已全部在用户消息中给出**；禁止再拉数据或读盘。
- 风险点数、盈亏比、交易者方程、胜率估算等**一律在思考过程或 JSON 字段内心算**；禁止为 `risk=stop-entry` 之类简单算术启动解释器。
- **唯一交付物**：assistant 正文 `content` 中的裸 JSON（阶段一或阶段二 schema）。不得在磁盘上留档后再回复。

违反会导致分析极慢、工具刷屏，且程序无法解析你的输出。";

pub const THINKING_CONTENT_OUTPUT_RULE: &str = "\
## 思考与正式输出分离（硬约束，违反则程序判定失败）

启用扩展思考时，**思考区仅用于推演草稿**；**程序只读取 assistant 消息的 `content`（正文）** 做 JSON 校验，**不会**把 `reasoning_content` / 思考流当作阶段结果。

**你必须做到：**
1. 思考可以较长，但思考结束后**必须在 `content` 正文里输出完整、可 `json.loads` 的裸 JSON 对象**（阶段一诊断 JSON 或阶段二决策 JSON）。
2. **禁止**把完整 JSON **只**写在思考里而让 `content` 为空、空白或纯叙述文字。
3. **禁止**在 `content` 里输出 markdown 说明、英文长文分析、或「详见上文思考」——`content` 里**只能**是裸 JSON。
4. 若思考预算较大，请**预留足够 token** 给最终 JSON；宁可压缩思考篇幅，也**不得**省略正文 JSON。

阶段一：`content` = 阶段一诊断 JSON（含 `gate_trace`、`gate_result` 等必填字段）。
阶段二：`content` = 阶段二决策 JSON（含 `decision`、`decision_trace`、`terminal` 等必填字段）。";

pub const STAGE1_OUTPUT_REMINDER: &str = "\
请严格按照以下 JSON 格式输出诊断结果,不要输出任何其他内容。
**硬约束：思考结束后，必须在 assistant 正文 `content` 输出下方完整阶段一 JSON；不得仅在思考区分析而让 `content` 为空。**
**思考过程与 JSON 内所有说明性文字必须使用简体中文**（仅 JSON 键名与规定枚举除外）。
禁止用 markdown 代码围栏（不要写 ```json 或结尾的 ```），只输出裸 JSON 对象。

```json
{
  \"cycle_position\": \"spike|micro_channel|tight_channel|normal_channel|broad_channel|trending_tr|trading_range|extreme_tr|unknown\",
  \"alternative_cycle_position\": null,
  \"direction\": \"bullish|bearish|neutral\",
  \"dominant_force\": \"bulls|bears|neutral\",
  \"diagnosis_confidence\": 75,
  \"spike_stage\": null,
  \"climax_risk\": \"none|warning|triggered\",
  \"market_phase\": \"stable|transitioning\",
  \"transition_risk\": null,
  \"detected_patterns\": [],
  \"key_signals\": [],
  \"htf_context\": \"\",
  \"entry_setup\": \"\",
  \"support_levels\": [],
  \"resistance_levels\": [],
  \"strategy_files_needed\": [\"下跌通道分析识别.txt\", \"下跌通道交易策略.txt\"],
  \"risk_warning\": \"\",
  \"bar_analysis\": {
    \"always_in\": \"long|short|neutral\",
    \"last_closed_bar\": \"K1\",
    \"bar_type\": \"trend_bull|trend_bear|doji|inside|outside_bull|outside_bear|flat|other\",
    \"signal_bar\": {
      \"bar\": \"K2 或 null\",
      \"quality\": \"strong|medium|weak|invalid\",
      \"reason\": \"信号棒质量判断\"
    },
    \"entry_setup_type\": \"H1|H2|L1|L2|MTR|wedge|tr_boundary|breakout_pullback|none\",
    \"follow_through\": \"yes|no|pending|failed\"
  },
  \"bar_by_bar_summary\": [
    {
      \"bar\": \"K1\",
      \"role\": \"structure|signal|entry|confirmation|noise|trap|climax|test\",
      \"bar_type\": \"trend_bull|trend_bear|doji|inside|outside_bull|outside_bear|flat|other\",
      \"context_effect\": \"strengthens_bull|weakens_bull|strengthens_bear|weakens_bear|neutral\",
      \"follow_through\": \"yes|no|pending|failed\",
      \"trapped_side\": \"bulls|bears|both|none|unknown\",
      \"reason\": \"一句话说明该K线对当前市场状态的增量影响\"
    }
  ],
  \"gate_trace\": [
    {
      \"node_id\": \"1.2\",
      \"question\": \"是否能识别出当前市场周期？\",
      \"answer\": \"是\",
      \"reason\": \"K线结构特征清晰\",
      \"branch\": \"normal_channel\",
      \"section\": \"K线识别\",
      \"bar_range\": \"K12-K1\"
    },
    {
      \"node_id\": \"2.1\",
      \"question\": \"近期结构是否呈现明确惯性方向？\",
      \"answer\": \"是\",
      \"reason\": \"LH+LL 结构清晰\",
      \"branch\": \"bearish\",
      \"section\": \"方向判断\",
      \"bar_range\": \"K8-K1\"
    }
  ],
  \"gate_result\": \"proceed\"
}
```

## 阶段一闸门（二元决策树 §1–§2，必须执行）
当 gate_result=proceed 时，必须包含节点 1.2、1.3、2.1、2.2、2.5 共 5 条。
§2.5 answer=否/中性 ≠ gate_result=wait。gate_result=wait 仅在 §1.2 unknown 或 §1.3 extreme_tr 时触发。";

pub const STAGE1_TAIL_REMINDER: &str = "\
【最后一步·必做】思考结束后，立即在 assistant 正文 `content` 输出完整阶段一裸 JSON。
思考请用简体中文并尽量简洁；`content` 不得为空。
禁止调用 exec/Python/写文件等工具。
若 token 紧张：可缩短思考、将 bar_by_bar_summary 保持 5 根，但 gate_trace 与 gate_result 必须写在 JSON 末尾且不可省略。";

pub const STAGE2_API_TASK_RULE: &str = "\
## 阶段二 API 任务模式（硬约束，非聊天）

本次调用是 PA Agent **阶段二的一次独立 API 请求**。提示中虽含阶段一诊断 JSON，**不代表**阶段二已完成或可以收尾对话。

**禁止**输出：
- 「阶段一和阶段二都已输出完毕」「分析已完成」等会话总结
- 「告诉我你想怎么处理」「请选择 1/2/3/4」等菜单式追问
- Markdown 摘要、复盘建议、保存文件提示（除非写在 JSON 字段内）

**必须**：在 assistant 正文 `content` 输出**完整阶段二裸 JSON**（仅此一种交付物）。";

pub const STAGE2_OUTPUT_CONTRACT: &str = "\
请严格按照以下 JSON 格式输出决策结果，不要输出任何其他内容。
**硬约束：思考结束后，必须在 assistant 正文 `content` 输出下方完整阶段二 JSON；不得仅在思考区分析而让 `content` 为空。**
**思考过程与 JSON 内所有说明性文字必须使用简体中文**（仅 JSON 键名与规定枚举除外）。
禁止用 markdown 代码围栏（不要写 ```json 或结尾的 ```），只输出裸 JSON 对象。
重要规则：当 order_type 为“不下单”时，entry_price、take_profit_price、take_profit_price_2、stop_loss_price、order_direction 必须全部为 null。

```json
{
  \"decision\": {
    \"order_direction\": \"做多|做空|null\",
    \"order_type\": \"限价单|突破单|市价单|不下单\",
    \"entry_price\": null,
    \"entry_basis_bar\": null,
    \"entry_basis_extreme\": null,
    \"entry_rule\": null,
    \"take_profit_price\": null,
    \"take_profit_price_2\": null,
    \"stop_loss_price\": null,
    \"reasoning\": \"\",
    \"diagnosis_confidence\": 75,
    \"trade_confidence\": 70,
    \"estimated_win_rate\": null,
    \"key_factors\": [],
    \"watch_points\": [],
    \"risk_assessment\": \"\",
    \"invalidation_condition\": \"\"
  },
  \"diagnosis_summary\": {
    \"cycle_position\": \"\",
    \"direction\": \"\",
    \"key_signals\": []
  },
  \"bar_analysis\": {
    \"always_in\": \"long|short|neutral\",
    \"last_closed_bar\": \"K1\",
    \"bar_type\": \"trend_bull|trend_bear|doji|inside|outside_bull|outside_bear|flat|other\",
    \"signal_bar\": {
      \"bar\": \"K2 或 null\",
      \"quality\": \"strong|medium|weak|invalid\",
      \"pattern\": \"H1|H2|L1|L2|MTR|wedge|tr_boundary|breakout_pullback|none\",
      \"reason\": \"信号棒质量判断\"
    },
    \"entry_bar\": {
      \"strength\": \"strong|weak|not_triggered\",
      \"follow_through\": true,
      \"still_valid\": true,
      \"freshness\": \"fresh|pending|stale|invalid\"
    },
    \"second_entry\": {
      \"is_second_entry\": true,
      \"type\": \"H2|L2|MTR|wedge|tr_boundary|trendline|none\"
    }
  },
  \"decision_trace\": [
    {
      \"node_id\": \"4.1\",
      \"section\": \"通道\",
      \"question\": \"是否出现有序波段结构？\",
      \"answer\": \"是\",
      \"reason\": \"HH+HL\",
      \"skipped\": false,
      \"bar_range\": \"由你填写\"
    }
  ],
  \"terminal\": {
    \"node_id\": \"11.2\",
    \"outcome\": \"trade\",
    \"label\": \"...\"
  }
}
```";

pub const STAGE2_TAIL_REMINDER: &str = "\
【最后一步·必做】思考结束后，立即在 assistant 正文 `content` 输出完整阶段二裸 JSON
（含 decision、decision_trace、terminal）。思考用简体中文并尽量简洁；`content` 不得为空。
禁止调用 exec/Python/写文件等工具；算术在 JSON 推理字段内完成。
若 token 紧张，优先保证 `content` 有 JSON，可缩短思考。";

pub const STAGE2_BASE_PROMPT_TXT_FILES: &[&str] = &[
    "逐棒分析检查单.txt",
    "文件16-K线信号识别.txt",
    "文件17-止损和止盈与仓位管理.txt",
    "文件23-MeasuredMove与结构目标.txt",
];

pub const STAGE2_FULL_STRATEGY_PROMPT_TXT_FILES: &[&str] = &[
    "上涨通道分析识别.txt",
    "上涨通道交易策略.txt",
    "下跌通道分析识别.txt",
    "下跌通道交易策略.txt",
    "极速上涨分析识别.txt",
    "极速上涨交易策略.txt",
    "极速下跌分析识别.txt",
    "极速下跌交易策略.txt",
    "震荡区间分析识别.txt",
    "震荡区间交易策略.txt",
    "文件13-窄通道与宽通道策略.txt",
    "文件14-楔形形态分析交易.txt",
    "文件15-二次入场机会.txt",
    "文件18-突破失败与突破测试.txt",
    "文件19-H1H2-L1L2计数.txt",
    "文件20-AlwaysIn与20GB.txt",
    "文件21-铁丝网与无交易环境.txt",
    "文件22-信号失败后的磁力位.txt",
    "文件24-最终旗形与趋势末端.txt",
    "文件25-主要趋势反转MTR.txt",
    "文件27-三角形与收敛形态.txt",
    "文件28-双重顶底与微型结构.txt",
];

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

pub const HARNESS_CORE_LANGUAGE_RULES: &str = include_str!("../../prompt_engineering/harness/core/language_rules.md");
pub const HARNESS_CORE_RISK_PROTOCOL: &str = include_str!("../../prompt_engineering/harness/core/risk_protocol.md");
pub const HARNESS_CORE_EXECUTION_CONTRACT: &str = include_str!("../../prompt_engineering/harness/core/execution_contract.md");

pub const HARNESS_SKILL_2PA: &str = include_str!("../../prompt_engineering/harness/skills/price_action_2pa.md");
pub const HARNESS_SKILL_DOG_WALKING: &str = include_str!("../../prompt_engineering/harness/skills/dog_walking.md");

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

/// Load modular core rules from the harness directory or embedded fallback.
pub fn load_harness_core(module: &str, prompt_dir: Option<&Path>) -> String {
    let harness_dir = prompt_dir
        .map(|p| p.join("harness").join("core"))
        .unwrap_or_else(|| crate::config::paths::harness_dir().join("core"));
    let target = harness_dir.join(format!("{}.md", module));
    if target.exists() {
        if let Ok(content) = std::fs::read_to_string(&target) {
            if !content.trim().is_empty() {
                return content;
            }
        }
    }
    match module {
        "language_rules" => HARNESS_CORE_LANGUAGE_RULES.to_string(),
        "risk_protocol" => HARNESS_CORE_RISK_PROTOCOL.to_string(),
        "execution_contract" => HARNESS_CORE_EXECUTION_CONTRACT.to_string(),
        _ => String::new(),
    }
}

/// Load modular skill rules from the harness directory or embedded fallback.
pub fn load_harness_skill(system: &str, prompt_dir: Option<&Path>) -> String {
    let canon = crate::strategies::canonical(system).unwrap_or(system);
    let skill_name = match canon {
        "dog_reversion" | "dog_trend" | "dog_walking" => "dog_walking",
        _ => "price_action_2pa",
    };
    let harness_dir = prompt_dir
        .map(|p| p.join("harness").join("skills"))
        .unwrap_or_else(|| crate::config::paths::harness_dir().join("skills"));
    let target = harness_dir.join(format!("{}.md", skill_name));
    if target.exists() {
        if let Ok(content) = std::fs::read_to_string(&target) {
            if !content.trim().is_empty() {
                return content;
            }
        }
    }
    match skill_name {
        "dog_walking" => HARNESS_SKILL_DOG_WALKING.to_string(),
        _ => HARNESS_SKILL_2PA.to_string(),
    }
}

/// Resolve strategy prompt dynamically based on the active trading system.
pub fn resolve_strategy_prompt_for_system(system: &str, prompt_dir: Option<&Path>) -> StrategyPrompt {
    let canon = crate::strategies::canonical(system).unwrap_or(system);
    match canon {
        "dog_reversion" | "dog_trend" | "dog_walking" => {
            let skill = load_harness_skill("dog_walking", prompt_dir);
            let contract = load_harness_core("execution_contract", prompt_dir);
            let content = format!("{}\n\n{}", skill, contract);
            StrategyPrompt {
                hash: crate::learning::artifact::content_hash(&content),
                content,
                version: "dog_walking_harness".to_string(),
                source: "harness",
            }
        }
        _ => resolve_strategy_prompt(prompt_dir),
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

/// Mount indicator tables dynamically based on the active trading strategy.
///
/// Eliminates redundant dual-table injection (EMA20 vs SMA14/SMA170) and attaches
/// relevant Price Action features (e.g. geometric candlestick attributes) where needed.
pub fn render_indicators_for_system(system: &str, frame: &KlineFrame) -> String {
    let canon = crate::strategies::canonical(system).unwrap_or("2pa_trend");
    match canon {
        "dog_reversion" => {
            // Dog walking only requires the deviation and SMA14/SMA170 table
            render_dog_walking_kline_table(frame)
        }
        "adaptive" => {
            // Adaptive strategy monitors both price action and deviation regimes
            format!("{}\n\n{}", render_kline_table(frame), render_dog_walking_kline_table(frame))
        }
        _ => {
            // Price action strategies (2pa, 2pa_trend, etc.): mount EMA20 table and candle geometry table
            format!("{}\n\n#### 📐 近端 K 线微观几何特征明细 (实体比/影线比/收盘位置)\n{}",
                render_kline_table(frame),
                render_geometry_features_table(frame)
            )
        }
    }
}

pub fn build_stage2_prompt_with_strategy(
    req: &Stage2PromptRequest<'_>,
) -> (String, Vec<String>, Vec<ExperienceEntry>) {
    let canon = crate::strategies::canonical(req.system).unwrap_or(req.system);
    if canon == "2pa_source" {
        return build_source_2pa_stage2_prompt(
            req.frame,
            req.stage1_diagnosis,
            req.decision_stance,
            false,
            None,
            req.experience_dir,
            req.position_context,
            req.htf_context,
        );
    }
    let (cycle, direction, patterns) = experience_query(req.stage1_diagnosis);
    let experiences = load_experiences(
        req.experience_dir,
        &cycle,
        &direction,
        &patterns,
        req.experience_max_entries,
    );
    let experience_section = render_experience_section(&experiences);

    let indicators_section = render_indicators_for_system(req.system, req.frame);

    let prompt = format!(
        "{}\n{}\n策略：{}\n{}\n阶段一：{}\n持仓：{}\n{}\n{}\n{}\n阶段二：仅输出交易决策 JSON。",
        LANGUAGE_ZH_RULE,
        req.strategy_prompt,
        crate::strategies::canonical(req.system).unwrap_or("unknown"),
        build_decision_stance_guidance(req.decision_stance),
        req.stage1_diagnosis,
        serde_json::to_string(&req.position_context).unwrap_or_default(),
        indicators_section,
        req.htf_context.unwrap_or("高周期数据缺失"),
        experience_section
    );

    let mut files_used = vec!["strategy_v1.txt".to_string()];
    if !experiences.is_empty() {
        files_used.push(format!("experience:{}", cycle));
    }
    (prompt, files_used, experiences)
}

pub fn load_prompt_file(name: &str, prompt_dir: Option<&Path>) -> String {
    let candidate_paths = [
        prompt_dir.map(|p| p.join(name)),
        Some(PathBuf::from("prompt_engineering").join(name)),
        Some(crate::config::paths::harness_dir().parent().unwrap_or(Path::new("prompt_engineering")).join(name)),
    ];
    for p_opt in candidate_paths.into_iter().flatten() {
        if p_opt.exists() {
            if let Ok(c) = std::fs::read_to_string(&p_opt) {
                if !c.trim().is_empty() {
                    return c;
                }
            }
        }
    }
    String::new()
}

pub fn render_simple_market_features_block(frame: &KlineFrame) -> String {
    if frame.bars.is_empty() {
        return String::new();
    }
    let b0 = &frame.bars[0];
    let close = b0.close;
    let ema = frame.indicators.ema20.first().copied().unwrap_or(close);
    let atr = frame.indicators.atr14.first().copied().unwrap_or(close * 0.01).max(1e-6);
    let dist_atr = (close - ema) / atr;

    let n = frame.indicators.ema20.len();
    let slope_5 = if n >= 5 {
        (frame.indicators.ema20[0] - frame.indicators.ema20[4]) / (4.0 * atr)
    } else {
        0.0
    };

    let mut bull_count = 0;
    let mut bear_count = 0;
    let check_bars = frame.bars.len().min(8);
    for b in &frame.bars[..check_bars] {
        if b.close > b.open {
            bull_count += 1;
        } else if b.close < b.open {
            bear_count += 1;
        }
    }

    format!(
        "## 程序预计算市场特征辅助摘要（客观数值，用于验证你的主观判断）\n\
         - **最新收盘价**: {:.4} | **EMA20**: {:.4} | **ATR14**: {:.4}\n\
         - **价格偏离 EMA20**: {:+.2} ATR ({})\n\
         - **近 5 根 EMA20 斜率**: {:+.3} ATR/棒\n\
         - **近 {} 根 K 线实体统计**: 阳线 {} 根，阴线 {} 根\n",
        close, ema, atr,
        dist_atr, if dist_atr >= 0.0 { "处于均线上方" } else { "处于均线下方" },
        slope_5,
        check_bars, bull_count, bear_count
    )
}

pub fn format_breakout_tick_hint(_frame: &KlineFrame) -> String {
    let tick = 0.01;
    format!(
        "**突破单定价（程序推断最小跳动 ≈ {:.2}）**：做多时 entry_price 必须严格大于 entry_basis_bar 的 high（推荐 high + {:.2}）；做空时 entry_price 必须严格低于 low（推荐 low - {:.2}）。entry_rule 必须写明基准 K 线序号与极点价格。",
        tick, tick, tick
    )
}

pub fn route_stage2_strategy_files(stage1_json: &Value, load_full_strategy_library: bool) -> Vec<String> {
    if load_full_strategy_library {
        let mut all: Vec<String> = STAGE2_FULL_STRATEGY_PROMPT_TXT_FILES.iter().map(|s| s.to_string()).collect();
        all.extend(STAGE2_BASE_PROMPT_TXT_FILES.iter().map(|s| s.to_string()));
        let mut deduped = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for f in all {
            if seen.insert(f.clone()) {
                deduped.push(f);
            }
        }
        return deduped;
    }

    let cp = stage1_json.get("cycle_position").and_then(|v| v.as_str()).unwrap_or("unknown").trim().to_lowercase();
    let dir = stage1_json.get("direction")
        .or_else(|| stage1_json.get("dominant_force"))
        .and_then(|v| v.as_str())
        .unwrap_or("neutral")
        .trim()
        .to_lowercase();

    let patterns: Vec<String> = stage1_json.get("detected_patterns")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|x| x.as_str().map(|s| s.to_lowercase())).collect())
        .unwrap_or_default();
    let pat_set: std::collections::HashSet<_> = patterns.into_iter().collect();

    let is_channel = matches!(cp.as_str(), "micro_channel" | "tight_channel" | "normal_channel" | "broad_channel" | "channel");
    let is_range = matches!(cp.as_str(), "trading_range" | "trending_tr" | "range");
    let is_spike = matches!(cp.as_str(), "spike" | "trend");

    let mut files = Vec::new();

    // 1. Base cycle files
    if is_channel {
        if dir.contains("bull") || dir.contains("long") {
            files.push("上涨通道分析识别.txt".to_string());
            files.push("上涨通道交易策略.txt".to_string());
        } else if dir.contains("bear") || dir.contains("short") {
            files.push("下跌通道分析识别.txt".to_string());
            files.push("下跌通道交易策略.txt".to_string());
        } else {
            files.push("震荡区间分析识别.txt".to_string());
            files.push("震荡区间交易策略.txt".to_string());
        }
        files.push("文件13-窄通道与宽通道策略.txt".to_string());
    } else if is_spike {
        if dir.contains("bull") || dir.contains("long") {
            files.push("极速上涨分析识别.txt".to_string());
            files.push("极速上涨交易策略.txt".to_string());
        } else if dir.contains("bear") || dir.contains("short") {
            files.push("极速下跌分析识别.txt".to_string());
            files.push("极速下跌交易策略.txt".to_string());
        }
    } else if is_range {
        files.push("震荡区间分析识别.txt".to_string());
        files.push("震荡区间交易策略.txt".to_string());
    }

    // 2. Pattern overlays for 2PA
    if pat_set.contains("wedge") {
        files.push("文件14-楔形形态分析交易.txt".to_string());
    }
    if is_channel || pat_set.contains("reversal_attempt") || pat_set.contains("mtr") || pat_set.contains("final_flag") || pat_set.contains("h2") || pat_set.contains("l2") {
        files.push("文件15-二次入场机会.txt".to_string());
    }
    if pat_set.contains("mtr") {
        files.push("文件25-主要趋势反转MTR.txt".to_string());
    }
    if pat_set.contains("final_flag") {
        files.push("文件24-最终旗形与趋势末端.txt".to_string());
    }
    if is_channel || pat_set.contains("h1") || pat_set.contains("h2") || pat_set.contains("l1") || pat_set.contains("l2") {
        files.push("文件19-H1H2-L1L2计数.txt".to_string());
    }
    if pat_set.contains("breakout_failure") || pat_set.contains("failed_breakout") || pat_set.contains("breakout_test") || pat_set.contains("breakout_pullback") {
        files.push("文件18-突破失败与突破测试.txt".to_string());
    }
    if pat_set.contains("always_in") || pat_set.contains("ail") || pat_set.contains("ais") || pat_set.contains("20gb") || pat_set.contains("gap_bar") {
        files.push("文件20-AlwaysIn与20GB.txt".to_string());
    }
    if is_range || pat_set.contains("barbwire") || pat_set.contains("wire") || pat_set.contains("overlap") || pat_set.contains("middle_range") {
        files.push("文件21-铁丝网与无交易环境.txt".to_string());
    }
    if pat_set.contains("failed_signal") || pat_set.contains("magnet") || pat_set.contains("trapped_traders") {
        files.push("文件22-信号失败后的磁力位.txt".to_string());
    }
    if pat_set.iter().any(|p| p.contains("triangle")) {
        files.push("文件27-三角形与收敛形态.txt".to_string());
    }
    if pat_set.contains("double_top_bottom") || pat_set.contains("double_top") || pat_set.contains("double_bottom") {
        files.push("文件28-双重顶底与微型结构.txt".to_string());
    }

    // 3. Always append base files
    for base in STAGE2_BASE_PROMPT_TXT_FILES {
        files.push(base.to_string());
    }

    // 4. Stable dedup
    let mut deduped = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for f in files {
        if seen.insert(f.clone()) {
            deduped.push(f);
        }
    }
    deduped
}

pub fn build_source_2pa_system_prompt(prompt_dir: Option<&Path>) -> String {
    let human_mindset = load_prompt_file("提示词大纲_人设与思维方式.txt", prompt_dir);
    let binary_decision = load_prompt_file("二元决策.txt", prompt_dir);

    format!(
        "{}\n\n{}\n\n{}\n\n{}\n\n---\n\n{}\n\n---\n\n{}",
        LANGUAGE_ZH_RULE,
        PA_TERMINOLOGY_ZH,
        OPENCLAW_AGENT_NO_TOOLS_RULE,
        THINKING_CONTENT_OUTPUT_RULE,
        human_mindset,
        binary_decision
    )
}

pub fn build_source_2pa_stage1_user_prompt(
    frame: &KlineFrame,
    prompt_dir: Option<&Path>,
    htf_context: Option<&str>,
) -> String {
    let diag_framework = load_prompt_file("市场诊断框架.txt", prompt_dir);
    let bar_signals = load_prompt_file("文件16-K线信号识别.txt", prompt_dir);
    let kline_tbl = render_kline_table(frame);
    let geom_tbl = render_geometry_features_table(frame);
    let features_block = render_simple_market_features_block(frame);
    let n_bars = frame.bars.len();

    let bg_window = if n_bars > 40 {
        format!("**长程背景 K{}–K41**（较老部分）：\n- swing 高低点、磁力位参考 → 写入 htf_context；背景方向\n- **禁止**用背景方向否决近期 direction；冲突时近期为主、背景作风险参考\n\n", n_bars)
    } else {
        format!("**长程背景**（当前仅 {} 根，不足 41 根，与近期窗口重叠）：\n\n", n_bars)
    };

    let htf_block = if let Some(htf) = htf_context {
        format!("## 高时间框架背景上下文\n{}\n\n", htf)
    } else {
        String::new()
    };

    format!(
        "## 阶段一任务\n\n\
         你现在只执行阶段一：市场诊断与闸门判断。不要评估具体下单、止损、止盈或仓位。\n\n\
         {}\n\n---\n\n{}\n\n---\n\n\
         {}\n\n---\n\n\
         ## 当前分析目标\n\
         品种: {} 周期: {} K线数量: {}\n\
         （K线序号：1=最新已收盘，最大 K{}；每个决策节点的 bar_range 由你自行选择子区间，勿超出 K{}-K1）\n\n\
         ## ⚠️ 分析窗口分层规则（必须遵守）\n\
         {}\
         **近期结构 K{}–K1：**\n\
         - cycle_position、direction、通道/区间/波段主结构\n\n\
         **即时惯性 K{}–K1：**\n\
         - Always In、惯性强度、近端 spike_stage / 尖峰识别\n\n\
         **即时信号 K{}–K1：**\n\
         - 信号棒/入场棒/二次入场/突破失败（阶段二 §9 裁定窗口）\n\n\
         **逐棒摘要 K5–K1：**\n\
         - bar_by_bar_summary **必须**恰好 5 条（窗口>=5 根时），每条 1 句 reason\n\n\
         {}\
         ## K线数据(序号1=最新已收盘K线,序号越大越早;不含当前未收盘K线)\n\n\
         {}\n\n\
         ## K线几何特征(程序预计算，类型包含 inside/outside/doji/trend/flat/other)\n\n\
         {}\n\n\
         {}\n\n\
         请根据以上数据，严格输出阶段一 JSON 诊断结果。\n\n\
         {}",
        diag_framework,
        bar_signals,
        STAGE1_OUTPUT_REMINDER,
        frame.symbol, frame.timeframe, n_bars,
        n_bars, n_bars,
        bg_window,
        n_bars.min(40),
        n_bars.min(8),
        n_bars.min(10),
        htf_block,
        kline_tbl,
        geom_tbl,
        features_block,
        STAGE1_TAIL_REMINDER
    )
}

pub fn build_source_2pa_stage1_messages(
    frame: &KlineFrame,
    prompt_dir: Option<&Path>,
    htf_context: Option<&str>,
) -> Vec<ChatMessage> {
    vec![
        ChatMessage {
            role: "system".to_string(),
            content: build_source_2pa_system_prompt(prompt_dir),
        },
        ChatMessage {
            role: "user".to_string(),
            content: build_source_2pa_stage1_user_prompt(frame, prompt_dir, htf_context),
        },
    ]
}

pub fn build_source_2pa_stage1_prompt(
    frame: &KlineFrame,
    prompt_dir: Option<&Path>,
    htf_context: Option<&str>,
) -> String {
    format!(
        "{}\n\n====================\n\n{}",
        build_source_2pa_system_prompt(prompt_dir),
        build_source_2pa_stage1_user_prompt(frame, prompt_dir, htf_context)
    )
}

pub fn build_source_2pa_stage2_user_prompt(
    frame: &KlineFrame,
    stage1_json: &Value,
    decision_stance: &str,
    load_full_strategy_library: bool,
    prompt_dir: Option<&Path>,
    experience_dir: Option<&Path>,
    position_context: Option<&PositionContext>,
    htf_context: Option<&str>,
) -> (String, Vec<String>, Vec<ExperienceEntry>) {
    let routed_files = route_stage2_strategy_files(stage1_json, load_full_strategy_library);
    let mut strategy_contents = Vec::new();
    for f in &routed_files {
        let content = load_prompt_file(f, prompt_dir);
        if !content.is_empty() {
            strategy_contents.push(format!("### 【策略手册】{}\n{}", f, content));
        }
    }
    let strategy_context = strategy_contents.join("\n\n---\n\n");

    let (cycle, direction, patterns) = experience_query(stage1_json);
    let experiences = load_experiences(
        experience_dir,
        &cycle,
        &direction,
        &patterns,
        5,
    );
    let experience_section = render_experience_section(&experiences);

    let stance_guidance = build_decision_stance_guidance(decision_stance);
    let pos_section = render_position_context_section(position_context);
    let kline_tbl = render_kline_table(frame);
    let geom_tbl = render_geometry_features_table(frame);
    let features_block = render_simple_market_features_block(frame);
    let breakout_hint = format_breakout_tick_hint(frame);
    let stage1_str = serde_json::to_string_pretty(stage1_json).unwrap_or_default();
    let n_bars = frame.bars.len();

    let htf_block = if let Some(htf) = htf_context {
        format!("## 高时间框架背景上下文\n{}\n\n", htf)
    } else {
        String::new()
    };

    let user_content = format!(
        "{}\n\n\
         ## 阶段二任务\n\n\
         你现在独立执行阶段二：交易决策、风险收益和下单方式评估（基于阶段一诊断结果）。\n\
         以下 JSON 是程序校验通过后的阶段一诊断结果，请以此为权威依据；本消息下方附有完整 K 线表与几何特征。\n\n\
         {}\n\n---\n\n\
         {}\n\n---\n\n\
         {}\n\n---\n\n\
         {}\n\n---\n\n\
         ## 阶段一诊断结果\n\n```json\n{}\n```\n\n\
         {}\
         ## K线数据(共{}根，序号1=最新已收盘)\n\n{}\n\n\
         ## K线几何特征(程序预计算，仅作逐棒客观辅助)\n\n{}\n\n\
         {}\n\n\
         {}\n\n\
         {}\n\n\
         请根据以上诊断和K线数据,按《二元决策.txt》§3–§11、§14 输出 JSON 决策结果。\n\
         注意:如果判断不下单,entry_price、take_profit_price、take_profit_price_2、stop_loss_price、order_direction 必须全部为 null。\n\n\
         {}",
        STAGE2_API_TASK_RULE,
        stance_guidance,
        pos_section,
        strategy_context,
        STAGE2_OUTPUT_CONTRACT,
        stage1_str,
        htf_block,
        n_bars,
        kline_tbl,
        geom_tbl,
        features_block,
        breakout_hint,
        experience_section,
        STAGE2_TAIL_REMINDER
    );

    (user_content, routed_files, experiences)
}

pub fn build_source_2pa_stage2_messages(
    frame: &KlineFrame,
    stage1_json: &Value,
    decision_stance: &str,
    load_full_strategy_library: bool,
    prompt_dir: Option<&Path>,
    experience_dir: Option<&Path>,
    position_context: Option<&PositionContext>,
    htf_context: Option<&str>,
) -> (Vec<ChatMessage>, Vec<String>, Vec<ExperienceEntry>) {
    let (user_prompt, files, exps) = build_source_2pa_stage2_user_prompt(
        frame, stage1_json, decision_stance, load_full_strategy_library,
        prompt_dir, experience_dir, position_context, htf_context,
    );
    let messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: build_source_2pa_system_prompt(prompt_dir),
        },
        ChatMessage {
            role: "user".to_string(),
            content: user_prompt,
        },
    ];
    (messages, files, exps)
}

pub fn build_source_2pa_stage2_prompt(
    frame: &KlineFrame,
    stage1_json: &Value,
    decision_stance: &str,
    load_full_strategy_library: bool,
    prompt_dir: Option<&Path>,
    experience_dir: Option<&Path>,
    position_context: Option<&PositionContext>,
    htf_context: Option<&str>,
) -> (String, Vec<String>, Vec<ExperienceEntry>) {
    let (user_prompt, files, exps) = build_source_2pa_stage2_user_prompt(
        frame, stage1_json, decision_stance, load_full_strategy_library,
        prompt_dir, experience_dir, position_context, htf_context,
    );
    let full_prompt = format!(
        "{}\n\n====================\n\n{}",
        build_source_2pa_system_prompt(prompt_dir),
        user_prompt
    );
    (full_prompt, files, exps)
}

pub fn build_stage1_prompt_for_system(
    system: &str, frame: &KlineFrame, prompt_dir: Option<&Path>, htf_context: Option<&str>,
) -> String {
    let canon = crate::strategies::canonical(system).unwrap_or(system);
    if canon == "2pa_source" {
        return build_source_2pa_stage1_prompt(frame, prompt_dir, htf_context);
    }
    let strategy = resolve_strategy_prompt_for_system(system, prompt_dir);
    let indicators_section = render_indicators_for_system(system, frame);
    format!("{}\n{}\n策略：{}\n{}\n高时间框架：{}\n阶段一：仅输出市场诊断 JSON。",
        LANGUAGE_ZH_RULE, strategy.content,
        crate::strategies::canonical(system).unwrap_or("unknown"),
        indicators_section, htf_context.unwrap_or("高周期数据缺失"))
}

pub fn build_stage2_prompt_for_system(
    system: &str, frame: &KlineFrame, stage1_diagnosis: &Value, decision_stance: &str,
    load_all_strategies: bool, prompt_dir: Option<&Path>, experience_dir: Option<&Path>,
    position_context: Option<&PositionContext>, htf_context: Option<&str>,
) -> (String, Vec<String>, Vec<ExperienceEntry>) {
    let canon = crate::strategies::canonical(system).unwrap_or(system);
    if canon == "2pa_source" {
        return build_source_2pa_stage2_prompt(
            frame, stage1_diagnosis, decision_stance, load_all_strategies,
            prompt_dir, experience_dir, position_context, htf_context,
        );
    }
    let strategy = resolve_strategy_prompt_for_system(system, prompt_dir);
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

    #[test]
    fn test_render_indicators_for_system_decouples_indicators() {
        let f = frame();

        // 2PA system must mount EMA20 and geometry features, and MUST NOT mount SMA14/SMA170
        let pa_indicators = render_indicators_for_system("2pa_trend", &f);
        assert!(pa_indicators.contains("EMA20"));
        assert!(pa_indicators.contains("微观几何特征"));
        assert!(!pa_indicators.contains("SMA170 (主人)"), "2PA prompt must not include dog walking table");

        // Dog walking system must mount SMA14/SMA170, and MUST NOT mount EMA20 or geometry features
        let dog_indicators = render_indicators_for_system("dog_walking", &f);
        assert!(dog_indicators.contains("SMA14 (狗绳)"));
        assert!(dog_indicators.contains("SMA170 (主人)"));
        assert!(!dog_indicators.contains("微观几何特征"), "Dog walking prompt must not include PA geometry table");

        // Adaptive system mounts both
        let adaptive_indicators = render_indicators_for_system("adaptive", &f);
        assert!(adaptive_indicators.contains("EMA20"));
        assert!(adaptive_indicators.contains("SMA170 (主人)"));
    }

    #[test]
    fn test_source_2pa_prompt_pipeline() {
        let f = frame();
        let s1_msgs = build_source_2pa_stage1_messages(&f, None, Some("HTF Trend: Bullish"));
        assert_eq!(s1_msgs.len(), 2);
        assert_eq!(s1_msgs[0].role, "system");
        assert_eq!(s1_msgs[1].role, "user");
        assert!(s1_msgs[0].content.contains("二元决策"));
        assert!(s1_msgs[1].content.contains("阶段一任务"));

        let diag = serde_json::json!({
            "cycle_position": "normal_channel",
            "direction": "bullish",
            "detected_patterns": ["h2", "breakout_test"]
        });
        let (s2_msgs, files, _) = build_source_2pa_stage2_messages(&f, &diag, "balanced", false, None, None, None, None);
        assert_eq!(s2_msgs.len(), 2);
        assert!(files.contains(&"文件15-二次入场机会.txt".to_string()));
        assert!(files.contains(&"文件19-H1H2-L1L2计数.txt".to_string()));
        assert!(files.contains(&"文件18-突破失败与突破测试.txt".to_string()));
        assert!(files.contains(&"文件23-MeasuredMove与结构目标.txt".to_string()));

        let s1_str = build_stage1_prompt_for_system("2pa_source", &f, None, None);
        assert!(s1_str.contains("阶段一任务"));
        let (s2_str, s2_files, _) = build_stage2_prompt_for_system("2pa_source", &f, &diag, "balanced", false, None, None, None, None);
        assert!(s2_str.contains("阶段二任务"));
        assert!(s2_files.contains(&"文件15-二次入场机会.txt".to_string()));
    }
}
