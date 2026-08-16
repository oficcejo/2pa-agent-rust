use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    let t = text.trim();
    if let Some(caps) = Regex::new(r"(?s)```(?:json)?\s*(.*?)\s*```").unwrap().captures(t) {
        if let Some(m) = caps.get(1) {
            return m.as_str().trim().to_string();
        }
    }
    let without_leading = Regex::new(r"(?i)^```(?:json)?\s*\n?").unwrap().replace(t, "");
    let without_trailing = Regex::new(r"\n?```\s*$").unwrap().replace(&without_leading, "");
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
            let repaired = Regex::new(r",\s*([\]}])").unwrap().replace_all(&json_candidate, "$1");
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

pub fn validate_stage1_json(val: &Value, raw_text: &str) -> Result<Value, ValidationError> {
    let mut missing: Vec<String> = Vec::new();

    let obj = match val.as_object() {
        Some(o) => o,
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

    if !obj.contains_key("cycle_position") { missing.push("cycle_position".to_string()); }
    if !obj.contains_key("dominant_force") { missing.push("dominant_force".to_string()); }
    if !obj.contains_key("gate_result") { missing.push("gate_result".to_string()); }

    if !missing.is_empty() {
        return Err(ValidationError {
            category: "b".to_string(),
            stage: "stage1".to_string(),
            raw_text: raw_text.to_string(),
            parse_position: None,
            missing_fields: missing,
            invalid_fields: Vec::new(),
            message: "Missing required fields in Stage 1".to_string(),
        });
    }

    Ok(val.clone())
}

pub fn validate_stage2_json(val: &Value, raw_text: &str) -> Result<Value, ValidationError> {
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

    // Coherence check: if order_type is 不下单, entry/stop/target should be null or 0
    if let Some(d) = decision {
        let order_type = d.get("order_type").and_then(|v| v.as_str()).unwrap_or("");
        if order_type == "不下单" {
            // prices must be empty or null
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
