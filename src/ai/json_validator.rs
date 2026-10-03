use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::OnceLock;

fn re_think() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?s)<think>.*?</think>").expect("think regex"))
}

fn re_fenced() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?s)```(?:json)?\s*(.*?)\s*```").expect("fence regex"))
}

fn re_leading_fence() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)^```(?:json)?\s*\n?").expect("leading fence regex"))
}

fn re_trailing_fence() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\n?```\s*$").expect("trailing fence regex"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationError {
    pub category: String, // "a" (syntax), "b" (missing), "c" (illegal), "d" (plain text), "e" (provider error)
    pub stage: String,
    pub raw_text: String,
    pub parse_position: Option<String>,
    #[serde(default)]
    pub missing_fields: Vec<String>,
    #[serde(default)]
    pub invalid_fields: Vec<String>,
    #[serde(default)]
    pub message: String,
}

pub fn strip_markdown_fences(text: &str) -> String {
    let without_think = re_think().replace_all(text, "");
    let t = without_think.trim();
    if let Some(caps) = re_fenced().captures(t) {
        if let Some(m) = caps.get(1) {
            return m.as_str().trim().to_string();
        }
    }
    let without_leading = re_leading_fence().replace(t, "");
    let without_trailing = re_trailing_fence().replace(&without_leading, "");
    without_trailing.trim().to_string()
}

pub fn extract_outer_json_object(text: &str) -> String {
    let stripped = strip_markdown_fences(text);
    let start = match stripped.find('{') {
        Some(pos) => pos,
        None => return stripped,
    };

    let mut depth = 0;
    let mut in_string = false;
    let mut escape = false;

    let chars: Vec<char> = stripped[start..].chars().collect();
    for (i, &ch) in chars.iter().enumerate() {
        if in_string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        if ch == '"' {
            in_string = true;
            continue;
        }
        if ch == '{' {
            depth += 1;
        } else if ch == '}' {
            depth -= 1;
            if depth == 0 {
                return chars[..=i].iter().collect::<String>();
            }
        }
    }

    stripped[start..].to_string()
}

fn repair_trailing_commas(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut quoted = false;
    let mut escaped = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if escaped { escaped = false; }
            else if c == '\\' { escaped = true; }
            else if c == '"' { quoted = false; }
        } else if c == '"' { quoted = true; }
        else if c == ',' {
            let next = chars.clone().find(|ch| !ch.is_whitespace());
            if matches!(next, Some('}' | ']')) { continue; }
        }
        out.push(c);
    }
    out
}

pub fn parse_and_clean_json(text: &str, stage: &str) -> Result<Value, ValidationError> {
    let raw = text.trim();
    if raw.is_empty() {
        return Err(ValidationError {
            category: "d".to_string(),
            stage: stage.to_string(),
            raw_text: text.to_string(),
            parse_position: None,
            missing_fields: Vec::new(),
            invalid_fields: Vec::new(),
            message: "LLM output is empty".to_string(),
        });
    }

    let json_candidate = extract_outer_json_object(raw);
    match serde_json::from_str::<Value>(&json_candidate) {
        Ok(v) => {
            if !v.is_object() {
                return Err(ValidationError {
                    category: "d".to_string(),
                    stage: stage.to_string(),
                    raw_text: json_candidate,
                    parse_position: None,
                    missing_fields: Vec::new(),
                    invalid_fields: Vec::new(),
                    message: "Output is not a JSON object".to_string(),
                });
            }
            Ok(v)
        }
        Err(e) => {
            // Attempt simple trailing comma repair
            let repaired = repair_trailing_commas(&json_candidate);
            if let Ok(v) = serde_json::from_str::<Value>(&repaired) {
                if v.is_object() {
                    return Ok(v);
                }
            }

            Err(ValidationError {
                category: "a".to_string(),
                stage: stage.to_string(),
                raw_text: json_candidate,
                parse_position: Some(format!("{}", e)),
                missing_fields: Vec::new(),
                invalid_fields: Vec::new(),
                message: format!("JSON parse error: {}", e),
            })
        }
    }
}

