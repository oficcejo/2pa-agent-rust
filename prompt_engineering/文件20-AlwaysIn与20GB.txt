【文件定位】本文件职责：Always In（AIL/AIS）、强趋势、均线缺口棒（gap_bar）与 20GB 的方向过滤与逆势禁止。`detected_patterns` 含 always_in / ail / ais / 20gb / gap_bar 时加载。

⚠️ **PA Agent 适配**：只输出单次三价；下文「已有顺势仓」「wait」等为背景知识，禁止写入 JSON。

⚠️ **术语拆分**（禁止混用）：
- `opening_gap`：开盘跳空（GU/GD），见 spike 与 gap 叙事
- `gap_bar`：均线缺口棒（Moving Average Gap Bar），整根 K 线在 EMA 另一侧
- `20gb`：约 20 根连续未触及 EMA

---

## 1. Always In Long / Short

### 1.1 AIL（Always In Long）倾向

- 近端（K8）多数收在 EMA 上方，加权同侧占比高
- 回撤浅，空头反转信号连续失败
- 多头趋势棒有跟随，更高低点
- 下跌尝试被快速买回

### 1.2 AIS（Always In Short）倾向

- 近端多数收在 EMA 下方
- 反弹浅，多头反转连续失败
- 空头趋势棒有跟随，更低高点

### 1.3 交易含义

- **AIL 中首个做空反转** → 仅诊断；§14 逆势禁止（无例外）
- **AIS 中首个做多反转** → 同上
- 顺 Always In：信号不完美也可评估回撤入场（H1/H2、§9.0P 计划型限价）
- `bar_analysis.always_in` 与 `order_direction` 冲突时，reasoning 须写双确认

⚠️ **宽通道**：**禁止逆势**；仅顺 Always In / direction 一侧。

---

## 2. 20GB（Twenty Gap Bars）

**定义**：连续约 20 根 K 线**未触及** EMA20（几何特征表 `ema_gap_count≥20`）。

**含义**：
- 趋势极强，均值回归风险上升
- **无明确反转前不逆势**交易 20GB 趋势
- 勿仅因 20GB 判定趋势结束（禁止在 JSON 写wait/收紧止损）

**第一次触 EMA**：
- 高胜率**顺势刮头皮**结构之一，预期至少测试原趋势极点
- 须信号棒 + 小止损 + §10.3

**两次失败规则**：
- 第一次 20GB/均线缺口棒结构失败 → 可等第二次
- **两次都失败** → 不再第三次强行尝试，回阶段一重判（通道/区间/反转）

§14 须扫描「20GB 逆势交易」：`answer=是` 表示触犯。

---

## 3. 均线缺口棒（gap_bar）

**定义**（非开盘跳空）：
- 多头背景：整根 K 线高点 **低于** EMA，K 线与 EMA 之间有空隙
- 空头背景：整根 K 线低点 **高于** EMA

**含义**：
- 第一根 gap_bar 常意味回撤已够深，**大概率测试原趋势极点**
- 仍须信号棒、入场棒、上下文确认
- reasoning 须写「第一根 gap_bar」或「第二次 gap_bar 尝试」

**与 opening_gap**：开盘跳空用 `opening_gap`；勿把中文「缺口」混指两类。

---

## 4. Gap Up / Gap Down（开盘跳空，辅助）

- **GU**：开盘高于前日高点区域 → 偏多背景，但须看首 3–5 棒是否反转
- **GD**：开盘低于前日低点区域 → 偏空背景
- 跳空后尖峰 → `spike_candidate` / `spike_active`；非单独下单依据

---

## 5. 逆 Always In 的「双确认」【仅 watch_points 监测，禁止逆势三价】

Brooks 背景：须同时满足才可*考虑*反转监测（**不构成**下单依据）：
1. 趋势线/通道有效突破（收盘确认）
2. 前极点测试失败（或 MTR 组件齐全）
3. 优先 **H2/L2** 或二次入场监测，非 H1

**PA Agent**：满足亦仅写 `watch_points`；`order_type=不下单`；全局禁令禁止一切逆势三价。

---

## 6. 阶段二 checklist

- [ ] `always_in` 与 `order_direction` 一致？若否，双确认是否写明？
- [ ] `ema_gap_count≥20` 是否扫描 §14？
- [ ] gap_bar 是第几根？叙事是否区分 opening_gap？
- [ ] 20GB 后逆势单是否触犯 §14？
