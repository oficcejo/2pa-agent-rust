# 两阶段执行契约 (Execution Contract)

## 两阶段分工
- **阶段一 (市场诊断)**：输出周期形态、主导力量、关键点位、检测形态和阶段一闸门结果（proceed / wait / unknown）。严禁在此阶段决定具体买卖价格。
- **阶段二 (交易决策)**：基于阶段一诊断和账户持仓状态，输出交易决策 JSON：
  - `action`: `OPEN` / `WAIT` / `HOLD` / `MOVE_STOP_LOSS` / `CLOSE_EARLY`
  - `order_direction`: `做多` / `做空` (空仓时)
  - `order_type`: `限价单` / `市价单`
  - `entry_price`, `stop_loss_price`, `take_profit_price`
  - `confidence_score`: 决策置信度 (0.0 ~ 1.0)
  - `estimated_win_rate`: estimated_win_rate 必须 null，trade_confidence 仅是主观信号评级，不能称为胜率。禁止给出无统计支持的百分比或宣称策略必然盈利。
