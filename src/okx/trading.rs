use crate::data::timeframe_to_seconds;
use crate::okx::client::OKXClient;
use crate::util::timefmt::now_local_ms;
use anyhow::{anyhow, Result};
use parking_lot::{Mutex, ReentrantMutex};
use rust_decimal::prelude::*;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

/// Hardcoded Broker Tag for OKX rebate & attribution.
pub const BROKER_TAG: &str = "c314b0aecb5bBCDE";

pub const PA_CLIENT_ORDER_PREFIX: &str = "pa";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub submitted: bool,
    pub signal_id: String,
    pub request: Value,
    pub response: Option<Value>,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub error_code: String,
    #[serde(default)]
    pub broker_tag: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: String,
    pub timestamp_ms: i64,
    pub submitted: bool,
    pub signal_id: String,
    pub instrument: String,
    pub timeframe: String,
    pub direction: String,
    pub order_type: String,
    pub confidence: Option<Value>,
    pub size: Option<Value>,
    pub price: Option<Value>,
    pub stop_loss_price: Option<Value>,
    pub take_profit_price: Option<Value>,
    pub order_id: String,
    pub reason: String,
    pub error_code: String,
    pub broker_tag: String,
    #[serde(default)]
    pub deleted: bool,
}

#[derive(Debug, Clone)]
pub struct OKXTradeExecutor {
    client: OKXClient,
    default_order_size: Decimal,
    trade_mode: String,
    position_mode: String,
    default_leverage: Decimal,
    block_new_entries_when_position_open: bool,
    confidence_threshold: u32,
    max_signal_age_seconds: u64,
    pub max_pending_bars: usize,
    audit_path: Option<PathBuf>,
    seen: Arc<Mutex<HashSet<String>>>,
    audit_lock: Arc<ReentrantMutex<()>>,
}

impl OKXTradeExecutor {
    pub fn new(
        client: OKXClient,
        default_order_size: f64,
        trade_mode: &str,
        position_mode: &str,
        default_leverage: f64,
        block_new_entries_when_position_open: bool,
        confidence_threshold: u32,
        max_signal_age_seconds: u64,
        max_pending_bars: usize,
        audit_path: Option<PathBuf>,
    ) -> Self {
        let default_order_size = Decimal::from_f64_retain(default_order_size).unwrap_or(Decimal::ONE);
        let default_leverage = Decimal::from_f64_retain(default_leverage).unwrap_or(Decimal::from(3));

        let executor = Self {
            client,
            default_order_size,
            trade_mode: trade_mode.to_string(),
            position_mode: position_mode.to_string(),
            default_leverage,
            block_new_entries_when_position_open,
            confidence_threshold: confidence_threshold.min(100),
            max_signal_age_seconds: max_signal_age_seconds.max(5),
            max_pending_bars: max_pending_bars.max(1),
            audit_path,
            seen: Arc::new(Mutex::new(HashSet::new())),
            audit_lock: Arc::new(ReentrantMutex::new(())),
        };
        executor.load_seen();
        executor
    }

