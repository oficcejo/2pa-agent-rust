use std::collections::HashSet;

pub const STAGE1_DETECTED_PATTERNS_GUIDE: &str = "\
## detected_patterns 判定表（阶段一必填标签）

在 `detected_patterns` 中填写**英文 key**（可多个）。程序据此在阶段二加载对应策略文件。
若 `bar_analysis.entry_setup_type` 已判定为 wedge / breakout_pullback 等，**必须**在 `detected_patterns` 中写入对应 key。

| key | 何时填写 | 阶段二加载 |
|-----|----------|------------|
| wedge | 三次同向推进、幅度递减、趋势线/通道收敛；含楔形回撤与楔形反转 | 文件14-楔形形态分析交易.txt |
| reversal_attempt | 反转尝试、MTR 前后结构、final flag、明显二次测试失败 | 文件15-二次入场机会.txt |
| mtr | 主要趋势反转结构已成型（常与 reversal_attempt 同现） | 文件15（叠加）、文件25 |
| final_flag | 趋势末段 final flag / 末端旗形 | 文件15（叠加）、文件24 |
| h1 / h2 / l1 / l2 | 计数入场结构（High1/High2/Low1/Low2） | 文件19-H1H2-L1L2计数.txt |
| breakout_test | 突破后回测突破位、突破测试棒 | 文件18 |
| breakout_pullback | 突破失败后的再次失败（突破回踩）顺势机会 | 文件18 |
| breakout_failure / failed_breakout | 普通突破失败、假突破 | 文件18、文件22 |
| always_in / ail / ais / 20gb / gap_bar | Always In、20GB、缺口棒等 | 文件20-AlwaysIn与20GB.txt |
| barbwire / wire / overlap / middle_range | 铁丝网、重叠、区间中部 | 文件21-铁丝网与无交易环境.txt |
| failed_signal / magnet / trapped_traders | 信号失败后磁力位、交易者被套 | 文件22-信号失败后的磁力位.txt |
| ascending_triangle / descending_triangle / symmetrical_triangle / expanding_triangle | 三角形收敛形态 | 文件27-三角形与收敛形态.txt |
| double_top_bottom | 双顶、双底、微型双顶底 | 文件28-双重顶底与微型结构.txt |

阶段二常驻（非 pattern 触发）：文件23（Measured Move）。";

pub const STAGE1_PATTERN_BRIEFS_BLOCK: &str = "\
## 特殊形态阶段一速查（判定要点；细则在阶段二 playbook）

**wedge**：三推同向、每推幅度递减、两线收敛；上升楔形偏看跌突破、下降楔形偏看涨突破。
**breakout_test / breakout_pullback**：突破后回测突破位；「失败的失败」= 突破回踩顺势。
**breakout_failure**：突破后无跟随、快速回到结构内。
**reversal_attempt / mtr**：逆主趋势反转尝试；等待二次入场优于第一次。
**h1/h2/l1/l2**：计数入场；h2/l2 二次入场胜率通常更高。
**barbwire / overlap / middle_range**：铁丝网、重叠、区间中部或边界；entry_setup_type=tr_boundary 时两者均应写入 detected_patterns。
**always_in / 20gb**：强趋势连续同向棒；逆势需双确认。";

