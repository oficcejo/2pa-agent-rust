//! Versioned, deterministic entry rules. All thresholds are research baselines,
//! not fitted parameters or estimates of profitability.
use crate::data::base::{KlineFrame, PositionContext};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const VERSION: &str = "2026-09-v1";
pub const MIN_NET_RR: f64 = 1.5;
pub const SLIPPAGE_PER_SIDE: f64 = 0.0002;

pub fn canonical(id: &str) -> Option<&'static str> {
    match id.trim().to_lowercase().as_str() {
        "2pa_source" | "2pa_original" => Some("2pa_source"),
        "2pa" | "2pa_trend" => Some("2pa_trend"),
        "dog_walking" | "dog_reversion" | "遛狗" => Some("dog_reversion"),
        "dog_trend" => Some("dog_trend"),
        "adaptive" | "自适应" => Some("adaptive"),
        _ => None,
    }
}

pub fn cost_rate(symbol: &str) -> f64 {
    let derivative = symbol.ends_with("-SWAP") || symbol.split('-').count() >= 4;
    (if derivative { 0.0005 } else { 0.001 }) + SLIPPAGE_PER_SIDE
}

/// Cost includes entry and exit notional; use separate stop/target exit prices.
pub fn net_rr(symbol: &str, entry: f64, stop: f64, target: f64) -> f64 {
    let rate = cost_rate(symbol);
    ((target - entry).abs() - (entry + target) * rate)
        / ((entry - stop).abs() + (entry + stop) * rate)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub symbol: String,
    pub timeframe: String,
    pub strategy_id: String,
    pub strategy_version: String,
    pub direction: String,
    pub setup: String,
    pub signal_ts_ms: i64,
    pub atr: f64,
    pub reference_close: f64,
    pub invalidation: f64,
    pub target_bound: f64,
}

fn ready(f: &KlineFrame) -> bool {
    f.bars.len() >= 20
        && f.bars.iter().all(|b| {
            b.closed
                && b.seq > 0
                && [b.open, b.high, b.low, b.close]
                    .iter()
                    .all(|v| v.is_finite() && *v > 0.0)
                && b.low <= b.open.min(b.close)
                && b.high >= b.open.max(b.close)
        })
        && f.bars.windows(2).all(|b| b[0].ts_open > b[1].ts_open)
        && [
            &f.indicators.atr14,
            &f.indicators.ema20,
            &f.indicators.sma14,
            &f.indicators.sma170,
        ]
        .iter()
        .all(|v| v.len() >= 6 && v[..6].iter().all(|x| x.is_finite() && *x > 0.0))
}

fn ready_htf(f: &KlineFrame) -> bool {
    f.bars.len() >= 20
        && f.bars.iter().all(|b| {
            b.closed
                && b.seq > 0
                && [b.open, b.high, b.low, b.close]
                    .iter()
                    .all(|v| v.is_finite() && *v > 0.0)
                && b.low <= b.open.min(b.close)
                && b.high >= b.open.max(b.close)
        })
        && f.bars.windows(2).all(|b| b[0].ts_open > b[1].ts_open)
        && [
            &f.indicators.atr14,
            &f.indicators.ema20,
        ]
        .iter()
        .all(|v| v.len() >= 6 && v[..6].iter().all(|x| x.is_finite() && *x > 0.0))
}

fn trend(f: &KlineFrame, sign: f64) -> bool {
    let a = f.indicators.atr14[0];
    sign * (f.bars[0].close - f.indicators.ema20[0]) > 0.0
        && sign * (f.indicators.ema20[0] - f.indicators.ema20[5]) / a >= 0.1
}

fn trend_htf(h: &KlineFrame, sign: f64) -> bool {
    let a = h.indicators.atr14[0];
    // HTF price is not deeply counter-trend and HTF EMA20 is not sloping strongly against
    sign * (h.bars[0].close - h.indicators.ema20[0]) >= -0.6 * a
        && sign * (h.indicators.ema20[0] - h.indicators.ema20[4]) / a >= -0.05
}

