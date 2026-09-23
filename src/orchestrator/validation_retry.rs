use crate::ai::client::{AIClient, ChatMessage, LLMReply};
use crate::ai::json_validator::{parse_and_clean_json, validate_stage1_json, validate_stage2_json_with_stage1};
use crate::ai::retry_feedback::build_retry_feedback_prompt;
use anyhow::{anyhow, Result};
use serde_json::Value;
use tracing::{info, warn};

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

    for attempt in 0..=max_retries {
        info!("Calling Stage 1 AI (attempt {}/{})", attempt + 1, max_retries + 1);
        let reply = client.chat_completion(&messages).await?;

        match parse_and_clean_json(&reply.content, "stage1") {
            Ok(parsed) => {
                match validate_stage1_json(&parsed, &reply.content) {
                    Ok(validated) => {
                        messages.push(ChatMessage {
                            role: "assistant".to_string(),
                            content: reply.content.clone(),
                        });
                        return Ok((validated, reply, messages));
                    }
                    Err(e) => {
                        warn!("Stage 1 validation failed: {}", e.message);
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

    for attempt in 0..=max_retries {
        info!("Calling Stage 2 AI (attempt {}/{})", attempt + 1, max_retries + 1);
        let reply = client.chat_completion(&messages).await?;

        match parse_and_clean_json(&reply.content, "stage2") {
            Ok(parsed) => {
                match validate_stage2_json_with_stage1(&parsed, &reply.content, stage1_diagnosis) {
                    Ok(validated) => {
                        messages.push(ChatMessage {
                            role: "assistant".to_string(),
                            content: reply.content.clone(),
                        });
                        return Ok((validated, reply, messages));
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