pub fn route_strategy_files(
    cycle_position: &str,
    patterns: &[String],
    load_all: bool,
) -> Vec<String> {
    if load_all {
        return vec![
            "上涨通道交易策略.txt".to_string(),
            "下跌通道交易策略.txt".to_string(),
            "极速上涨交易策略.txt".to_string(),
            "极速下跌交易策略.txt".to_string(),
            "震荡区间交易策略.txt".to_string(),
            "文件13-窄通道与宽通道策略.txt".to_string(),
            "文件14-楔形形态分析交易.txt".to_string(),
            "文件15-二次入场机会.txt".to_string(),
            "文件16-K线信号识别.txt".to_string(),
            "文件17-止损和止盈与仓位管理.txt".to_string(),
            "文件18-突破失败与突破测试.txt".to_string(),
            "文件19-H1H2-L1L2计数.txt".to_string(),
            "文件20-AlwaysIn与20GB.txt".to_string(),
            "文件21-铁丝网与无交易环境.txt".to_string(),
            "文件22-信号失败后的磁力位.txt".to_string(),
            "文件23-MeasuredMove与结构目标.txt".to_string(),
            "文件24-最终旗形与趋势末端.txt".to_string(),
            "文件25-主要趋势反转MTR.txt".to_string(),
            "文件27-三角形与收敛形态.txt".to_string(),
            "文件28-双重顶底与微型结构.txt".to_string(),
        ];
    }

    let mut files = Vec::new();
    let pat_set: HashSet<String> = patterns.iter().map(|p| p.trim().to_lowercase()).collect();

    // 1. Cycle position base file
    match cycle_position {
        "spike" => {
            files.push("极速上涨交易策略.txt".to_string());
            files.push("极速下跌交易策略.txt".to_string());
        }
        "tight_channel" | "micro_channel" => {
            files.push("文件13-窄通道与宽通道策略.txt".to_string());
        }
        "broad_channel" | "normal_channel" => {
            files.push("上涨通道交易策略.txt".to_string());
            files.push("下跌通道交易策略.txt".to_string());
            files.push("文件13-窄通道与宽通道策略.txt".to_string());
        }
        "trading_range" | "trending_tr" | "extreme_tr" => {
            files.push("震荡区间交易策略.txt".to_string());
        }
        _ => {
            files.push("震荡区间交易策略.txt".to_string());
        }
    }

    // 2. Pattern-based files
    if pat_set.contains("wedge") {
        files.push("文件14-楔形形态分析交易.txt".to_string());
    }
    if pat_set.contains("reversal_attempt") || pat_set.contains("mtr") {
        files.push("文件15-二次入场机会.txt".to_string());
        if pat_set.contains("mtr") {
            files.push("文件25-主要趋势反转MTR.txt".to_string());
        }
    }
    if pat_set.contains("final_flag") {
        files.push("文件15-二次入场机会.txt".to_string());
        files.push("文件24-最终旗形与趋势末端.txt".to_string());
    }
    if pat_set.contains("h1") || pat_set.contains("h2") || pat_set.contains("l1") || pat_set.contains("l2") {
        files.push("文件19-H1H2-L1L2计数.txt".to_string());
    }
    if pat_set.contains("breakout_test") || pat_set.contains("breakout_pullback") || pat_set.contains("breakout_failure") || pat_set.contains("failed_breakout") {
        files.push("文件18-突破失败与突破测试.txt".to_string());
    }
    if pat_set.contains("always_in") || pat_set.contains("ail") || pat_set.contains("ais") || pat_set.contains("20gb") || pat_set.contains("gap_bar") {
        files.push("文件20-AlwaysIn与20GB.txt".to_string());
    }
    if pat_set.contains("barbwire") || pat_set.contains("wire") || pat_set.contains("overlap") || pat_set.contains("middle_range") {
        files.push("文件21-铁丝网与无交易环境.txt".to_string());
    }
    if pat_set.contains("failed_signal") || pat_set.contains("magnet") || pat_set.contains("trapped_traders") {
        files.push("文件22-信号失败后的磁力位.txt".to_string());
    }
    if pat_set.contains("ascending_triangle") || pat_set.contains("descending_triangle") || pat_set.contains("symmetrical_triangle") || pat_set.contains("expanding_triangle") {
        files.push("文件27-三角形与收敛形态.txt".to_string());
    }
    if pat_set.contains("double_top_bottom") {
        files.push("文件28-双重顶底与微型结构.txt".to_string());
    }

    // 3. Always include core files
    files.push("文件16-K线信号识别.txt".to_string());
    files.push("文件17-止损和止盈与仓位管理.txt".to_string());
    files.push("文件23-MeasuredMove与结构目标.txt".to_string());

    // Deduplicate while preserving order
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();
    for f in files {
        if seen.insert(f.clone()) {
            deduped.push(f);
        }
    }
    deduped
}
