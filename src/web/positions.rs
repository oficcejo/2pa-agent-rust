use crate::{
    data::base::PositionContext,
    okx::{
        client::OKXClient,
        trading::{BROKER_TAG, PA_CLIENT_ORDER_PREFIX},
    },
};
use anyhow::{anyhow, ensure, Result};
use serde_json::{json, Value};

pub fn number(row: &Value, key: &str) -> Option<f64> {
    row.get(key)
        .and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse().ok()))
        .filter(|v| v.is_finite())
}

pub fn position_side(row: &Value) -> Result<&'static str> {
    let size = number(row, "pos").ok_or_else(|| anyhow!("持仓数量无效，拒绝推断仓位"))?;
    Ok(
        match row.get("posSide").and_then(Value::as_str).unwrap_or("net") {
            "long" => "long",
            "short" => "short",
            "net" | "" => {
                if size < 0.0 {
                    "short"
                } else {
                    "long"
                }
            }
            _ => return Err(anyhow!("未知的持仓方向")),
        },
    )
}

pub fn owned_order(row: &Value) -> bool {
    row.get("tag").and_then(Value::as_str) == Some(BROKER_TAG)
        || ["clOrdId", "algoClOrdId", "attachAlgoClOrdId"]
            .iter()
            .any(|key| {
                row.get(key)
                    .and_then(Value::as_str)
                    .map(|v| v.starts_with(PA_CLIENT_ORDER_PREFIX))
                    .unwrap_or(false)
            })
}

pub fn protection_matches(row: &Value, pos: &PositionContext, mode: &str) -> bool {
    owned_order(row)
        && row["instId"].as_str() == Some(pos.symbol.as_str())
        && row["side"].as_str()
            == Some(if pos.pos_side == "long" {
                "sell"
            } else {
                "buy"
            })
        && (mode != "long_short" || row["posSide"].as_str() == Some(pos.pos_side.as_str()))
        && row
            .get("tdMode")
            .and_then(Value::as_str)
            .map(|v| v == pos.mgn_mode)
            .unwrap_or(true)
}

pub async fn read_position(
    client: &OKXClient,
    symbol: &str,
    mode: &str,
) -> Result<PositionContext> {
    let rows = client.get_positions(Some(symbol)).await?;
    let mut active = Vec::new();
    for row in rows {
        let size = number(&row, "pos").ok_or_else(|| anyhow!("无法读取持仓数量"))?;
        if size != 0.0 {
            active.push(row);
        }
    }
    ensure!(
        active.len() <= 1,
        "同品种存在多个持仓方向，暂停自动管理以免操作错误仓位"
    );
    let mut pos = PositionContext {
        symbol: symbol.to_string(),
        pos_side: "none".into(),
        pos_size: "0".into(),
        ..Default::default()
    };
    if let Some(row) = active.first() {
        pos.has_position = true;
        pos.pos_side = position_side(row)?.into();
        pos.pos_size = number(row, "pos").unwrap().abs().to_string();
        pos.open_avg_px = number(row, "avgPx");
        pos.mark_px = number(row, "markPx");
        pos.unrealized_pnl = number(row, "upl");
        pos.unrealized_pnl_ratio = number(row, "uplRatio").map(|r| r * 100.0);
        pos.leverage = number(row, "lever");
        pos.mgn_mode = row["mgnMode"]
            .as_str()
            .ok_or_else(|| anyhow!("持仓保证金模式缺失"))?
            .into();
        pos.open_time_ms = row["cTime"].as_str().and_then(|v| v.parse().ok());
        let mut algos = client
            .get_pending_algo_orders(Some(symbol), "conditional")
            .await?;
        algos.extend(client.get_pending_algo_orders(Some(symbol), "oco").await?);
        let matching: Vec<_> = algos
            .iter()
            .filter(|a| protection_matches(a, &pos, mode))
            .collect();
        // Never guess which order to amend when multiple protections exist.
        if matching.len() == 1 {
            let a = matching[0];
            pos.current_sl = number(a, "slTriggerPx").filter(|p| *p > 0.0);
            pos.current_tp = number(a, "tpTriggerPx").filter(|p| *p > 0.0);
            pos.algo_id = a["algoId"].as_str().map(str::to_string);
        }
    }
    Ok(pos)
}

