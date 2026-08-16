use crate::ai::json_validator::ValidationError;

pub fn build_retry_feedback_prompt(err: &ValidationError) -> String {
    let mut feedback = String::from("上一轮输出未能通过 JSON Schema 或价格行为逻辑校验，请立即按以下指引修正并重新输出纯 JSON：\n");

    if !err.missing_fields.is_empty() {
        feedback.push_str(&format!("- 【缺少必填字段】：{}\n", err.missing_fields.join(", ")));
    }

    if !err.invalid_fields.is_empty() {
        feedback.push_str(&format!("- 【字段或逻辑错误】：{}\n", err.invalid_fields.join("; ")));
    }

    if !err.message.is_empty() {
        feedback.push_str(&format!("- 【错误详情】：{}\n", err.message));
    }

    feedback.push_str("\n请务必只输出修正后的裸 JSON（不得带有 ```json 代码块或前后任何解释文字）。");
    feedback
}