fn find_alias_recursive(
    node: &Value,
    alts: &[&str],
    depth: usize,
    max_depth: usize,
) -> Option<(String, Value)> {
    if depth > max_depth {
        return None;
    }
    match node {
        Value::Object(map) => {
            for alt in alts {
                if let Some(v) = map.get(*alt) {
                    return Some((alt.to_string(), v.clone()));
                }
            }
            for v in map.values() {
                if let Some(hit) = find_alias_recursive(v, alts, depth + 1, max_depth) {
                    return Some(hit);
                }
            }
            None
        }
        Value::Array(arr) => {
            for v in arr {
                if let Some(hit) = find_alias_recursive(v, alts, depth + 1, max_depth) {
                    return Some(hit);
                }
            }
            None
        }
        _ => None,
    }
}

fn collect_stage1_keys(
    node: &Value,
    need: &[&str],
    depth: usize,
    max_depth: usize,
    out: &mut Vec<(String, Value)>,
) {
    if depth > max_depth || out.len() >= need.len() {
        return;
    }
    match node {
        Value::Object(map) => {
            for key in need {
                if !out.iter().any(|(k, _)| k == key) {
                    if let Some(v) = map.get(*key) {
                        out.push((key.to_string(), v.clone()));
                    }
                }
            }
            for v in map.values() {
                if out.len() >= need.len() {
                    break;
                }
                collect_stage1_keys(v, need, depth + 1, max_depth, out);
            }
        }
        Value::Array(arr) => {
            for v in arr {
                if out.len() >= need.len() {
                    break;
                }
                collect_stage1_keys(v, need, depth + 1, max_depth, out);
            }
        }
        _ => {}
    }
}