    fn load_seen(&self) {
        if let Some(path) = &self.audit_path {
            if path.is_file() {
                if let Ok(file) = File::open(path) {
                    let reader = BufReader::new(file);
                    let mut seen = self.seen.lock();
                    for line in reader.lines().flatten() {
                        if let Ok(val) = serde_json::from_str::<Value>(&line) {
                            if val.get("submitted").and_then(|v| v.as_bool()).unwrap_or(false) {
                                if let Some(sig) = val.get("signal_id").and_then(|v| v.as_str()) {
                                    seen.insert(sig.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn generate_signal_id(
        inst_id: &str,
        timeframe: &str,
        signal_ts_ms: i64,
        decision: &Value,
    ) -> String {
        let material = serde_json::json!({
            "inst_id": inst_id,
            "timeframe": timeframe,
            "signal_ts_ms": signal_ts_ms,
            "order_direction": decision.get("order_direction"),
            "order_type": decision.get("order_type"),
            "entry_price": decision.get("entry_price"),
            "stop_loss_price": decision.get("stop_loss_price"),
            "take_profit_price": decision.get("take_profit_price"),
        });
        let s = material.to_string();
        let mut hasher = Sha256::new();
        hasher.update(s.as_bytes());
        let hex = hex::encode(hasher.finalize());
        hex[..24].to_string()
    }

    fn audit(
        &self,
        result: &ExecutionResult,
        inst_id: &str,
        timeframe: &str,
        decision: &Value,
    ) {
        let path = match &self.audit_path {
            Some(p) => p,
            None => return,
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        let attached = result.request.get("attachAlgoOrds")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first());

        let _stops = attached.cloned().unwrap_or(Value::Null);

        let entry = serde_json::json!({
            "id": Uuid::new_v4().simple().to_string(),
            "ts_ms": now_ms,
            "inst_id": inst_id,
            "timeframe": timeframe,
            "submitted": result.submitted,
            "signal_id": result.signal_id,
            "request": result.request,
            "response": result.response,
            "reason": result.reason,
            "error_code": result.error_code,
            "broker_tag": BROKER_TAG,
            "decision": {
                "order_direction": decision.get("order_direction"),
                "order_type": decision.get("order_type"),
                "entry_price": decision.get("entry_price"),
                "stop_loss_price": decision.get("stop_loss_price"),
                "take_profit_price": decision.get("take_profit_price"),
                "trade_confidence": decision.get("trade_confidence"),
            }
        });

        let _guard = self.audit_lock.lock();
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(file, "{}", entry);
        }
    }

    pub fn audit_history(&self, limit: usize) -> Vec<AuditEntry> {
        let path = match &self.audit_path {
            Some(p) => p,
            None => return Vec::new(),
        };
        if !path.is_file() {
            return Vec::new();
        }

        let _guard = self.audit_lock.lock();
        let file = match File::open(path) {
            Ok(f) => f,
            Err(_) => return Vec::new(),
        };

        let reader = BufReader::new(file);
        let mut entries = Vec::new();

        for line in reader.lines().flatten() {
            if let Ok(item) = serde_json::from_str::<Value>(&line) {
                if item.get("deleted").and_then(|v| v.as_bool()).unwrap_or(false) {
                    continue;
                }

                let req = item.get("request").cloned().unwrap_or(Value::Null);
                let resp = item.get("response").cloned().unwrap_or(Value::Null);
                let dec = item.get("decision").cloned().unwrap_or(Value::Null);

                let attached = req.get("attachAlgoOrds").and_then(|v| v.as_array()).and_then(|a| a.first());
                let stops = attached.cloned().unwrap_or(Value::Null);

                let order_id = resp.get("ordId")
                    .or_else(|| resp.get("algoId"))
                    .or_else(|| resp.get("clOrdId"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                let id = item.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let timestamp_ms = item.get("ts_ms").and_then(|v| v.as_i64()).unwrap_or(0);
                let submitted = item.get("submitted").and_then(|v| v.as_bool()).unwrap_or(false);
                let signal_id = item.get("signal_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let instrument = req.get("instId").or_else(|| item.get("inst_id")).and_then(|v| v.as_str()).unwrap_or("").to_string();
                let timeframe = item.get("timeframe").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let direction = req.get("side").or_else(|| dec.get("order_direction")).and_then(|v| v.as_str()).unwrap_or("").to_string();
                let order_type = req.get("ordType").or_else(|| dec.get("order_type")).and_then(|v| v.as_str()).unwrap_or("").to_string();
                let confidence = dec.get("trade_confidence").cloned();
                let size = req.get("sz").cloned();
                let price = req.get("px").or_else(|| req.get("triggerPx")).or_else(|| dec.get("entry_price")).cloned();
                let stop_loss_price = stops.get("slTriggerPx").or_else(|| dec.get("stop_loss_price")).cloned();
                let take_profit_price = stops.get("tpTriggerPx").or_else(|| dec.get("take_profit_price")).cloned();
                let reason = item.get("reason").or_else(|| resp.get("sMsg")).and_then(|v| v.as_str()).unwrap_or("").to_string();
                let error_code = item.get("error_code").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let broker_tag = item.get("broker_tag").and_then(|v| v.as_str()).unwrap_or(BROKER_TAG).to_string();

                entries.push(AuditEntry {
                    id,
                    timestamp_ms,
                    submitted,
                    signal_id,
                    instrument,
                    timeframe,
                    direction,
                    order_type,
                    confidence,
                    size,
                    price,
                    stop_loss_price,
                    take_profit_price,
                    order_id,
                    reason,
                    error_code,
                    broker_tag,
                    deleted: false,
                });
            }
        }

        entries.sort_by(|a, b| b.timestamp_ms.cmp(&a.timestamp_ms));
        if entries.len() > limit {
            entries.truncate(limit);
        }
        entries
    }

    pub fn delete_audit_entry(&self, entry_id: &str) -> bool {
        let path = match &self.audit_path {
            Some(p) => p,
            None => return false,
        };
        if !path.is_file() || entry_id.is_empty() {
            return false;
        }

        let _guard = self.audit_lock.lock();
        let file = match File::open(path) {
            Ok(f) => f,
            Err(_) => return false,
        };

        let reader = BufReader::new(file);
        let mut output = Vec::new();
        let mut found = false;

        for line in reader.lines().flatten() {
            if let Ok(item) = serde_json::from_str::<Value>(&line) {
                let id = item.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let is_del = item.get("deleted").and_then(|v| v.as_bool()).unwrap_or(false);

                if !found && id == entry_id && !is_del {
                    found = true;
                    if item.get("submitted").and_then(|v| v.as_bool()).unwrap_or(false) {
                        let tombstone = serde_json::json!({
                            "id": entry_id,
                            "ts_ms": item.get("ts_ms"),
                            "submitted": true,
                            "signal_id": item.get("signal_id"),
                            "deleted": true,
                        });
                        output.push(tombstone.to_string());
                    }
                    continue;
                }
                output.push(line);
            }
        }

        if !found {
            return false;
        }

        if let Ok(mut out_file) = File::create(path) {
            for l in output {
                let _ = writeln!(out_file, "{}", l);
            }
            true
        } else {
            false
        }
    }

    fn validate_prices(&self, decision: &Value) -> Result<(Decimal, Decimal, Decimal)> {
        let entry_f = decision.get("entry_price").and_then(|v| v.as_f64()).ok_or_else(|| anyhow!("缺少入场价 (entry_price)"))?;
        let stop_f = decision.get("stop_loss_price").and_then(|v| v.as_f64()).ok_or_else(|| anyhow!("缺少止损价 (stop_loss_price)"))?;
        let target_f = decision.get("take_profit_price").and_then(|v| v.as_f64()).ok_or_else(|| anyhow!("缺少止盈价 (take_profit_price)"))?;

        let entry = Decimal::from_f64_retain(entry_f).ok_or_else(|| anyhow!("无效的入场价数值"))?;
        let stop = Decimal::from_f64_retain(stop_f).ok_or_else(|| anyhow!("无效的止损价数值"))?;
        let target = Decimal::from_f64_retain(target_f).ok_or_else(|| anyhow!("无效的止盈价数值"))?;

        let direction = decision.get("order_direction").and_then(|v| v.as_str()).unwrap_or("");
        if direction == "做多" {
            if !(stop < entry && entry < target) {
                return Err(anyhow!("做多价格关系异常：必须满足 止损价 < 入场价 < 止盈价"));
            }
        } else if direction == "做空" {
            if !(target < entry && entry < stop) {
                return Err(anyhow!("做空价格关系异常：必须满足 止盈价 < 入场价 < 止损价"));
            }
        } else {
            return Err(anyhow!("订单方向必须为 做多 或 做空"));
        }

        Ok((entry, stop, target))
    }

    fn floor_step(value: Decimal, step: Decimal) -> Decimal {
        if step <= Decimal::ZERO { return value; }
        (value / step).floor() * step
    }

    fn round_tick(value: Decimal, tick: Decimal) -> Decimal {
        if tick <= Decimal::ZERO { return value; }
        (value / tick).round() * tick
    }

    pub async fn build_request(
        &self,
        inst_id: &str,
        decision: &Value,
        signal_id: &str,
    ) -> Result<(Value, bool)> {
        let order_type = decision.get("order_type").and_then(|v| v.as_str()).unwrap_or("");
        if !["限价单", "突破单", "市价单"].contains(&order_type) {
            return Err(anyhow!("决策为不下单或不包含可执行订单"));
        }

        let confidence = decision.get("trade_confidence").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        if confidence < self.confidence_threshold {
            return Err(anyhow!("交易信心度 {}% 低于设定风控门槛 {}%", confidence, self.confidence_threshold));
        }

        let (entry, stop, target) = self.validate_prices(decision)?;

        // Query instrument specs from OKX
        let inst_type = if inst_id.ends_with("-SWAP") {
            "SWAP"
        } else if inst_id.split('-').count() >= 4 {
            "FUTURES"
        } else {
            "SPOT"
        };

        let insts = self.client.get_instruments(inst_type, Some(inst_id)).await?;
        let instrument = insts.into_iter().next().ok_or_else(|| anyhow!("未找到 OKX 合约/产品规格: {}", inst_id))?;

        let tick_sz_str = instrument.get("tickSz").and_then(|v| v.as_str()).unwrap_or("0.00000001");
        let lot_sz_str = instrument.get("lotSz").and_then(|v| v.as_str()).unwrap_or("0.00000001");
        let min_sz_str = instrument.get("minSz").and_then(|v| v.as_str()).unwrap_or(lot_sz_str);

        let tick_sz = Decimal::from_str(tick_sz_str).unwrap_or(Decimal::new(1, 8));
        let lot_sz = Decimal::from_str(lot_sz_str).unwrap_or(Decimal::new(1, 8));
        let min_sz = Decimal::from_str(min_sz_str).unwrap_or(lot_sz);

        let size = Self::floor_step(self.default_order_size, lot_sz);
        if size < min_sz {
            return Err(anyhow!("下单数量 {} 低于交易所最小下单量 {}", size, min_sz));
        }

        let direction = decision.get("order_direction").and_then(|v| v.as_str()).unwrap_or("");
        let side = if direction == "做多" { "buy" } else { "sell" };
        let td_mode = if inst_type == "SPOT" && self.trade_mode == "cash" { "cash" } else { &self.trade_mode };

        let entry_s = Self::round_tick(entry, tick_sz).to_string();
        let stop_s = Self::round_tick(stop, tick_sz).to_string();
        let target_s = Self::round_tick(target, tick_sz).to_string();

        let attached_tp_sl = serde_json::json!([{
            "tpTriggerPx": target_s,
            "tpOrdPx": "-1",
            "tpTriggerPxType": "last",
            "slTriggerPx": stop_s,
            "slOrdPx": "-1",
            "slTriggerPxType": "last",
        }]);

        let mut req = serde_json::json!({
            "instId": inst_id,
            "tdMode": td_mode,
            "side": side,
            "sz": size.to_string(),
            "tag": BROKER_TAG, // Hardcoded Broker Tag
        });

        if inst_type != "SPOT" && self.position_mode == "long_short" {
            req["posSide"] = serde_json::json!(if direction == "做多" { "long" } else { "short" });
        }

        if inst_type == "SPOT" {
            if td_mode == "cross" {
                let quote_ccy = instrument.get("quoteCcy").and_then(|v| v.as_str()).unwrap_or("USDT");
                req["ccy"] = serde_json::json!(quote_ccy);
            } else if order_type == "市价单" {
                req["tgtCcy"] = serde_json::json!("base_ccy");
            }
        }

        let cl_id = format!("{}{}", PA_CLIENT_ORDER_PREFIX, signal_id);
        let cl_id_truncated = if cl_id.len() > 32 { &cl_id[..32] } else { &cl_id };

        if order_type == "突破单" {
            req["ordType"] = serde_json::json!("trigger");
            req["triggerPx"] = serde_json::json!(entry_s);
            req["orderPx"] = serde_json::json!("-1");
            req["triggerPxType"] = serde_json::json!("last");
            req["algoClOrdId"] = serde_json::json!(cl_id_truncated);
            req["attachAlgoOrds"] = attached_tp_sl;
            return Ok((req, true));
        }

        req["ordType"] = serde_json::json!(if order_type == "市价单" { "market" } else { "limit" });
        req["clOrdId"] = serde_json::json!(cl_id_truncated);
        if order_type == "限价单" {
            req["px"] = serde_json::json!(entry_s);
        }
        req["attachAlgoOrds"] = attached_tp_sl;

        Ok((req, false))
    }

    pub async fn execute(
        &self,
        inst_id: &str,
        timeframe: &str,
        signal_ts_ms: i64,
        decision: &Value,
    ) -> ExecutionResult {
        let signal_id = Self::generate_signal_id(inst_id, timeframe, signal_ts_ms, decision);
        
        let tf_seconds = timeframe_to_seconds(timeframe).unwrap_or(300);
        let bar_duration_ms = (tf_seconds as i64) * 1000;
        let bar_close_ts_ms = signal_ts_ms + bar_duration_ms;
        let now_ms = now_local_ms();

        // 计算自该 K 线闭合时刻起经过的实际秒数（若本地时间落后则按 0 处理）
        let age_since_close_seconds = if now_ms > bar_close_ts_ms {
            ((now_ms - bar_close_ts_ms) as f64) / 1000.0
        } else {
            0.0
        };

        // 允许的最大过期秒数：以最大挂单保留 K 线数 (max_pending_bars) 乘以周期时长为基准，确保在有效窗口内均可正常下单
        let max_age_allowed = ((self.max_pending_bars.max(1) as u64) * tf_seconds).max(self.max_signal_age_seconds) as f64;

        if age_since_close_seconds > max_age_allowed {
            let res = ExecutionResult {
                submitted: false,
                signal_id: signal_id.clone(),
                request: Value::Null,
                response: None,
                reason: format!("信号已过期 (K线闭合距今已过 {:.0} 秒，超过最大容许时效 {:.0} 秒)", age_since_close_seconds, max_age_allowed),
                error_code: String::new(),
                broker_tag: BROKER_TAG.to_string(),
            };
            self.audit(&res, inst_id, timeframe, decision);
            return res;
        }

        {
            let seen = self.seen.lock();
            if seen.contains(&signal_id) {
                return ExecutionResult {
                    submitted: false,
                    signal_id: signal_id.clone(),
                    request: Value::Null,
                    response: None,
                    reason: "重复信号 (当前K线周期已处理或挂单)".to_string(),
                    error_code: String::new(),
                    broker_tag: BROKER_TAG.to_string(),
                };
            }
        }

        let (request, is_algo) = match self.build_request(inst_id, decision, &signal_id).await {
            Ok(r) => r,
            Err(e) => {
                let res = ExecutionResult {
                    submitted: false,
                    signal_id: signal_id.clone(),
                    request: Value::Null,
                    response: None,
                    reason: e.to_string(),
                    error_code: String::new(),
                    broker_tag: BROKER_TAG.to_string(),
                };
                self.audit(&res, inst_id, timeframe, decision);
                return res;
            }
        };

        // Guard check: Open position check for derivatives
        let is_derivative = inst_id.ends_with("-SWAP") || inst_id.split('-').count() >= 4;
        if is_derivative && self.block_new_entries_when_position_open {
            if let Ok(positions) = self.client.get_positions(Some(inst_id)).await {
                for pos in positions {
                    let pos_sz = pos.get("pos").and_then(|v| v.as_str()).unwrap_or("0");
                    if let Ok(p_dec) = Decimal::from_str(pos_sz) {
                        if p_dec != Decimal::ZERO {
                            let res = ExecutionResult {
                                submitted: false,
                                signal_id: signal_id.clone(),
                                request: request.clone(),
                                response: None,
                                reason: format!("{} 已存在活跃持仓，系统已启用持仓互斥保护（禁止同向加仓）", inst_id),
                                error_code: String::new(),
                                broker_tag: BROKER_TAG.to_string(),
                            };
                            self.audit(&res, inst_id, timeframe, decision);
                            return res;
                        }
                    }
                }
            }

            // Set leverage
            let _ = self.client.set_leverage(inst_id, &self.default_leverage.to_string(), &self.trade_mode).await;
        }

        // Cancel-Replace 机制：下达新委托前，自动撤销同品种此前未成交的旧限价挂单与旧突破挂单（新委托无缝替代旧委托，不堆积订单）
        if let Ok(old_orders) = self.client.get_pending_orders(Some(inst_id)).await {
            for old_ord in old_orders {
                let old_ord_id = old_ord.get("ordId").and_then(|v| v.as_str());
                let old_cl_ord_id = old_ord.get("clOrdId").and_then(|v| v.as_str());
                if old_ord_id.is_some() || old_cl_ord_id.is_some() {
                    let _ = self.client.cancel_order(inst_id, old_ord_id, old_cl_ord_id).await;
                }
            }
        }
        if let Ok(old_algos) = self.client.get_pending_algo_orders(Some(inst_id), "trigger").await {
            for old_algo in old_algos {
                let algo_id = old_algo.get("algoId").and_then(|v| v.as_str()).unwrap_or("");
                let cl_id = old_algo.get("algoClOrdId").and_then(|v| v.as_str()).unwrap_or("");
                // 只撤销本系统生成的突破挂单，不影响已成交持仓附带的止盈止损条件单
                if !algo_id.is_empty() && cl_id.starts_with(PA_CLIENT_ORDER_PREFIX) {
                    let _ = self.client.cancel_algo_order(inst_id, algo_id).await;
                }
            }
        }

        // Place order
        let order_res = if is_algo {
            self.client.place_algo_order(&request).await
        } else {
            self.client.place_order(&request).await
        };

        match order_res {
            Ok(resp) => {
                {
                    let mut seen = self.seen.lock();
                    seen.insert(signal_id.clone());
                }
                let res = ExecutionResult {
                    submitted: true,
                    signal_id: signal_id.clone(),
                    request: request.clone(),
                    response: Some(resp),
                    reason: String::new(),
                    error_code: String::new(),
                    broker_tag: BROKER_TAG.to_string(),
                };
                self.audit(&res, inst_id, timeframe, decision);
                res
            }
            Err(e) => {
                let res = ExecutionResult {
                    submitted: false,
                    signal_id: signal_id.clone(),
                    request: request.clone(),
                    response: None,
                    reason: e.to_string(),
                    error_code: String::new(),
                    broker_tag: BROKER_TAG.to_string(),
                };
                self.audit(&res, inst_id, timeframe, decision);
                res
            }
        }
    }
}