/// Closed, confirmed pivots only; K1 and K2 cannot invent forward resistance.
fn obstacle(f: &KlineFrame, ref_price: f64, sign: f64, min_dist: f64) -> Option<f64> {
    (2..f.bars.len() - 1)
        .filter_map(|i| {
            let (b, newer, older) = (&f.bars[i], &f.bars[i - 1], &f.bars[i + 1]);
            let p = if sign > 0.0 && b.high > newer.high && b.high >= older.high {
                b.high
            } else if sign < 0.0 && b.low < newer.low && b.low <= older.low {
                b.low
            } else {
                return None;
            };
            (sign * (p - ref_price) >= min_dist).then_some(p)
        })
        .min_by(|a, b| ((a - ref_price).abs()).total_cmp(&(b - ref_price).abs()))
}

pub fn evidence(
    system: &str,
    f: &KlineFrame,
    htf: Option<&KlineFrame>,
    sign: f64,
) -> Result<Evidence> {
    let id = canonical(system).unwrap_or("");
    ensure!(
        ["2pa_source", "2pa_trend", "dog_reversion", "dog_trend"].contains(&id),
        "该模式仅供观察，不允许自动开仓"
    );
    ensure!(ready(f), "已收盘行情或指标预热不足");
    let step = timeframe_ms(&f.timeframe)?;
    ensure!(
        f.bars
            .windows(2)
            .all(|b| b[0].ts_open - b[1].ts_open == step),
        "行情存在缺失 K 线，不能跨缺口确认形态"
    );
    let h = htf
        .filter(|h| ready_htf(h) && h.symbol == f.symbol && h.timeframe != f.timeframe)
        .ok_or_else(|| anyhow::anyhow!("缺少有效的结构化高周期行情"))?;
    // The HTF snapshot must be contemporaneous with this analysis, never future data.
    let low_close = f.bars[0].ts_open + timeframe_ms(&f.timeframe)?;
    let high_close = h.bars[0].ts_open + timeframe_ms(&h.timeframe)?;
    ensure!(
        high_close <= low_close && low_close - high_close < timeframe_ms(&h.timeframe)?,
        "高周期行情过期或包含未来数据"
    );
    ensure!(
        timeframe_ms(&h.timeframe)? > timeframe_ms(&f.timeframe)?,
        "确认周期必须高于交易周期"
    );
    let b = &f.bars;
    let a = f.indicators.atr14[0];
    let close = b[0].close;
    let owner = f.indicators.sma170[0];
    let slope = (owner - f.indicators.sma170[5]) / (5.0 * a);
    let confirmation = sign * (b[0].close - b[0].open) > 0.0
        && if sign > 0.0 {
            close > b[1].high || (b[0].high > b[1].high && close >= b[1].close)
        } else {
            close < b[1].low || (b[0].low < b[1].low && close <= b[1].close)
        };
    ensure!(confirmation, "K1 尚未收盘突破前棒且形成同向实体");
    let setup;
    let mut bound = obstacle(f, close, sign, 1.8 * a).or_else(|| obstacle(h, close, sign, 1.8 * a));
    match id {
        "dog_reversion" => {
            let extended = (1..=12.min(b.len() - 1)).any(|i| {
                sign * (f.indicators.sma170[i] - b[i].close) / f.indicators.atr14[i] >= 1.6
                    || (sign * (f.indicators.sma170[i] - b[i].close) / f.indicators.sma170[i] >= 0.012)
            });
            ensure!(extended, "近十二根没有出现至少 1.6 ATR 或 1.2% 的反向偏离");
            let reclaimed_sma14 = sign * (close - f.indicators.sma14[0]) >= -0.2 * a
                && (0..=4.min(b.len() - 1)).any(|j| sign * (b[j].close - f.indicators.sma14[j]) > 0.0);
            ensure!(reclaimed_sma14, "尚未确认站上/跌破 SMA14 动量线");

            let recent_extreme = if sign > 0.0 {
                b[..=2.min(b.len() - 1)].iter().map(|x| x.low).fold(f64::INFINITY, f64::min)
            } else {
                b[..=2.min(b.len() - 1)].iter().map(|x| x.high).fold(f64::NEG_INFINITY, f64::max)
            };
            let prior_extreme = b[2..=10.min(b.len() - 1)]
                .iter()
                .map(|x| if sign > 0.0 { x.low } else { -x.high })
                .fold(f64::INFINITY, f64::min)
                * sign;
            ensure!(
                (recent_extreme - prior_extreme).abs() <= 1.2 * a,
                "缺少对称的二次极值测试"
            );
            ensure!(
                sign * slope >= -0.15,
                "主人均线 (SMA170) 斜率过度顺延，禁止逆势摸顶抄底"
            );
            ensure!(
                sign * (owner - close) >= 1.2 * a,
                "确认后已无充分回归 SMA170 的空间"
            );
            bound = Some(
                bound
                    .map(|p| if sign * (p - owner) < 0.0 { p } else { owner })
                    .unwrap_or(owner),
            );
            setup = "sma14_reclaim_second_test";
        }
        "dog_trend" => {
            ensure!(
                trend_htf(h, sign) && sign * slope >= -0.01,
                "SMA170 斜率或高周期趋势不支持方向"
            );
            let touched_sma170 = (1..=4.min(b.len() - 1)).any(|j| {
                (b[j].close - f.indicators.sma170[j]).abs() <= 0.8 * a
                    || (b[j].low <= f.indicators.sma170[j] + 0.4 * a && b[j].high >= f.indicators.sma170[j] - 0.4 * a)
            });
            ensure!(
                touched_sma170 && sign * (close - owner) > 0.0,
                "尚未确认 SMA170 附近回踩/反抽"
            );
            setup = "sma170_retest";
        }
        "2pa_source" => {
            ensure!(
                trend(f, sign) && !trend(h, -sign),
                "本级别趋势不明确或高周期强逆向"
            );
            let touched_ema = (1..=6.min(b.len() - 1)).any(|j| {
                if sign > 0.0 {
                    b[j].low <= f.indicators.ema20[j] + 0.8 * a
                } else {
                    b[j].high >= f.indicators.ema20[j] - 0.8 * a
                }
            });
            let has_two_attempts = (2..=7.min(b.len() - 2)).any(|j| {
                if sign > 0.0 {
                    b[j].low < b[j + 1].low && b[1].low <= b[j].low + 0.6 * a
                } else {
                    b[j].high > b[j + 1].high && b[1].high >= b[j].high - 0.6 * a
                }
            }) || (b[1].close - f.indicators.ema20[1]).abs() <= 1.2 * a;

            ensure!(touched_ema && has_two_attempts, "未识别到有效的 2PA 回踩结构");
            setup = "2pa_second_entry";
        }
        _ => {
            ensure!(
                trend(f, sign) && trend_htf(h, sign),
                "本级别与高周期趋势未同向确认"
            );
            let level = if sign > 0.0 {
                b[3..10]
                    .iter()
                    .map(|x| x.high)
                    .fold(f64::NEG_INFINITY, f64::max)
            } else {
                b[3..10].iter().map(|x| x.low).fold(f64::INFINITY, f64::min)
            };
            let breakout_retest = sign * (b[2].close - level) > 0.0
                && b[1].low <= level + 0.35 * a
                && b[1].high >= level - 0.35 * a
                && sign * (b[1].close - level) >= -0.35 * a
                && sign * (close - level) > 0.0;

            let second_5bar = if sign > 0.0 {
                b[4].high < b[5].high
                    && b[3].high > b[4].high
                    && b[2].high <= b[3].high
                    && b[1].low < b[2].low
            } else {
                b[4].low > b[5].low
                    && b[3].low < b[4].low
                    && b[2].low >= b[3].low
                    && b[1].high > b[2].high
            };

            let second_6bar = if sign > 0.0 {
                b[5].high < b[6].high
                    && b[4].high > b[5].high
                    && b[3].high <= b[4].high
                    && b[1].low < b[3].low
            } else {
                b[5].low > b[6].low
                    && b[4].low < b[5].low
                    && b[3].low >= b[4].low
                    && b[1].high > b[3].high
            };

            let second_4bar = if sign > 0.0 {
                b[3].high < b[4].high
                    && b[2].high > b[3].high
                    && b[2].high < b[4].high
                    && b[1].low < b[2].low
            } else {
                b[3].low > b[4].low
                    && b[2].low < b[3].low
                    && b[2].low > b[4].low
                    && b[1].high > b[2].high
            };

            let is_two_legged = (second_5bar || second_6bar || second_4bar)
                && (1..=5).any(|j| {
                    if sign > 0.0 {
                        b[j].low <= f.indicators.ema20[j] + 0.8 * a
                    } else {
                        b[j].high >= f.indicators.ema20[j] - 0.8 * a
                    }
                })
                && (b[1].close - f.indicators.ema20[1]).abs() <= 1.5 * a;

            ensure!(
                breakout_retest || is_two_legged,
                "没有确认的突破回踩或 H2/L2 二次入场"
            );
            setup = if breakout_retest {
                "breakout_retest"
            } else {
                "second_entry"
            };
        }
    }
    let structure = if id == "dog_reversion" {
        &b[..8]
    } else if id == "2pa_source" {
        &b[..4]
    } else {
        &b[..3]
    };
    let raw_invalidation = if sign > 0.0 {
        structure
            .iter()
            .map(|x| x.low)
            .fold(f64::INFINITY, f64::min)
    } else {
        structure
            .iter()
            .map(|x| x.high)
            .fold(f64::NEG_INFINITY, f64::max)
    };
    let invalidation = if id == "2pa_source" {
        if sign > 0.0 {
            raw_invalidation.max(close - 2.2 * a).min(close - 0.8 * a)
        } else {
            raw_invalidation.min(close + 2.2 * a).max(close + 0.8 * a)
        }
    } else if id == "dog_reversion" {
        if sign > 0.0 {
            raw_invalidation.max(close - 2.5 * a).min(close - 0.8 * a)
        } else {
            raw_invalidation.min(close + 2.5 * a).max(close + 0.8 * a)
        }
    } else if sign > 0.0 {
        raw_invalidation.max(close - 2.5 * a)
    } else {
        raw_invalidation.min(close + 2.5 * a)
    };
    let min_target_dist = (((close - invalidation).abs() + 0.25 * a) * 1.55).max(if id == "dog_reversion" { 1.2 * a } else { 2.5 * a });
    let bound = bound
        .filter(|p| sign * (*p - close) >= min_target_dist)
        .or_else(|| obstacle(f, close, sign, min_target_dist))
        .or_else(|| obstacle(h, close, sign, min_target_dist))
        .or_else(|| Some(close + sign * (min_target_dist + 0.5 * a)));
    let target_bound =
        bound.ok_or_else(|| anyhow::anyhow!("缺少前方已确认结构目标"))? - sign * 0.02 * a;
    Ok(Evidence {
        symbol: f.symbol.clone(),
        timeframe: f.timeframe.clone(),
        strategy_id: id.into(),
        strategy_version: VERSION.into(),
        direction: if sign > 0.0 { "做多" } else { "做空" }.into(),
        setup: setup.into(),
        signal_ts_ms: b[0].ts_open,
        atr: a,
        reference_close: close,
        invalidation,
        target_bound,
    })
}