pub fn validate_stage1_json(val: &Value, raw_text: &str) -> Result<Value, ValidationError> {
    let mut missing: Vec<String> = Vec::new();

    let mut obj_map = match val.as_object() {
        Some(o) => o.clone(),
        None => {
            return Err(ValidationError {
                category: "d".to_string(),
                stage: "stage1".to_string(),
                raw_text: raw_text.to_string(),
                parse_position: None,
                missing_fields: vec!["root object".to_string()],
                invalid_fields: Vec::new(),
                message: "Stage 1 output must be a JSON object".to_string(),
            });
        }
    };

    // Alias / camelCase recovery before failing the response.
    let aliases: &[(&str, &[&str])] = &[
        ("cycle_position", &["cyclePosition", "cycle", "cycle_pattern", "period_shape", "period", "market_state", "市场状态", "周期位置", "周期形态", "周期", "形态"]),
        ("dominant_force", &["dominantForce", "dominant", "force", "主趋势", "优势方", "主导力量", "主导力量描述", "主导力量方向"]),
        ("gate_result", &["gateResult", "gate", "gate_status", "闸门结果", "门控结果", "阶段一_闸门结果", "闸门判定", "门控"]),
    ];

    // Recover required keys from one nested wrapper level (common LLM shape).
    for wrapper in [
        "diagnosis",
        "stage1",
        "result",
        "data",
        "phase1_gate",
        "phase",
        "market_state",
        "market_diagnosis",
        "阶段一_市场诊断",
        "阶段一_闸门",
        "phase_1_gate",
        "market_diagnosis.phase1_gate",
        "market_diagnosis.phase_1_gate",
        "analysis",
    ] {
        // Support "parent.child" wrappers by walking the path.
        let inner_val: Option<Value> = if wrapper.contains('.') {
            let mut cur: Option<&Value> = Some(&Value::Null);
            // rebuild from obj_map each path segment
            let mut node = Value::Object(obj_map.clone());
            for seg in wrapper.split('.') {
                node = match node.get(seg) {
                    Some(v) => v.clone(),
                    None => Value::Null,
                };
            }
            if node.is_object() {
                Some(node)
            } else {
                let _ = cur.take();
                None
            }
        } else {
            obj_map.get(wrapper).cloned()
        };

        let recovered: Vec<(String, Value)> = match inner_val.as_ref().and_then(|v| v.as_object()) {
            Some(inner) => ["cycle_position", "dominant_force", "gate_result"]
                .iter()
                .filter(|key| !obj_map.contains_key(**key) && inner.contains_key(**key))
                .map(|key| (key.to_string(), inner[*key].clone()))
                .collect(),
            None => Vec::new(),
        };
        for (k, v) in recovered {
            obj_map.insert(k, v);
        }
    }

    // phase1_gate / phase_1_gate often nests the gate verdict one level down.
    if !obj_map.contains_key("gate_result") {
        if let Some(pg) = obj_map
            .get("market_diagnosis")
            .and_then(|v| v.get("phase1_gate"))
            .or_else(|| obj_map.get("phase1_gate"))
            .or_else(|| obj_map.get("phase_1_gate"))
        {
            if let Some(g) = pg
                .get("gate_result")
                .or_else(|| pg.get("result"))
                .or_else(|| pg.get("status"))
            {
                obj_map.insert("gate_result".to_string(), g.clone());
            } else if let Some(g) = pg.as_str() {
                obj_map.insert("gate_result".to_string(), Value::String(g.to_string()));
            }
        }
    }

    // Generic: any top-level key whose name contains "gate" may hold the verdict.
    if !obj_map.contains_key("gate_result") {
        for (k, v) in &obj_map {
            if !k.to_lowercase().contains("gate") {
                continue;
            }
            if let Some(g) = v
                .get("gate_result")
                .or_else(|| v.get("result"))
                .or_else(|| v.get("status"))
                .or_else(|| v.get("value"))
            {
                obj_map.insert("gate_result".to_string(), g.clone());
                break;
            } else if let Some(s) = v.as_str() {
                let sl = s.to_ascii_lowercase();
                if sl == "proceed" || sl == "wait" {
                    obj_map.insert("gate_result".to_string(), Value::String(s.to_string()));
                    break;
                }
            }
        }
    }

    // market_diagnosis.cycle_pattern / period_shape is the Stage-1 cycle label.
    if !obj_map.contains_key("cycle_position") {
        if let Some(md) = obj_map.get("market_diagnosis") {
            if let Some(cp) = md
                .get("cycle_pattern")
                .or_else(|| md.get("period_shape"))
                .or_else(|| md.get("cycle"))
                .or_else(|| md.get("period"))
            {
                obj_map.insert("cycle_position".to_string(), cp.clone());
            }
        }
    }

    // Depth-limited search: pick up keys nested one more level (e.g. diagnosis.cycle_position).
    let need = ["cycle_position", "dominant_force", "gate_result"];
    let missing_now: Vec<&str> = need
        .iter()
        .copied()
        .filter(|k| !obj_map.contains_key(*k))
        .collect();
    if !missing_now.is_empty() {
        let mut found: Vec<(String, Value)> = Vec::new();
        collect_stage1_keys(&Value::Object(obj_map.clone()), &need, 0, 4, &mut found);
        // Also look for alias names anywhere in the tree.
        for (canonical, alts) in aliases {
            if obj_map.contains_key(*canonical) {
                continue;
            }
            if found.iter().any(|(k, _)| k == canonical) {
                continue;
            }
            if let Some((_k, v)) = find_alias_recursive(&Value::Object(obj_map.clone()), alts, 0, 5) {
                found.push((canonical.to_string(), v));
            }
        }
        for (k, v) in found {
            if !obj_map.contains_key(&k) {
                obj_map.insert(k, v);
            }
        }
    }

    for (canonical, alts) in aliases {
        if obj_map.contains_key(*canonical) {
            continue;
        }
        for alt in *alts {
            if let Some(v) = obj_map.get(*alt) {
                obj_map.insert(canonical.to_string(), v.clone());
                break;
            }
        }
    }

    if !obj_map.contains_key("cycle_position") {
        missing.push("cycle_position".to_string());
    }
    if !obj_map.contains_key("dominant_force") {
        missing.push("dominant_force".to_string());
    }
    if !obj_map.contains_key("gate_result") {
        missing.push("gate_result".to_string());
    }

    if !missing.is_empty() {
        // Inventory nested objects so the next log line shows where fields actually live.
        let mut nested_keys: Vec<String> = Vec::new();
        for (k, v) in &obj_map {
            if let Some(inner) = v.as_object() {
                let ks: Vec<&str> = inner.keys().map(|s| s.as_str()).collect();
                if !ks.is_empty() {
                    nested_keys.push(format!("{}:{{{}}}", k, ks.join(",")));
                }
            }
        }
        return Err(ValidationError {
            category: "b".to_string(),
            stage: "stage1".to_string(),
            raw_text: raw_text.to_string(),
            parse_position: None,
            missing_fields: missing.clone(),
            invalid_fields: Vec::new(),
            message: format!(
                "Missing required fields in Stage 1: {} (present: {}; nested: {})",
                missing.join(", "),
                {
                    let keys: Vec<&str> = obj_map.keys().map(|s| s.as_str()).collect();
                    if keys.is_empty() {
                        "<none>".to_string()
                    } else {
                        keys.join(",")
                    }
                },
                if nested_keys.is_empty() {
                    "<none>".to_string()
                } else {
                    nested_keys.join(" | ")
                }
            ),
        });
    }

    // Return a repaired object with recovered aliases applied.
    Ok(Value::Object(obj_map))
}

