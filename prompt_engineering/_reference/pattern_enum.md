# Brooks Pattern Enum Reference

本文件是提示词工程参考层，不参与每次 Stage 1 / Stage 2 prompt 加载。用途是统一 `detected_patterns`、`bar_analysis.entry_setup_type` 与后续程序特征命名。

## detected_patterns 建议枚举

- `wedge`：楔形或三推结构（含 Micro Wedge、Nested Wedge）。
- `reversal_attempt`：反转尝试，未必满足完整 MTR。
- `mtr`：主要趋势反转，需趋势线突破 + 前极点测试失败等核心条件（见文件25）。
- `final_flag`：最终旗形或趋势末端旗形失败（见文件24）。**勿与尖峰旗形（Spike Flag）混淆**：尖峰旗形顺向突破属 SPS 变体，允许顺主趋势评估；FF 禁止追顺势。
- `h1` / `h2` / `l1` / `l2`：High1/High2/Low1/Low2 计数入场结构。
- `breakout_failure`：突破失败，突破后快速回到原结构内。
- `failed_breakout`：`breakout_failure` 的同义标签；优先统一为 `breakout_failure`。
- `breakout_test`：突破后回测突破位或信号价位。
- `breakout_pullback`：突破回踩或突破测试（失败的失败）。
- `barbwire`：铁丝网或极紧凑交易区间。
- `wire`：`barbwire` 的同义标签；优先统一为 `barbwire`。
- `overlap`：K线高度重叠、方向不明。
- `middle_range`：区间中部或当日区间中部，交易价值低。
- `always_in`：Always In Long/Short 状态。
- `ail` / `ais`：Always In Long / Always In Short。
- `20gb`：连续约20根K线未触及 EMA。
- `gap_bar`：均线缺口棒（Moving Average Gap Bar），不是普通开盘跳空。
- `opening_gap`：开盘跳空缺口，区别于 `gap_bar`。
- `spike_candidate`：单根超大突破棒或尚未达到2根的尖峰候选；不得单独触发 `cycle_position=spike`。
- `spike_active` / `spike_ending` / `spike_transitioning`：尖峰阶段标签。
- `double_top_bottom`：双顶、双底、微型双顶或微型双底（见文件28）。
- `climax_warning` / `climax_triggered`：高潮风险（与 JSON 字段 `climax_risk=warning/triggered` 对应；**勿**单独使用裸 `climax`）
- `shrinking_stairs`：收缩台阶或推进幅度递减。
- `failed_signal`：信号失败，失败信号的入场价/止损位成为磁力位。
- `magnet`：磁力位（见文件22）。
- `trapped_traders`：被套交易者结构。
- `ascending_triangle` / `descending_triangle` / `symmetrical_triangle` / `expanding_triangle`：三角形（见文件27）。

## bar_type 建议枚举

- `trend_bull` / `trend_bear` / `doji` / `inside` / `outside_bull` / `outside_bear` / `flat` / `other`

## entry_setup_type 建议枚举

- `H1` / `H2` / `L1` / `L2` / `MTR` / `wedge` / `tr_boundary` / `breakout_pullback` / `EMA_pullback` / `triangle_breakout` / `none`

## Stage2 常驻 playbook（非 detected_patterns 触发）

- `文件23-MeasuredMove与结构目标.txt`

## 命名原则

- 程序特征使用英文枚举，提示词解释使用简体中文。
- `detected_patterns` 只放结构候选，不放具体交易方向。
- 是否可交易由 Stage 2 的 §9、§10、§14 决定。
- 同义标签应尽量统一：`failed_breakout` → `breakout_failure`，`wire` → `barbwire`。
- MRV（小趋势反转）不单独枚举；用 `reversal_attempt` 表达，勿轻易标 `mtr`。