pub fn validate_stop_change(pos: &PositionContext, stop: f64) -> Result<()> {
    ensure!(stop.is_finite() && stop > 0.0, "止损必须为正数");
    let old = pos
        .current_sl
        .ok_or_else(|| anyhow!("当前止损未知，拒绝自动修改"))?;
    let mark = pos
        .mark_px
        .filter(|p| *p > 0.0)
        .ok_or_else(|| anyhow!("当前标记价格未知"))?;
    if pos.pos_side == "long" {
        ensure!(
            stop >= old && stop < mark,
            "多仓止损只能收紧，且必须低于当前价格"
        );
    } else {
        ensure!(
            stop <= old && stop > mark,
            "空仓止损只能收紧，且必须高于当前价格"
        );
    }
    Ok(())
}

pub async fn execute_management(
    client: &OKXClient,
    symbol: &str,
    mode: &str,
    decision: &Value,
) -> Result<Value> {
    let pos = read_position(client, symbol, mode).await?;
    ensure!(pos.has_position, "当前无持仓，不能执行持仓管理");
    let action = decision["action"].as_str().unwrap_or("");
    let order_type = decision["order_type"].as_str().unwrap_or("");
    if action == "CLOSE_EARLY" || order_type == "平仓" {
        return client
            .close_position(
                symbol,
                &pos.mgn_mode,
                if mode == "long_short" {
                    Some(&pos.pos_side)
                } else {
                    None
                },
            )
            .await;
    }
    let algo_id = pos
        .algo_id
        .as_deref()
        .ok_or_else(|| anyhow!("未找到唯一的本策略保护单；保留现有委托，拒绝自动撤换"))?;
    let combined = action == "MOVE_SL_TP" || order_type == "修改止盈止损";
    let change_sl = combined || action == "MOVE_STOP_LOSS" || order_type == "修改止损";
    let change_tp = combined
        || ["MOVE_TAKE_PROFIT", "TRAILING_TAKE_PROFIT"].contains(&action)
        || order_type == "修改止盈";
    let inst_type = if symbol.ends_with("-SWAP") {
        "SWAP"
    } else if symbol.split('-').count() >= 4 {
        "FUTURES"
    } else {
        "SPOT"
    };
    let instruments = client.get_instruments(inst_type, Some(symbol)).await?;
    let tick = instruments
        .first()
        .and_then(|v| number(v, "tickSz"))
        .filter(|v| *v > 0.0)
        .ok_or_else(|| anyhow!("无法读取价格精度"))?;
    let rounded = |v: f64| (v / tick).round() * tick;
    let sl = if change_sl {
        number(decision, "new_stop_loss_price")
            .or_else(|| number(decision, "stop_loss_price"))
            .map(rounded)
    } else {
        None
    };
    let tp = if change_tp {
        number(decision, "new_take_profit_price")
            .or_else(|| number(decision, "take_profit_price"))
            .map(rounded)
    } else {
        None
    };
    ensure!(sl.is_some() || tp.is_some(), "缺少有效的新止损/止盈价格");
    if decision["strategy_version"] == crate::strategies::VERSION {
        let mut checked = decision.clone();
        checked["new_stop_loss_price"] = serde_json::json!(sl);
        checked["new_take_profit_price"] = serde_json::json!(tp);
        crate::strategies::validate_management(&checked, Some(&pos), symbol)?;
    }
    if let Some(stop) = sl {
        validate_stop_change(&pos, stop)?;
    }
    if let Some(target) = tp {
        let mark = pos
            .mark_px
            .filter(|p| *p > 0.0)
            .ok_or_else(|| anyhow!("当前价格未知"))?;
        ensure!(
            target > 0.0
                && if pos.pos_side == "long" {
                    target > mark
                } else {
                    target < mark
                },
            "止盈价格位于错误方向"
        );
    }
    // An amend failure leaves the old SL/TP intact. Do not cancel/recreate.
    client.amend_algo_order(symbol, algo_id, sl, tp).await
}

pub fn normalize_position(row: &Value) -> Result<Value> {
    Ok(json!({
        "instrument": row["instId"], "direction": position_side(row)?,
        "size": number(row,"pos").map(f64::abs), "contract_size": number(row,"pos").map(f64::abs),
        "average_price": number(row,"avgPx"), "mark_price": number(row,"markPx"),
        "unrealized_pnl": number(row,"upl"), "unrealized_pnl_ratio": number(row,"uplRatio"),
        "leverage": number(row,"lever"), "margin_mode": row["mgnMode"]
    }))
}
