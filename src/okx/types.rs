use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OKXApiResponse<T> {
    pub code: String,
    #[serde(default)]
    pub msg: String,
    #[serde(default)]
    pub data: Vec<T>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OKXInstrument {
    #[serde(rename = "instId")]
    pub inst_id: String,
    #[serde(rename = "instType")]
    pub inst_type: String,
    #[serde(rename = "baseCcy", default)]
    pub base_ccy: String,
    #[serde(rename = "quoteCcy", default)]
    pub quote_ccy: String,
    #[serde(rename = "tickSz", default)]
    pub tick_sz: String,
    #[serde(rename = "lotSz", default)]
    pub lot_sz: String,
    #[serde(rename = "minSz", default)]
    pub min_sz: String,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OKXAccountBalance {
    #[serde(rename = "uTime", default)]
    pub u_time: String,
    #[serde(rename = "totalEq", default)]
    pub total_eq: String,
    #[serde(rename = "isoEq", default)]
    pub iso_eq: String,
    #[serde(rename = "adjEq", default)]
    pub adj_eq: String,
    #[serde(rename = "ordFzs", default)]
    pub ord_fzs: String,
    #[serde(default)]
    pub details: Vec<serde_json::Value>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OKXPosition {
    #[serde(rename = "instId")]
    pub inst_id: String,
    #[serde(rename = "instType", default)]
    pub inst_type: String,
    #[serde(rename = "mgnMode", default)]
    pub mgn_mode: String,
    #[serde(rename = "posSide", default)]
    pub pos_side: String,
    #[serde(default)]
    pub pos: String,
    #[serde(rename = "avgPx", default)]
    pub avg_px: String,
    #[serde(default)]
    pub upl: String,
    #[serde(rename = "uplRatio", default)]
    pub upl_ratio: String,
    #[serde(rename = "lever", default)]
    pub lever: String,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OKXOrderResponse {
    #[serde(rename = "ordId", default)]
    pub ord_id: String,
    #[serde(rename = "clOrdId", default)]
    pub cl_ord_id: String,
    #[serde(rename = "algoId", default)]
    pub algo_id: String,
    #[serde(rename = "algoClOrdId", default)]
    pub algo_cl_ord_id: String,
    #[serde(rename = "sCode", default)]
    pub s_code: String,
    #[serde(rename = "sMsg", default)]
    pub s_msg: String,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}
