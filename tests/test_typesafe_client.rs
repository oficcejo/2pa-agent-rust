use okx_2pa_agent::ai::typesafe::{
    QuestionType, TypeSafeClient, TypeSafeQuestion, TypeSafeRequest, TypeSafeResponse,
};
use serde_json::json;
use std::collections::HashMap;

#[test]
fn test_typesafe_question_constructors() {
    // 1. Noul question
    let noul_q = TypeSafeQuestion::noul("Is the market in an uptrend?");
    assert_eq!(noul_q.question_type, QuestionType::Noul);
    assert_eq!(
        noul_q.instructions,
        json!("Is the market in an uptrend?")
    );
    assert!(noul_q.criteria.is_none());

    // 2. Noul with criteria
    let noul_crit = TypeSafeQuestion::noul_with_criteria(
        "Does this bar confirm a second entry?",
        "High conviction second entry",
        "False breakout or chop",
    );
    assert_eq!(noul_crit.question_type, QuestionType::Noul);
    assert!(noul_crit.criteria.is_some());
    assert_eq!(
        noul_crit.criteria.as_ref().unwrap()["true"],
        "High conviction second entry"
    );

    // 3. Choice question
    let mut options = HashMap::new();
    options.insert("bull".to_string(), "Bull trend".to_string());
    options.insert("bear".to_string(), "Bear trend".to_string());
    options.insert("chop".to_string(), "Range chop".to_string());
    let choice_q = TypeSafeQuestion::choice("Classify trend regime", options);
    assert_eq!(choice_q.question_type, QuestionType::Choice);
    assert!(choice_q.criteria.is_some());

    // 4. Score question
    let score_q = TypeSafeQuestion::score(
        "Rate breakout strength",
        vec!["weak".to_string(), "medium".to_string(), "strong".to_string()],
    );
    assert_eq!(score_q.question_type, QuestionType::Score);
    assert!(score_q.criteria.is_some());
}

#[test]
fn test_typesafe_request_serialization() {
    let state = json!({
        "symbol": "BTC-USDT-SWAP",
        "close": 65000.0,
        "ema20": 64800.0
    });

    let mut questions = HashMap::new();
    questions.insert(
        "regime".to_string(),
        TypeSafeQuestion::noul("Is price above EMA20?"),
    );

    let req = TypeSafeRequest {
        state,
        model: "jev-latest".to_string(),
        questions,
    };

    let serialized = serde_json::to_string(&req).expect("serialize request");
    assert!(serialized.contains("jev-latest"));
    assert!(serialized.contains("BTC-USDT-SWAP"));
    assert!(serialized.contains("Is price above EMA20?"));
}

#[test]
fn test_typesafe_response_deserialization() {
    let raw_response = r#"{
        "model": "jev-1.13.0",
        "answers": {
            "regime": {
                "type": "choice",
                "choice": "bull_trend",
                "probabilities": {
                    "bull_trend": 0.88,
                    "bear_trend": 0.08,
                    "range_chop": 0.04
                },
                "confidence": 0.82
            },
            "is_h2": {
                "type": "noul",
                "noul": 0.94
            },
            "bar_quality": {
                "type": "score",
                "score": 1.85,
                "legend": {
                    "0": "weak",
                    "1": "moderate",
                    "2": "strong"
                },
                "probabilities": {
                    "0": 0.02,
                    "1": 0.11,
                    "2": 0.87
                },
                "confidence": 0.85
            }
        },
        "usage": {
            "input_tokens": 320,
            "output_tokens": 42
        }
    }"#;

    let parsed: TypeSafeResponse = serde_json::from_str(raw_response).expect("parse response");
    assert_eq!(parsed.model, "jev-1.13.0");
    assert_eq!(parsed.answers.len(), 3);

    // Verify Choice answer
    let regime = parsed.answers.get("regime").expect("regime answer");
    assert_eq!(regime.answer_type, QuestionType::Choice);
    assert_eq!(regime.choice.as_deref(), Some("bull_trend"));
    assert_eq!(regime.confidence, Some(0.82));
    assert_eq!(regime.effective_confidence(), 0.82);

    // Verify Noul answer
    let is_h2 = parsed.answers.get("is_h2").expect("is_h2 answer");
    assert_eq!(is_h2.answer_type, QuestionType::Noul);
    assert_eq!(is_h2.noul, Some(0.94));
    // Effective confidence for 0.94 should be (0.94 - 0.5) * 2 = 0.88
    assert!((is_h2.effective_confidence() - 0.88).abs() < 1e-4);

    // Verify Score answer
    let score = parsed.answers.get("bar_quality").expect("bar_quality answer");
    assert_eq!(score.answer_type, QuestionType::Score);
    assert_eq!(score.score, Some(1.85));
    assert_eq!(score.confidence, Some(0.85));
    assert_eq!(score.legend.as_ref().unwrap().get("2").unwrap(), "strong");
}

#[test]
fn test_typesafe_client_configuration() {
    let client = TypeSafeClient::new("jev-latest", "https://api.typesafe.ai/v1", "ts-test-key-123", 10);
    assert!(client.has_api_key());
    assert_eq!(client.model, "jev-latest");
    assert_eq!(client.base_url, "https://api.typesafe.ai/v1");

    let empty_client = TypeSafeClient::new("", "", "", 0);
    assert!(!empty_client.has_api_key());
    assert_eq!(empty_client.model, "jev-latest");
    assert_eq!(empty_client.base_url, "https://api.typesafe.ai/v1");
}