pub fn validate_stage2_json_with_stage1(val: &Value, raw_text: &str, stage1: Option<&Value>) -> Result<Value, ValidationError> {
    let mut missing = Vec::new();
    let mut invalid = Vec::new();

    let obj = match val.as_object() {
        Some(o) => o,
        None => {
            return Err(ValidationError {
                category: "d".to_string(),
                stage: "stage2".to_string(),
                raw_text: raw_text.to_string(),
                parse_position: None,
                missing_fields: vec!["root object".to_string()],
                invalid_fields: Vec::new(),
                message: "Stage 2 output must be a JSON object".to_string(),
            });
        }
    };

    let decision = obj.get("decision").and_then(|v| v.as_object());
    if decision.is_none() {
        missing.push("decision".to_string());
    } else {
        let d = decision.unwrap();
        if !d.contains_key("order_type") { missing.push("decision.order_type".to_string()); }
        if !d.contains_key("order_direction") { missing.push("decision.order_direction".to_string()); }
    }

    if !missing.is_empty() {
        return Err(ValidationError {
            category: "b".to_string(),
            stage: "stage2".to_string(),
            raw_text: raw_text.to_string(),
            parse_position: None,
            missing_fields: missing,
            invalid_fields: Vec::new(),
            message: "Missing required fields in Stage 2".to_string(),
        });
    }

    // Coherence check: if order_type is 不下单 / 持有 / 平仓 / 修改止损
    if let Some(d) = decision {
        let order_type = d.get("order_type").and_then(|v| v.as_str()).unwrap_or("");
        let action = d.get("action").and_then(|v| v.as_str()).unwrap_or("");
        
        if order_type == "不下单" || order_type == "持有" || order_type == "平仓" || action == "HOLD" || action == "CLOSE_EARLY" || action == "WAIT" {
            // prices can be empty or null
        } else if order_type == "修改止损" || action == "MOVE_STOP_LOSS" {
            let new_sl = d.get("new_stop_loss_price").and_then(|v| v.as_f64())
                .or_else(|| d.get("stop_loss_price").and_then(|v| v.as_f64()));
            if new_sl.is_none() || new_sl.unwrap() <= 0.0 {
                invalid.push("修改止损必须提供有效的 new_stop_loss_price 或 stop_loss_price".to_string());
            }
        } else if order_type == "修改止盈" || action == "MOVE_TAKE_PROFIT" || action == "TRAILING_TAKE_PROFIT" {
            let new_tp = d.get("new_take_profit_price").and_then(|v| v.as_f64())
                .or_else(|| d.get("take_profit_price").and_then(|v| v.as_f64()));
            if new_tp.is_none() || new_tp.unwrap() <= 0.0 {
                invalid.push("修改止盈必须提供有效的 new_take_profit_price 或 take_profit_price".to_string());
            }
        } else if order_type == "修改止盈止损" || action == "MOVE_SL_TP" {
            let new_sl = d.get("new_stop_loss_price").and_then(|v| v.as_f64())
                .or_else(|| d.get("stop_loss_price").and_then(|v| v.as_f64()));
            let new_tp = d.get("new_take_profit_price").and_then(|v| v.as_f64())
                .or_else(|| d.get("take_profit_price").and_then(|v| v.as_f64()));
            if (new_sl.is_none() || new_sl.unwrap() <= 0.0) && (new_tp.is_none() || new_tp.unwrap() <= 0.0) {
                invalid.push("修改止盈止损至少需要提供有效的止损或止盈价格".to_string());
            }
        } else if ["限价单", "突破单", "市价单"].contains(&order_type) {
            let entry = d.get("entry_price").and_then(|v| v.as_f64());
            let stop = d.get("stop_loss_price").and_then(|v| v.as_f64());
            let target = d.get("take_profit_price").and_then(|v| v.as_f64());
            let dir = d.get("order_direction").and_then(|v| v.as_str()).unwrap_or("");

            match (entry, stop, target) {
                (Some(e), Some(s), Some(t)) => {
                    if dir == "做多" && !(s < e && e < t) {
                        invalid.push("做多要求: 止损价 < 入场价 < 止盈价".to_string());
                    } else if dir == "做空" && !(t < e && e < s) {
                        invalid.push("做空要求: 止盈价 < 入场价 < 止损价".to_string());
                    }

                    // 1. 最低止损距离硬风控（防日内随机噪音被秒扫损）
                    if e > 0.0 && s > 0.0 {
                        let stop_dist = (e - s).abs();
                        let stop_ratio = stop_dist / e;
                        if stop_ratio < 0.0030 {
                            invalid.push(format!(
                                "止损距离过窄 (仅 {:.3}%, 约 {:.2} 点)，落入随机噪音区极易被扫损。止损必须保留足够结构缓冲 (建议 >= 1.0 ATR 或 >= 0.5%)",
                                stop_ratio * 100.0, stop_dist
                            ));
                        }
                    }

                    // 2. 阶段一形态与阶段二决策冲突硬拦截 (Semantic Anti-Conflict)
                    // Versioned strategies validate raw market evidence in the orchestrator.
                    // Legacy pattern-name heuristics conflict with confirmed reversal setups.
                    if let Some(st1) = stage1.filter(|s| s.get("program_candidates").is_none()) {
                        let patterns: Vec<String> = st1.get("detected_patterns")
                            .and_then(|v| v.as_array())
                            .map(|arr| arr.iter().filter_map(|x| x.as_str().map(|s| s.to_lowercase())).collect())
                            .unwrap_or_default();
                        let pat_set: std::collections::HashSet<_> = patterns.into_iter().collect();
                        let dominant = st1.get("dominant_force").and_then(|v| v.as_str()).unwrap_or("");
                        let cycle = st1.get("cycle_position").and_then(|v| v.as_str()).unwrap_or("");

                        if dir == "做多" {
                            if pat_set.contains("double_top_candidate") || pat_set.contains("rejection_at_high") {
                                invalid.push("阶段一已诊断出高位受阻 (rejection_at_high) 或双顶 (double_top_candidate)，阶段二严禁在高位做多，必须保持观望或寻找做空机会".to_string());
                            }

                            if dominant.eq_ignore_ascii_case("bears") && (cycle.contains("overstretched_bearish") || cycle.contains("spike")) {
                                let has_reversal = pat_set.contains("double_bottom") 
                                    || pat_set.contains("double_bottom_candidate")
                                    || pat_set.contains("h2") 
                                    || pat_set.contains("break_above_sma14")
                                    || pat_set.contains("exhaustion");
                                if !has_reversal {
                                    invalid.push("市场处于强空头单边下跌趋势 (bears) 且无明确底部反转形态确认时，严禁左侧逆势盲目猜底做多".to_string());
                                }
                            }
                        } else if dir == "做空" {
                            if pat_set.contains("double_bottom_candidate") || pat_set.contains("rejection_at_low") {
                                invalid.push("阶段一已诊断出低位受阻 (rejection_at_low) 或双底 (double_bottom_candidate)，阶段二严禁在低位做空，必须保持观望或寻找做多机会".to_string());
                            }

                            if dominant.eq_ignore_ascii_case("bulls") && (cycle.contains("overstretched_bullish") || cycle.contains("spike")) {
                                let has_reversal = pat_set.contains("double_top") 
                                    || pat_set.contains("double_top_candidate")
                                    || pat_set.contains("l2") 
                                    || pat_set.contains("break_below_sma14")
                                    || pat_set.contains("exhaustion");
                                if !has_reversal {
                                    invalid.push("市场处于强多头单边上涨趋势 (bulls) 且无明确顶部反转形态确认时，严禁左侧逆势盲目摸顶做空".to_string());
                                }
                            }
                        }
                    }
                }
                _ => {
                    invalid.push("下单状态必须填写有效的 entry_price, stop_loss_price, take_profit_price".to_string());
                }
            }
        }
    }

    if !invalid.is_empty() {
        return Err(ValidationError {
            category: "c".to_string(),
            stage: "stage2".to_string(),
            raw_text: raw_text.to_string(),
            parse_position: None,
            missing_fields: Vec::new(),
            invalid_fields: invalid,
            message: "Logical consistency / price coherence failed".to_string(),
        });
    }

    Ok(val.clone())
}

pub fn validate_stage2_json(val: &Value, raw_text: &str) -> Result<Value, ValidationError> {
    validate_stage2_json_with_stage1(val, raw_text, None)
}
