【文件定位】本文件职责：规范 Major Trend Reversal（MTR，主要趋势反转）的四组件、概率与阶段二决策。`detected_patterns` 含 `mtr` 或 `entry_setup_type=MTR` 时加载（叠加文件15）。

⚠️ MRV（Minor Trend Reversal，小趋势反转）≠ MTR：小反转更可能只是回撤或进入区间，不要过早标 `mtr`。

⚠️ **PA Agent**：MTR / `mtr` 标签**仅诊断**；四组件齐全亦**禁止**逆原趋势三价；`order_type=不下单`，`terminal=wait`。

---

## 1. MTR 定义与概率

MTR 是**逆原主趋势**的可靠反转 attempt，目标形成新的 HH/HL 或 LL/LH 序列。

**概率锚点**（阶段二 `estimated_win_rate` 自述以此为准）：
- **完整 MTR**（四组件齐全）第一次反转尝试成功率约 **35–40%**（多数失败变旗形或区间）。**一般**趋势中的单根/弱反转尝试失败率更高（约 80%，见 `提示词大纲` 简表），勿混用两种口径。
- 第二次反转尝试成为主要反转的概率约 **40%**，**优先等待 H2/L2 或二次入场监测**（仅诊断时写 watch_points，**禁止逆势三价**）
- 即使图形完美，MTR 单笔胜率 rarely 超过 50%——须严格 §10.3（**仅适用于顺 direction 方案**）

---

## 2. MTR 四组件（缺一不可标 mtr）

1. **原趋势存在**：长程结构窗口有清晰 bull/bear trend（HH+HL 或 LL+LH）。
2. **趋势线/通道突破**：价格收盘突破主要趋势线或牛市/熊市通道线（非单根毛刺）。
3. **趋势恢复失败**：突破后原趋势方未能恢复（无强跟随、无新高/新低）。
4. **前极点测试失败**：价格测试前高/前低但无法创出超越性新高/新低（双顶底或更低高点/更高低点）。

**阶段一**：仅有单根反转棒 → `reversal_attempt`，**不要**写 `mtr`。
**阶段二**：四组件齐全 → **仅诊断**；**禁止**产出逆原趋势三价；`order_type=不下单`，`terminal=wait`。

---

## 3. MTR 与 H1/H2/L1/L2

| 背景 | 第一次信号 | 推荐 |
|------|------------|------|
| 强趋势 AIL/AIS | H1/L1 逆势 | 几乎总是等待 |
| 趋势线刚突破 | 第一次回撤入场 | 谨慎，偏 H2/L2 |
| 极点测试失败 | 第二次触发 | 仅诊断；**禁止** MTR 逆势三价 |
| 楔形三推后 | 第一次突破 | 禁止追顺势；禁止逆势；watch_points |

`bar_analysis.signal_bar.pattern`：MTR 场景优先 H2/L2；若用 H1/L1，reasoning 须逐条反驳「为何不等第二次」。

---

## 4. MTR vs 其他反转标签

| 标签 | 含义 | detected_patterns |
|------|------|-------------------|
| reversal_attempt | 有反转迹象，未完整 | reversal_attempt |
| mtr | 四组件基本齐全 | mtr + reversal_attempt |
| final_flag | 趋势末端旗形 | final_flag（见文件24） |
| wedge 反转 | 三推楔形 | wedge |
| MRV | 小级别反转，多半回撤 | 不写 mtr，写 reversal_attempt 或 none |

---

## 5. 入场、止损、目标【Brooks 背景；本 Agent 禁止逆势三价】

以下为 Brooks 识别逻辑，**不得**转化为 decision 三价：

**识别要点**：
- 二次确认：极点测试失败后的回撤再突破（背景知识）。
- 信号棒质量、铁丝网/区间中部信号无效等判断仍用于**诊断**与 watch_points。

**PA Agent 强制输出**：
- `order_type=不下单`
- `terminal.outcome=wait`，`node_id` 引用 §7 或 §8
- `watch_points`：四组件状态、监测触发价、失效条件

---

## 6. §14 与常见禁止

- 尖峰/微型通道中标 `mtr` → 仅诊断，**禁止**逆势三价。
- 仅单根外包棒/锤子线 → 禁止标 `mtr` 并暗示可下单。
- Always In 强趋势中首个逆势 MTR → **禁止下单**（无例外）。
- FF 失败但无极点测试失败 → `reversal_attempt`，wait + watch_points。

---

## 7. 阶段二 JSON 映射

- `bar_analysis.entry_setup_type` = `MTR`（**仅标签**，不代表可下单）
- `detected_patterns` 须含 `mtr` 与 `reversal_attempt`（四组件齐全时）
- `order_type` = **不下单**（强制，无例外）
- `terminal.outcome` = **wait**；reasoning 写明「MTR 仅诊断，全局禁令禁止逆势下单」
- `decision_trace` §7/§8 分支须覆盖 MTR 相关节点
- 方程是否通过**不影响**上述结论——MTR 场景一律 wait