fn timeframe_ms(tf: &str) -> Result<i64> {
    let s = tf.to_lowercase();
    let unit = if s.ends_with('m') {
        60_000
    } else if s.ends_with('h') {
        3_600_000
    } else if s.ends_with('d') {
        86_400_000
    } else {
        0
    };
    let n = s[..s.len().saturating_sub(1)].parse::<i64>().unwrap_or(0);
    ensure!(unit > 0 && n > 0 && n <= 1440, "不支持的行情周期");
    Ok(unit * n)
}

/// Reused after ticker refresh and tick rounding, before sizing/submission.
pub fn validate_entry(symbol: &str, d: &Value, e: &Evidence) -> Result<f64> {
    ensure!(symbol == e.symbol, "策略证据标的不一致");
    ensure!(
        e.strategy_version == VERSION && canonical(&e.strategy_id) == Some(e.strategy_id.as_str()),
        "策略证据版本无效"
    );
    let price = |key| {
        d[key]
            .as_f64()
            .filter(|p| p.is_finite() && *p > 0.0)
            .ok_or_else(|| anyhow::anyhow!("缺少有效价格: {key}"))
    };
    let entry = price("entry_price")?;
    let stop = price("stop_loss_price")?;
    let target = price("take_profit_price")?;
    ensure!(
        d["order_direction"].as_str() == Some(e.direction.as_str()),
        "方向与程序证据不一致"
    );
    let sign = if e.direction == "做多" { 1.0 } else { -1.0 };
    ensure!(
        e.atr.is_finite()
            && e.atr > 0.0
            && [e.reference_close, e.invalidation, e.target_bound]
                .iter()
                .all(|v| v.is_finite() && *v > 0.0),
        "策略证据价格无效"
    );
    ensure!(
        (entry - e.reference_close).abs() <= 0.25 * e.atr,
        "入场偏离确认收盘超过 0.25 ATR，等待新信号"
    );
    ensure!(
        sign * (entry - stop) >= (0.8 * e.atr).max(0.003 * entry)
            && (entry - stop).abs() <= 3.0 * e.atr,
        "止损距离不在波动率风险范围内"
    );
    ensure!(
        sign * (e.invalidation - stop) >= 0.2 * e.atr - 1e-9,
        "止损未覆盖结构失效点和 0.2 ATR 缓冲"
    );
    ensure!(
        sign * (target - entry) > 0.0 && sign * (e.target_bound - target) >= -1e-9,
        "目标越过前方结构或已无剩余空间"
    );
    let rr = net_rr(symbol, entry, stop, target);
    ensure!(
        rr >= MIN_NET_RR,
        "扣除手续费与滑点后的盈亏比 {:.3} 低于 {}",
        rr,
        MIN_NET_RR
    );
    Ok(rr)
}

