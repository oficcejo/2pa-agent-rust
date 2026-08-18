use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordMeta {
    pub timestamp_local_iso: String,
    pub timestamp_local_ms: i64,
    pub symbol: String,
    pub timeframe: String,
    pub bar_count: usize,
    #[serde(default)]
    pub ai_provider: Value,
    #[serde(default = "default_stance")]
    pub decision_stance: String,
    #[serde(default = "default_trading_system")]
    pub trading_system: String,
}

fn default_stance() -> String { "balanced".to_string() }
fn default_trading_system() -> String { "2pa".to_string() }


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisRecord {
    pub meta: RecordMeta,
    #[serde(default)]
    pub kline_data: Vec<Value>,
    #[serde(default)]
    pub htf_text: String,
    #[serde(default)]
    pub stage1_messages: Vec<Value>,
    #[serde(default)]
    pub stage1_response: Option<Value>,
    #[serde(default)]
    pub stage1_diagnosis: Option<Value>,
    #[serde(default)]
    pub stage2_messages: Vec<Value>,
    #[serde(default)]
    pub stage2_response: Option<Value>,
    #[serde(default)]
    pub stage2_decision: Option<Value>,
    #[serde(default)]
    pub strategy_files_used: Vec<String>,
    #[serde(default)]
    pub experience_loaded: Vec<Value>,
    #[serde(default)]
    pub exception: Option<Value>,
    #[serde(default)]
    pub usage_total: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperienceEntry {
    pub filename: String,
    pub case_type: String, // "success" or "failure"
    pub cycle_position: String,
    pub timestamp_ms: i64,
    pub content: Value,
}
