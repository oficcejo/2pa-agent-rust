use crate::ai::client::{AIClient, ChatMessage, LLMReply, Usage};
use crate::ai::json_validator::{parse_and_clean_json, validate_stage1_json, validate_stage2_json_with_stage1};
use crate::ai::retry_feedback::build_retry_feedback_prompt;
use anyhow::{anyhow, Result};
use serde_json::Value;
use tracing::{info, warn};

/// Fold the tokens spent by failed attempts into the final reply's usage so
/// cost accounting reflects the whole retry loop, not just the last call.
fn with_total_usage(mut reply: LLMReply, prompt_tokens: usize, completion_tokens: usize) -> LLMReply {
    reply.usage = Usage {
        prompt_tokens,
        completion_tokens,
        total_tokens: prompt_tokens + completion_tokens,
    };
    reply
}

pub async fn call_and_validate_stage1(
    client: &AIClient,
    prompt: &str,
    max_retries: usize,
) -> Result<(Value, LLMReply, Vec<ChatMessage>)> {
    call_and_validate_stage1_messages(
        client,
        vec![ChatMessage {
            role: "user".to_string(),
            content: prompt.to_string(),
        }],
        max_retries,
    ).await
}

pub async fn call_and_validate_stage1_messages(
    client: &AIClient,
    mut messages: Vec<ChatMessage>,
    max_retries: usize,
) -> Result<(Value, LLMReply, Vec<ChatMessage>)> {
    let mut last_err = None;
    let mut usage_prompt = 0usize;
    let mut usage_completion = 0usize;

    for attempt in 0..=max_retries {
        info!("Calling Stage 1 AI (attempt {}/{})", attempt + 1, max_retries + 1);
        // Transport failures (timeout, 429, 5xx) used to escape through `?`
        // and abort the whole stage, which made the automation loop re-run the
        // FULL two-stage pipeline and pay for Stage 1 twice. Retry them here.
        let reply = match client.chat_completion(&messages).await {
            Ok(r) => r,
            Err(e) if attempt < max_retries => {
                warn!("Stage 1 请求失败（第 {}/{} 次尝试）: {e:#}；在重试预算内重试", attempt + 1, max_retries + 1);
                continue;
            }
            Err(e) => return Err(e.context("Stage 1 请求失败，重试预算已用尽")),
        };
        usage_prompt += reply.usage.prompt_tokens;
        usage_completion += reply.usage.completion_tokens;

        match parse_and_clean_json(&reply.content, "stage1") {
            Ok(parsed) => {
                match validate_stage1_json(&parsed, &reply.content) {
                    Ok(validated) => {
                        messages.push(ChatMessage {
                            role: "assistant".to_string(),
                            content: reply.content.clone(),
                        });
                        return Ok((
                            validated,
                            with_total_usage(reply, usage_prompt, usage_completion),
                            messages,
                        ));
                    }
                    Err(e) => {
                        warn!(
                            "Stage 1 validation failed: {} | missing={:?} invalid={:?}",
                            e.message,
                            e.missing_fields,
                            e.invalid_fields
                        );
                        last_err = Some(e);
                    }
                }
            }
            Err(e) => {
                warn!("Stage 1 JSON parse failed: {}", e.message);
                last_err = Some(e);
            }
        }

        if attempt < max_retries {
            if let Some(err) = &last_err {
                let feedback = build_retry_feedback_prompt(err);
                messages.push(ChatMessage {
                    role: "assistant".to_string(),
                    content: reply.content.clone(),
                });
                messages.push(ChatMessage {
                    role: "user".to_string(),
                    content: feedback,
                });
            }
        }
    }

    Err(anyhow!(
        "Stage 1 failed after {} retries: {:?}",
        max_retries,
        last_err.map(|e| e.message).unwrap_or_default()
    ))
}

pub async fn call_and_validate_stage2(
    client: &AIClient,
    prompt: &str,
    max_retries: usize,
    stage1_diagnosis: Option<&Value>,
) -> Result<(Value, LLMReply, Vec<ChatMessage>)> {
    call_and_validate_stage2_messages(
        client,
        vec![ChatMessage {
            role: "user".to_string(),
            content: prompt.to_string(),
        }],
        max_retries,
        stage1_diagnosis,
    ).await
}

pub async fn call_and_validate_stage2_messages(
    client: &AIClient,
    mut messages: Vec<ChatMessage>,
    max_retries: usize,
    stage1_diagnosis: Option<&Value>,
) -> Result<(Value, LLMReply, Vec<ChatMessage>)> {
    let mut last_err = None;
    let mut usage_prompt = 0usize;
    let mut usage_completion = 0usize;

    for attempt in 0..=max_retries {
        info!("Calling Stage 2 AI (attempt {}/{})", attempt + 1, max_retries + 1);
        let reply = match client.chat_completion(&messages).await {
            Ok(r) => r,
            Err(e) if attempt < max_retries => {
                warn!("Stage 2 请求失败（第 {}/{} 次尝试）: {e:#}；在重试预算内重试", attempt + 1, max_retries + 1);
                continue;
            }
            Err(e) => return Err(e.context("Stage 2 请求失败，重试预算已用尽")),
        };
        usage_prompt += reply.usage.prompt_tokens;
        usage_completion += reply.usage.completion_tokens;

        match parse_and_clean_json(&reply.content, "stage2") {
            Ok(parsed) => {
                match validate_stage2_json_with_stage1(&parsed, &reply.content, stage1_diagnosis) {
                    Ok(validated) => {
                        messages.push(ChatMessage {
                            role: "assistant".to_string(),
                            content: reply.content.clone(),
                        });
                        return Ok((
                            validated,
                            with_total_usage(reply, usage_prompt, usage_completion),
                            messages,
                        ));
                    }
                    Err(e) => {
                        warn!("Stage 2 validation failed: {}", e.message);
                        last_err = Some(e);
                    }
                }
            }
            Err(e) => {
                warn!("Stage 2 JSON parse failed: {}", e.message);
                last_err = Some(e);
            }
        }

        if attempt < max_retries {
            if let Some(err) = &last_err {
                let feedback = build_retry_feedback_prompt(err);
                messages.push(ChatMessage {
                    role: "assistant".to_string(),
                    content: reply.content.clone(),
                });
                messages.push(ChatMessage {
                    role: "user".to_string(),
                    content: feedback,
                });
            }
        }
    }

    Err(anyhow!(
        "Stage 2 failed after {} retries: {:?}",
        max_retries,
        last_err.map(|e| e.message).unwrap_or_default()
    ))
}