pub fn diagnostics(system: &str, f: &KlineFrame, h: Option<&KlineFrame>) -> Value {
    let candidate = |sign| match evidence(system, f, h, sign) {
        Ok(e) => json!({"eligible":true,"evidence":e}),
        Err(err) => json!({"eligible":false,"reason":err.to_string()}),
    };
    json!({"strategy_version":VERSION,"long":candidate(1.0),"short":candidate(-1.0)})
}

pub fn enforce(
    system: &str,
    f: &KlineFrame,
    h: Option<&KlineFrame>,
    stage1: &Value,
    wrapper: &mut Value,
    pos: Option<&PositionContext>,
) {
    let proposed = wrapper["decision"].clone();
    let d = &mut wrapper["decision"];
    let id = canonical(system).unwrap_or("unknown");
    d["strategy_id"] = json!(id);
    d["strategy_version"] = json!(VERSION);
    d["estimated_win_rate"] = Value::Null;
    d["estimated_win_rate_reasoning"] = json!("未提供样本外成交统计，不能估计胜率");
    d.as_object_mut().map(|o| o.remove("strategy_evidence"));
    let opening = ["限价单", "突破单", "市价单"].contains(&d["order_type"].as_str().unwrap_or(""))
        || d["action"] == "OPEN";
    if !opening {
        d["risk_reward_ratio"] = Value::Null;
        d["traders_equation_passes"] = json!(false);
    }
    let result = if opening {
        (|| -> Result<()> {
            ensure!(proposed["action"] == "OPEN", "开仓订单类型与 action 冲突");
            ensure!(
                ["限价单", "突破单", "市价单"]
                    .contains(&proposed["order_type"].as_str().unwrap_or("")),
                "无效开仓订单类型"
            );
            ensure!(
                !pos.map(|p| p.has_position).unwrap_or(false),
                "已有持仓，禁止新开仓"
            );
            ensure!(stage1["gate_result"] == "proceed", "阶段一未放行");
            ensure!(
                wrapper_terminal_allows(&proposed, wrapper.get("terminal")),
                "模型决策与终止状态冲突"
            );
            let sign = match proposed["order_direction"].as_str() {
                Some("做多") => 1.0,
                Some("做空") => -1.0,
                _ => anyhow::bail!("无效交易方向"),
            };
            let e = evidence(system, f, h, sign)?;
            let rr = validate_entry(&f.symbol, &proposed, &e)?;
            let d = &mut wrapper["decision"];
            d["action"] = json!("OPEN");
            d["strategy_evidence"] = json!(e);
            d["risk_reward_ratio"] = json!(rr);
            d["traders_equation_passes"] = json!(true);
            d["take_profit_price_2"] = Value::Null;
            Ok(())
        })()
    } else {
        validate_management(&proposed, pos, &f.symbol)
    };
    match result {
        Ok(()) => {
            wrapper["program_validation"] =
                json!({"passed":true,"strategy_version":VERSION,"entry_checked":opening});
        }
        Err(err) => {
            wrapper["rejected_proposal"] = proposed;
            wrapper["decision"] = json!({"action":if pos.map(|p|p.has_position).unwrap_or(false){"HOLD"}else{"WAIT"},"order_type":"不下单","order_direction":null,"entry_price":null,"stop_loss_price":null,"take_profit_price":null,"trade_confidence":0,"estimated_win_rate":null,"traders_equation_passes":false,"reasoning":err.to_string(),"strategy_id":id,"strategy_version":VERSION});
            wrapper["terminal"] = json!({"outcome":"wait"});
            wrapper["program_validation"] =
                json!({"passed":false,"reason":err.to_string(),"strategy_version":VERSION});
        }
    }
}

