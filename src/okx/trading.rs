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
    #[serde(default)]
    pub strategy_id: String,
    #[serde(default)]
    pub strategy_version: String,
    /// Decision record that produced this order. Ties the executed order back
    /// to the full analysis (prompt, diagnosis, proposal).
    #[serde(default)]
    pub decision_record_id: String,
    /// Prompt artifact version and hash live at decision time.
    #[serde(default)]
    pub prompt_version: String,
    #[serde(default)]
    pub prompt_hash: String,
    /// Market context, carried here so reconciliation needs no extra lookup.
    #[serde(default)]
    pub cycle_position: String,
    #[serde(default)]
    pub detected_patterns: Vec<String>,
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
    execution_lock: Arc<tokio::sync::Mutex<()>>,
    pub auto_order_sizing: bool,
    pub risk_percent: Decimal,
    pub max_margin_percent: Decimal,
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
        auto_order_sizing: bool,
        risk_percent: f64,
        max_margin_percent: f64,
    ) -> Self {
        let default_order_size = Decimal::from_f64_retain(default_order_size).unwrap_or(Decimal::ONE);
        let default_leverage = Decimal::from_f64_retain(default_leverage).unwrap_or(Decimal::from(3));
        let risk_percent = Decimal::from_f64_retain(risk_percent).unwrap_or(Decimal::from(2));
        let max_margin_percent = Decimal::from_f64_retain(max_margin_percent).unwrap_or(Decimal::from(25));

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
            execution_lock: Arc::new(tokio::sync::Mutex::new(())),
            auto_order_sizing,
            risk_percent,
            max_margin_percent,
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
                            if val.get("submitted").and_then(|v| v.as_bool()).unwrap_or(false)
                                || val["error_code"] == "SUBMISSION_UNCONFIRMED" {
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

    pub fn audit(
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
                "strategy_id": decision.get("strategy_id"),
                "strategy_version": decision.get("strategy_version"),
                "strategy_evidence": decision.get("strategy_evidence"),
                "order_direction": decision.get("order_direction"),
                "order_type": decision.get("order_type"),
                "entry_price": decision.get("entry_price"),
                "stop_loss_price": decision.get("stop_loss_price"),
                "take_profit_price": decision.get("take_profit_price"),
                "trade_confidence": decision.get("trade_confidence").or_else(|| decision.get("confidence")),
                // Receipt linkage: without these the audit row cannot be tied
                // back to a decision or a prompt revision.
                "decision_record_id": decision.get("decision_record_id"),
                "prompt_version": decision.get("prompt_version"),
                "prompt_hash": decision.get("prompt_hash"),
                "cycle_position": decision.get("cycle_position"),
                "detected_patterns": decision.get("detected_patterns"),
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
                let confidence = dec.get("trade_confidence").or_else(|| dec.get("confidence")).cloned();
                let size = req.get("sz").cloned();
                let price = req.get("px").or_else(|| req.get("triggerPx")).or_else(|| dec.get("entry_price")).cloned();
                let stop_loss_price = stops.get("slTriggerPx").or_else(|| dec.get("stop_loss_price")).cloned();
                let take_profit_price = stops.get("tpTriggerPx").or_else(|| dec.get("take_profit_price")).cloned();
                let reason = item.get("reason").or_else(|| resp.get("sMsg")).and_then(|v| v.as_str()).unwrap_or("").to_string();
                let error_code = item.get("error_code").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let broker_tag = item.get("broker_tag").and_then(|v| v.as_str()).unwrap_or(BROKER_TAG).to_string();

                let detected_patterns: Vec<String> = dec
                    .get("detected_patterns")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|p| p.as_str().map(|s| s.to_string())).collect())
                    .unwrap_or_default();

                entries.push(AuditEntry {
                    strategy_id: dec["strategy_id"].as_str().unwrap_or("legacy_unknown").into(),
                    strategy_version: dec["strategy_version"].as_str().unwrap_or("").into(),
                    decision_record_id: dec["decision_record_id"].as_str().unwrap_or("").into(),
                    prompt_version: dec["prompt_version"].as_str().unwrap_or("").into(),
                    prompt_hash: dec["prompt_hash"].as_str().unwrap_or("").into(),
                    cycle_position: dec["cycle_position"].as_str().unwrap_or("").into(),
                    detected_patterns,
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

        anyhow::ensure!(entry > Decimal::ZERO && stop > Decimal::ZERO && target > Decimal::ZERO, "价格必须为正数");
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

        // 1. 最低止损距离硬下限（防日内微观噪音秒扫）
        let stop_dist = (entry - stop).abs();
        if entry > Decimal::ZERO {
            let rel_dist = stop_dist / entry;
            if rel_dist < Decimal::from_str("0.0030").unwrap_or_default() {
                return Err(anyhow!(
                    "风控拦截: 止损距离过窄 (仅 {:.3}%, 约 {:.2} 点)，极易被日内随机微观噪波秒扫，拒绝开仓",
                    rel_dist * Decimal::from(100), stop_dist
                ));
            }
        }

        // 2. ATR 波动率下限校验
        if let Some(atr_val) = decision.get("atr14").and_then(|v| v.as_f64()) {
            if let Some(atr) = Decimal::from_f64_retain(atr_val) {
                if atr > Decimal::ZERO {
                    let min_atr_dist = atr * Decimal::from_str("0.8").unwrap_or(Decimal::ONE);
                    if stop_dist < min_atr_dist {
                        return Err(anyhow!(
                            "风控拦截: 止损距离 ({:.2}) 低于最低安全波动率下限 ({:.2} / 0.8 ATR)，拒绝开仓",
                            stop_dist, min_atr_dist
                        ));
                    }
                }
            }
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

    pub async fn compute_order_size(
        &self,
        inst_id: &str,
        inst_type: &str,
        entry: Decimal,
        stop: Decimal,
        lot_sz: Decimal,
        min_sz: Decimal,
        instrument: &Value,
    ) -> Result<Decimal> {
        anyhow::ensure!(entry > Decimal::ZERO && stop > Decimal::ZERO && entry != stop
            && lot_sz > Decimal::ZERO && min_sz > Decimal::ZERO, "无效的价格或下单规格");
        let derivative = ["SWAP", "FUTURES"].contains(&inst_type);
        anyhow::ensure!(instrument["ctType"].as_str() != Some("inverse"), "当前资金风控仅支持 USDT 线性合约及 USDT 现货，拒绝按 USDT 余额估算币本位保证金");
        let ct_val = if derivative {
            instrument["ctVal"].as_str().and_then(|v| Decimal::from_str(v).ok())
                .filter(|v| *v > Decimal::ZERO).ok_or_else(|| anyhow!("合约面值无效"))?
        } else { Decimal::ONE };
        let settlement = instrument.get(if derivative { "settleCcy" } else { "quoteCcy" }).and_then(Value::as_str).unwrap_or("USDT");
        anyhow::ensure!(settlement == "USDT", "当前资金风控仅支持 USDT 结算产品");
        let parse = |row: &Value, key: &str| row.get(key).and_then(Value::as_str).and_then(|v| Decimal::from_str(v).ok());
        let balances = self.client.get_account_balance().await?;
        let account = balances.first().ok_or_else(|| anyhow!("账户余额为空，拒绝使用默认数量"))?;
        let total = parse(account, "totalEq").filter(|v| *v > Decimal::ZERO).ok_or_else(|| anyhow!("账户权益无效"))?;
        let cash = account["details"].as_array().and_then(|rows| rows.iter().find(|r| r["ccy"] == "USDT"))
            .ok_or_else(|| anyhow!("缺少 USDT 可用余额"))?;
        let available = parse(cash, "availEq").or_else(|| parse(cash, "availBal"))
            .ok_or_else(|| anyhow!("USDT 可用保证金未知"))?;
        anyhow::ensure!(available > Decimal::ZERO, "USDT 可用保证金不足，拒绝新开仓");
        let lev = if inst_type == "SPOT" { Decimal::ONE } else { self.default_leverage };
        anyhow::ensure!(lev > Decimal::ZERO, "杠杆必须为正数");
        let notional = ct_val * entry;
        let fee_rate = if derivative { Decimal::new(7,4) } else { Decimal::new(12,4) };
        // Include the same slippage reserve as the strategy's net reward gate.
        let fees = ct_val * (entry + stop.max(entry)) * fee_rate;
        let margin_per_unit = notional / lev + fees;
        let risk_per_unit = (entry - stop).abs() * ct_val + fees;
        let risk_budget = total * self.risk_percent.clamp(Decimal::new(1,1), Decimal::from(20)) / Decimal::from(100);
        let margin_budget = available * self.max_margin_percent.clamp(Decimal::ONE, Decimal::from(100)) / Decimal::from(100);
        let max_size = (risk_budget / risk_per_unit).min(margin_budget / margin_per_unit);
        let target = if self.auto_order_sizing { max_size } else { self.default_order_size.min(max_size) };
        let size = Self::floor_step(target, lot_sz);
        anyhow::ensure!(size >= min_sz, "风控额度不足以满足最小下单量 {}；拒绝放宽风险或保证金上限 (可下 {}，标的 {})", min_sz, size, inst_id);
        Ok(size)
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
        if let Some(action) = decision.get("action").and_then(Value::as_str) {
            anyhow::ensure!(action == "OPEN" || action.is_empty(), "开仓订单类型与 action 冲突，拒绝执行");
        }

        let confidence = decision.get("trade_confidence").or_else(|| decision.get("confidence")).and_then(|v| v.as_u64()).filter(|v| *v <= 100).unwrap_or(0) as u32;
        if confidence > 100 || confidence < self.confidence_threshold {
            return Err(anyhow!("交易信心度 {}% 低于设定风控门槛 {}%", confidence, self.confidence_threshold));
        }

        let (mut entry, mut stop, mut target) = self.validate_prices(decision)?;
        if order_type == "市价单" {
            let ticker = self.client.get_ticker(inst_id).await?;
            let last = ticker["last"].as_str().and_then(|v| v.parse::<f64>().ok()).filter(|p| p.is_finite() && *p > 0.0)
                .ok_or_else(|| anyhow!("当前市场价格无效"))?;
            let mut current = decision.clone();
            current["entry_price"] = serde_json::json!(last);
            (entry, stop, target) = self.validate_prices(&current)?;
        }

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

        anyhow::ensure!(tick_sz > Decimal::ZERO && lot_sz > Decimal::ZERO && min_sz > Decimal::ZERO, "交易所价格或数量步长无效");
        entry = Self::round_tick(entry, tick_sz);
        stop = Self::round_tick(stop, tick_sz);
        target = Self::round_tick(target, tick_sz);
        let mut rounded = decision.clone();
        rounded["entry_price"] = serde_json::json!(entry.to_f64());
        rounded["stop_loss_price"] = serde_json::json!(stop.to_f64());
        rounded["take_profit_price"] = serde_json::json!(target.to_f64());
        self.validate_prices(&rounded)?;
        if let Some(id) = decision["strategy_id"].as_str() {
            anyhow::ensure!(id != "adaptive", "自适应模式仅供观察");
            if id != "alpha_pilot" {
                anyhow::ensure!(crate::strategies::canonical(id) == Some(id), "未知或旧版策略不能直接执行");
                let evidence: crate::strategies::Evidence = serde_json::from_value(decision["strategy_evidence"].clone())
                    .map_err(|_| anyhow!("缺少程序确认的策略证据"))?;
                anyhow::ensure!(evidence.strategy_id == id, "策略证据归属不一致");
                crate::strategies::validate_entry(inst_id, &rounded, &evidence)?;
            }
        }
        let mut size = self.compute_order_size(inst_id, inst_type, entry, stop, lot_sz, min_sz, &instrument).await?;
        if let Some(scale) = decision.get("position_scale") {
            let scale = scale.as_f64().filter(|s| s.is_finite() && *s > 0.0 && *s <= 1.0)
                .and_then(Decimal::from_f64_retain).ok_or_else(|| anyhow!("目标仓位比例必须在 (0,1] 内"))?;
            size = Self::floor_step(size * scale, lot_sz);
            anyhow::ensure!(size >= min_sz, "目标仓位低于最小下单量，拒绝向上补量");
        }

        // 3. 手续费与止损风险比重风控（防交易摩擦吞噬本金）
        let ct_val_str = instrument.get("ctVal").and_then(|v| v.as_str()).unwrap_or("1");
        let ct_val = Decimal::from_str(ct_val_str).unwrap_or(Decimal::ONE);
        let ct_type = instrument.get("ctType").and_then(|v| v.as_str()).unwrap_or("linear");

        let notional_usd = if ["SWAP", "FUTURES"].contains(&inst_type) {
            if ct_type == "inverse" { size * ct_val } else { size * ct_val * entry }
        } else {
            size * entry
        };

        let stop_dist = (entry - stop).abs();
        let risk_usd = if ["SWAP", "FUTURES"].contains(&inst_type) {
            if ct_type == "inverse" {
                if entry > Decimal::ZERO { (stop_dist / entry) * size * ct_val } else { size * ct_val }
            } else {
                stop_dist * size * ct_val
            }
        } else {
            stop_dist * size
        };

        let taker_fee_rate = if ["SWAP", "FUTURES"].contains(&inst_type) {
            Decimal::from_str("0.0005").unwrap_or(Decimal::ZERO)
        } else {
            Decimal::from_str("0.0010").unwrap_or(Decimal::ZERO)
        };
        let est_roundtrip_fee = notional_usd * taker_fee_rate * Decimal::from(2);
        let gross_reward = if ["SWAP", "FUTURES"].contains(&inst_type) { (target-entry).abs() * size * ct_val } else { (target-entry).abs() * size };
        anyhow::ensure!(gross_reward > est_roundtrip_fee, "止盈目标不足以覆盖预估往返手续费，拒绝开仓");

        if risk_usd > Decimal::ZERO && risk_usd < est_roundtrip_fee * Decimal::from_str("1.5").unwrap_or(Decimal::ONE) {
            return Err(anyhow!(
                "风控拦截: 止损风险 (${:.4}) 过于接近预估往返手续费 (${:.4})，交易摩擦侵蚀严重，期望值为负",
                risk_usd, est_roundtrip_fee
            ));
        }

        let direction = decision.get("order_direction").and_then(|v| v.as_str()).unwrap_or("");
        let side = if direction == "做多" { "buy" } else { "sell" };
        let td_mode = if inst_type == "SPOT" && self.trade_mode == "cash" { "cash" } else { &self.trade_mode };

        let entry_s = Self::round_tick(entry, tick_sz).to_string();
        let stop_s = Self::round_tick(stop, tick_sz).to_string();
        let target_s = Self::round_tick(target, tick_sz).to_string();

        let attached_tp_sl = serde_json::json!([{
            "attachAlgoClOrdId": format!("pa{}s", signal_id),
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

    pub async fn cancel_expired_entries(&self, inst_id: &str, timeframe: &str) -> Result<usize> {
        let _guard = self.execution_lock.lock().await;
        let lifetime = timeframe_to_seconds(timeframe).ok_or_else(|| anyhow!("未知 K 线周期"))?
            * self.max_pending_bars as u64 * 1000;
        let cutoff = now_local_ms().saturating_sub(lifetime as i64);
        let expired = |row: &Value| crate::web::positions::owned_order(row)
            && row["cTime"].as_str().and_then(|v| v.parse::<i64>().ok()).map(|t| t <= cutoff).unwrap_or(false);
        let mut count = 0;
        for row in self.client.get_pending_orders(Some(inst_id)).await? {
            if expired(&row) {
                let id = row["ordId"].as_str().ok_or_else(|| anyhow!("过期挂单缺少 ID"))?;
                self.client.cancel_order(inst_id, Some(id), None).await?;
                count += 1;
            }
        }
        // Never expire SL/TP orders: only pending entry triggers.
        for row in self.client.get_pending_algo_orders(Some(inst_id), "trigger").await? {
            if expired(&row) {
                let id = row["algoId"].as_str().ok_or_else(|| anyhow!("过期策略单缺少 ID"))?;
                self.client.cancel_algo_order(inst_id, id).await?;
                count += 1;
            }
        }
        Ok(count)
    }

    pub async fn execute(
        &self,
        inst_id: &str,
        timeframe: &str,
        signal_ts_ms: i64,
        decision: &Value,
    ) -> ExecutionResult {
        let _execution_guard = self.execution_lock.lock().await;
        let signal_id = Self::generate_signal_id(inst_id, timeframe, signal_ts_ms, decision);
        if decision.get("strategy_evidence").is_some() {
            let evidence = &decision["strategy_evidence"];
            if evidence["signal_ts_ms"].as_i64() != Some(signal_ts_ms)
                || evidence["timeframe"].as_str() != Some(timeframe)
                || evidence["symbol"].as_str() != Some(inst_id) {
                let res = ExecutionResult {
                    submitted: false, signal_id, request: Value::Null, response: None,
                    reason: "策略证据与标的、周期或信号时间不一致".into(),
                    error_code: String::new(), broker_tag: BROKER_TAG.into(),
                };
                self.audit(&res, inst_id, timeframe, decision);
                return res;
            }
        }
        
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

        // Signal freshness is independent of the lifetime of an already submitted entry order.
        let max_age_allowed = self.max_signal_age_seconds as f64;

        if now_ms < bar_close_ts_ms || age_since_close_seconds > max_age_allowed {
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

        let prepared: Result<()> = async {
            let is_derivative = inst_id.ends_with("-SWAP") || inst_id.split('-').count() >= 4;
            if is_derivative {
                let positions = self.client.get_positions(Some(inst_id)).await?;
                for pos in positions {
                    let size = pos["pos"].as_str().and_then(|v| Decimal::from_str(v).ok())
                        .ok_or_else(|| anyhow!("持仓数量无效，拒绝新开仓"))?;
                    if self.block_new_entries_when_position_open && size != Decimal::ZERO {
                        return Err(anyhow!("{} 已存在活跃持仓，系统已启用持仓互斥保护（禁止同向加仓）", inst_id));
                    }
                }
                let side = if self.position_mode == "long_short" { request["posSide"].as_str() } else { None };
                self.client.set_leverage_for_side(inst_id, &self.default_leverage.to_string(), &self.trade_mode, side).await?;
            }
            for old in self.client.get_pending_orders(Some(inst_id)).await? {
                if !crate::web::positions::owned_order(&old) { continue; }
                let id = old["ordId"].as_str().ok_or_else(|| anyhow!("旧挂单缺少订单 ID"))?;
                self.client.cancel_order(inst_id, Some(id), None).await?;
                // A cancel request may race a fill; verify terminal state before replacement.
                let state = self.client.get_order(inst_id, Some(id), None).await?;
                anyhow::ensure!(state["state"] == "canceled" && state["accFillSz"].as_str().and_then(|s| Decimal::from_str(s).ok()) == Some(Decimal::ZERO),
                    "旧挂单已成交、部分成交或撤单尚未确认，暂停替换");
            }
            for old in self.client.get_pending_algo_orders(Some(inst_id), "trigger").await? {
                if !crate::web::positions::owned_order(&old) { continue; }
                let id = old["algoId"].as_str().ok_or_else(|| anyhow!("旧策略单缺少 ID"))?;
                self.client.cancel_algo_order(inst_id, id).await?;
                let state = self.client.get_algo_order(Some(id), None).await?;
                anyhow::ensure!(state["state"] == "canceled", "旧策略单撤销尚未确认，暂停替换");
            }
            if is_derivative && self.block_new_entries_when_position_open {
                for pos in self.client.get_positions(Some(inst_id)).await? {
                    anyhow::ensure!(pos["pos"].as_str().and_then(|s| Decimal::from_str(s).ok()) == Some(Decimal::ZERO),
                        "提交前发现持仓变化或持仓数据无效，停止开仓");
                }
            }
            anyhow::ensure!(now_local_ms() - bar_close_ts_ms <= (self.max_signal_age_seconds as i64) * 1000,
                "信号已过期，提交前停止执行");
            Ok(())
        }.await;
        if let Err(e) = prepared {
            let res = ExecutionResult { submitted: false, signal_id: signal_id.clone(), request: request.clone(),
                response: None, reason: e.to_string(), error_code: String::new(), broker_tag: BROKER_TAG.into() };
            self.audit(&res, inst_id, timeframe, decision);
            return res;
        }

        // A timeout is not proof of rejection. Never retry the same signal blindly.
        self.seen.lock().insert(signal_id.clone());
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
                    error_code: "SUBMISSION_UNCONFIRMED".into(),
                    broker_tag: BROKER_TAG.to_string(),
                };
                self.audit(&res, inst_id, timeframe, decision);
                res
            }
        }
    }
}
