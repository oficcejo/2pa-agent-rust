# 知识库概念 ↔ PA Agent Prompt 映射（参考层，不参与每次加载）

维护说明：左侧为 `D:\价格行为学习资料\extracted_text` 双链库路径；右侧为 prompt_engineering 文件与 detected_patterns。

| 知识库概念/主题 | detected_patterns / entry_setup | prompt 文件 |
|-----------------|----------------------------------|-------------|
| 概念/突破.md | breakout_failure, breakout_test, breakout_pullback | 文件18 |
| 概念/假突破.md | breakout_failure | 文件18 |
| 概念/趋势.md | spike_*, always_in | 极速上涨/下跌、文件20 |
| 概念/震荡.md | middle_range, barbwire | 震荡区间、文件21 |
| 概念/通道.md | wedge, h1/h2 | 通道策略、文件14、19 |
| 概念/止损.md | — | 文件17 |
| 概念/反转.md | reversal_attempt, mtr | 文件15、25 |
| 阿布缩写 FF | final_flag | 文件24 |
| 阿布缩写 MM | （止盈结构） | 文件23 |
| 阿布缩写 MTR | mtr | 文件25 |
| 阿布缩写 MGN | magnet, failed_signal | 文件22 |
| 阿布缩写 H1/H2/L1/L2 | h1/h2/l1/l2 | 文件19 |
| 二元决策 §6.6 | ascending_triangle 等 | 文件27 |
| 10种最佳模式 | 多标签并存 | 提示词大纲 + 各专篇 |

## 刻意不纳入 prompt 的主题（按产品决策）

- 事件交易（FOMC/CPI 等）：不单独建文件29
- 开盘与日内结构专篇：不单独建文件26