fn wrapper_terminal_allows(d: &Value, terminal: Option<&Value>) -> bool {
    !matches!(
        terminal.and_then(|t| t["outcome"].as_str()),
        Some("wait" | "reject")
    ) && d["traders_equation_passes"] != false
}

pub fn validate_management(d: &Value, pos: Option<&PositionContext>, symbol: &str) -> Result<()> {
    let action = d["action"].as_str().unwrap_or("WAIT");
    if let Some(order) = d["order_type"].as_str() {
        let allowed: &[&str] = match action {
            "WAIT" => &["不下单"],
            "HOLD" => &["持有", "不下单"],
            "CLOSE_EARLY" => &["平仓"],
            "MOVE_STOP_LOSS" => &["修改止损"],
            "MOVE_TAKE_PROFIT" | "TRAILING_TAKE_PROFIT" => &["修改止盈"],
            "MOVE_SL_TP" => &["修改止盈止损"],
            _ => &[],
        };
        ensure!(allowed.contains(&order), "管理动作与订单类型冲突");
    }
    if ["WAIT", "HOLD"].contains(&action) {
        return Ok(());
    }
    let p = pos
        .filter(|p| p.has_position)
        .ok_or_else(|| anyhow::anyhow!("无持仓可管理"))?;
    ensure!(
        [
            "CLOSE_EARLY",
            "MOVE_STOP_LOSS",
            "MOVE_TAKE_PROFIT",
            "TRAILING_TAKE_PROFIT",
            "MOVE_SL_TP"
        ]
        .contains(&action),
        "不支持的管理动作"
    );
    let sign = if p.pos_side == "long" { 1.0 } else { -1.0 };
    if let Some(tp) = d["new_take_profit_price"]
        .as_f64()
        .or(d["take_profit_price"].as_f64())
    {
        if action.contains("TAKE_PROFIT") || action == "MOVE_SL_TP" {
            let old = p
                .current_tp
                .ok_or_else(|| anyhow::anyhow!("无法核实原止盈"))?;
            ensure!(
                tp.is_finite() && tp > 0.0 && sign * (tp - old) <= 0.0,
                "禁止逐棒把止盈推远"
            );
        }
    }
    if let Some(sl) = d["new_stop_loss_price"]
        .as_f64()
        .or(d["stop_loss_price"].as_f64())
    {
        if action == "MOVE_STOP_LOSS" || action == "MOVE_SL_TP" {
            let old = p
                .current_sl
                .ok_or_else(|| anyhow::anyhow!("无法核实原止损"))?;
            ensure!(
                sl.is_finite() && sl > 0.0 && sign * (sl - old) > 0.0,
                "止损只能收紧"
            );
            if let Some(entry) = p.open_avg_px {
                if sign * (sl - entry) >= 0.0 {
                    ensure!(
                        sign * (sl - entry) >= (entry + sl) * cost_rate(symbol),
                        "保本止损尚未覆盖手续费和滑点"
                    );
                }
            }
        }
    }
    Ok(())
}
