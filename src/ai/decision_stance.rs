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
    format!("决策风格：{}。所有风格必须满足相同的结构确认、成本和风险门槛；不得强制下单，不得把主观信心度当作胜率，不得调整止损制造盈亏比。", stance_label_zh(stance))
}
