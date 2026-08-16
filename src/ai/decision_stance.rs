pub fn normalize_stance(value: &str) -> &'static str {
    let key = value.trim().to_lowercase();
    match key.as_str() {
        "conservative" | "保守" => "conservative",
        "balanced" | "均衡" => "balanced",
        "aggressive" | "激进" => "aggressive",
        "extreme_aggressive" | "extreme" | "极度激进" => "extreme_aggressive",
        _ => "balanced",
    }
}

pub fn stance_label_zh(stance: &str) -> &'static str {
    match normalize_stance(stance) {
        "conservative" => "保守",
        "balanced" => "均衡",
        "aggressive" => "激进",
        "extreme_aggressive" => "极度激进",
        _ => "均衡",
    }
}

pub fn build_decision_stance_guidance(stance: &str) -> String {
    let normalized = normalize_stance(stance);
    let label = stance_label_zh(normalized);

    let common_rules = "\
通用约束（各档都必须遵守）：
- 仍必须完整输出 decision_trace，按 §9–§11、§14 走适用节点，不得伪造 trace。
- 节点 10.3 须基于已拟定的 entry/stop/target 做数值判断；禁止无止损、无目标。
- **定价**：先定结构 entry/TP1/TP2，再定结构 stop；RR≥1.0。若 RR>1.0 程序会自动向外扩 stop（保持 TP 不变）。RR<1.0 时收紧 stop 或调整 entry，禁止向外扩 stop。禁止缩小 TP 凑 RR。
- **突破单不可行时尝试限价单**：无合格突破锚点/信号已失效时，若结构位可挂限价且 10.3 方程可通过（期望为正），应输出限价单而非默认观望。
- **§9.0 / §9.0P 计划型限价**：宽通道/区间/通道靠边界、或顺势回撤到结构位时，无已收盘信号棒 → §9.0=否，必须评估 §9.0P；§9.0P=是 时可输出限价单，signal_bar.quality 可为 weak/invalid。
- **direction=neutral 约束**：neutral 时 §9.0P 仅允许顺 §2.4（AIL→只做多侧，AIS→只做空侧；§2.4=否→默认 wait）；禁止上下边界双向刮头皮。
- **方案连续性**：若上一轮可执行方案未失效，默认 wait；同结构位 N 根内禁止反手（详见阶段二「方案连续性」块）。
- **计划型限价 K1 对照**：只看 entry 相对 K1.close 方向；影线触及 entry 不算失效。
- 完成 10.3 后必须在 decision 中填写 estimated_win_rate（0–100）与 estimated_win_rate_reasoning；order_type=不下单 时 estimated_win_rate 必须为 null。
- **decision 与 trace/terminal 必须一致**：decision_trace 中 10.3=否、terminal.outcome 为 wait/reject、或 §14 判「是」时，decision.order_type 必须为「不下单」，entry/stop/target/order_direction/entry_basis_* 全部为 null，禁止一边写突破单/限价单价格一边在 trace 里判方程不通过。
- 有下单 → terminal.outcome=trade；不下单 → outcome=wait 或 reject。
- 触犯 §14 硬性禁止项时，各档均须 order_type=不下单，并在 reasoning 明确写出触犯的条款。";

    let profile = match normalized {
        "conservative" => "\
【保守】= 当前系统默认裁定标准。
- §9 入场：优先典型、清晰、收盘确认的一类信号；次优/模糊 setup 默认继续等待。
- **§9.0P 计划型限价（边界例外）**：宽通道/区间/通道靠边界时，即使 direction=neutral、K1 为 doji/弱棒/无信号棒，仍应先评估 §9.0P 边界限价三价；中部位置或无结构锚点时才默认 wait。
- §10：止损必须明确且不过大；10.3 交易者方程边际情况倾向判「否」。RR 须 ≥1.0 且方程通过。
- §14：从严扫描；有疑虑即不下单。
- trade_confidence：40–59 或结构存在明显歧义时，优先 order_type=不下单。
- 交易区间/通道 **中部**、方向中性、信号棒质量一般时，默认观望；靠近支撑/阻力/通道边界时不在此列。",
        "balanced" => "\
【均衡】= 在遵守决策树的前提下，比【保守】更愿意执行交易；背景与周期优先于独立信号棒。
- §9.0P 计划型限价（默认路径）：当 cycle_position 为 broad_channel / trading_range / normal_channel / trending_tr，且 gate_result=proceed——即使无强信号棒，§9.0=否 后 必须写 §9.0P 并尝试背景限价三价。
- §9.0P=是 时：signal_bar.bar=null、quality=invalid/weak；entry_bar pending；禁止只在 watch_points 写方案而不填 decision 三价。
- §9 入场：除典型信号外，若结构与阶段一 direction/cycle_position 一致，允许「次优但可执行」的二类 setup（须在 reason 中写明为何仍值得做）。
- 宽通道/区间边界：优先限价单路径；weak/invalid 信号棒或无信号棒配合 tr_boundary/L1/H1 等 pattern 仍可下单，须在 §9/reasoning 说明接受的瑕疵。
- §10：10.3 边际可通过时，若方程为正且结构清晰，可判「是」；须在 trade_confidence_reasoning 写明假设。
- §14：仅明确触犯才不下单；不要因「不够完美」单独放弃。
- trade_confidence：35–49 且入场逻辑完整时，可给出下单方案（在 reasoning 说明风险克制）。
- 顺势通道/尖峰延续、区间边界反弹：方向与周期一致时可优先考虑限价单，而非默认等待。",
        "aggressive" => "\
【激进】= 在遵守决策树的前提下，比【均衡】更愿意执行交易。
- §9 入场：结构方向一致时，可接受更早、更不完美的入场触发；须在 reason 说明接受的瑕疵与失效条件。
- §10：10.3 在 entry/stop/target 已明确时，若方程略偏边际但方向与周期位置一致，可判「是」；须在 reasoning 强调风险克制。
- §14：仅硬性禁止项触发不下单；不要因为「理想目标位更远」而放弃可执行方案。
- trade_confidence：30–44 且逻辑链完整时，仍可输出具体下单类型；用 watch_points / invalidation_condition 补足不确定性。
- 趋势延续、突破回踩、区间边界：只要阶段一 gate_result=proceed，应主动寻找可下单方案，而不是先找理由观望。",
        _ => "\
【极度激进】= 强制产出交易；在 10.3 可通过且未触犯 §14 时，必须给出具体进场方案，禁止因犹豫而输出「不下单」。
- 拟下单路径（仅当 10.3 判「是」且 terminal.outcome=trade 时生效）：order_type 为「限价单」「突破单」「市价单」之一，order_direction 为「做多」或「做空」，entry/stop/target 为有效数值。
- 不下单路径（10.3=否、方程不达标、或 §14=是）：order_type=「不下单」，所有价格字段为 null；terminal.outcome=reject 或 wait。
- 强制选定方向（仅拟下单时）：综合阶段一、HTF 与最近 K 线，在多空之间选一个更优方向；neutral 时根据区间位置与近 3–5 根 K 线二选一。
- §9：信号不完美也可判「是」，但须在 reason 写明接受的瑕疵。
- §10.3：突破单须先用依据 K 的极值±1跳动写出 entry，再写 stop/target，用这三价做检验；RR 须 ≥1.0 且胜率×回报>败率×风险，否则必须判「否」并走不下单路径。
- 突破单不可行时，优先评估限价单结构位方案，方程通过即可下单。
- trade_confidence 可低至 25–40，但仅适用于 10.3=是 的下单方案。",
    };

    format!(
        "## 交易倾向（当前：{} / {}）\n\n{}\n{}\n请在 decision.reasoning 与 trade_confidence_reasoning 中体现本档位如何影响最终裁定。",
        label, normalized, common_rules, profile
    )
}
