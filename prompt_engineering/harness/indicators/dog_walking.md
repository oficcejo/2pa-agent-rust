# 遛狗系统指标规格 (Dog Walking Indicators)

## 挂载指标
- **SMA14 (狗绳)**：简单移动平均线 (Period 14)，反映快速动量。
- **SMA170 (主人)**：简单移动平均线 (Period 170)，反映基准趋势中枢。
- **偏离度 (dev170_pct)**：价格相对于 SMA170 的百分比偏移量 `(Close - SMA170) / SMA170 * 100%`。
- **ATR14**：真实波动幅度均值 (Period 14)。
