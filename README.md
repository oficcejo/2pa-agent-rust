# OKX 2PA Agent

[![Rust](https://img.shields.io/badge/language-Rust-orange.svg)](https://www.rust-lang.org/)
[![Release](https://img.shields.io/badge/Release-v0.4.0-blue.svg)](https://github.com/oficcejo/2pa-agent-rust/releases/tag/v0.4.0)
[![License](https://img.shields.io/badge/license-AGPL--3.0-blue.svg)](LICENSE)

基于 Rust、Axum 和 Tokio 的 OKX 交易研究与执行工具，支持 LLM 两阶段分析、原生 AlphaPilot 因子、Web 控制台及自动交易时段。模型负责解释和提出方案，程序根据已收盘行情、结构、成本和账户额度决定是否允许执行。

当前版本 **v0.4.0**，策略协议 **2026-09-v1**。规则阈值是研究基线，测试通过不代表策略已经实现盈利。

## v0.4.0 更新与升级说明

- 拆分 `2pa_trend`、`dog_reversion`、`dog_trend`，分别记录策略归属和程序校验结果。
- 校验入场确认、高周期支持、结构止损及费用后净盈亏比；无有效信号时等待，不强制生成挂单。
- 移除无成交统计支持的胜率估计；只执行一个止盈目标，禁止逐棒推远止盈。
- 所有 Web 页面及 API 增加身份验证；修复风险定仓、最小手数上调、信号过期、重复执行、保护单修改和 AlphaPilot 历史因子中的相关问题。

旧配置 `2pa` 映射到 `2pa_trend`，`dog_walking` 映射到 `dog_reversion`；遛狗顺势策略须单独选择。`adaptive` 改为**仅观察、不开新仓**，有持仓仍可按共同规则管理。历史记录保持旧版标识，不将其收益归入新策略。

升级前停止旧进程，保留 `.env`、`config/` 和 `records/`。发布程序内嵌页面资源；若运行目录有旧 `static/` 文件，会优先读取这些文件，须同步更新该目录或将其移至备份目录，以使用内嵌新版页面。更新后自动交易默认关闭，保存配置也会关闭自动交易，需要在页面重新开启。

## 身份验证：用户名和密码

**用户名固定为 `admin`，没有固定默认密码。**

1. 配置了 `WEB_AUTH_TOKEN` 时，其值就是登录密码。
2. 未配置时，首次启动会生成随机密码，保存为**程序运行目录**下的 `config/.web-auth-token`，之后启动继续使用。
3. Web 配置向导也需要登录。线上服务须读取服务器或容器的密码文件，本机密码与线上密码互不通用。

查看自动生成的密码：

```powershell
# Windows：在程序运行目录执行
Get-Content -LiteralPath .\config\.web-auth-token
```

```bash
# Linux / macOS：在程序运行目录执行
cat config/.web-auth-token

# Docker Compose
docker compose exec okx-agent cat /app/config/.web-auth-token
```

如需重设密码，在 `.env` 中设置 `WEB_AUTH_TOKEN` 为新的随机口令（建议至少 24 位），重启服务后用新密码登录。环境变量优先于自动生成的文件。不要把密码、`.env` 或口令文件提交到 GitHub。远程访问请通过 HTTPS 反向代理。

API 支持 HTTP Basic 和 `Authorization: Bearer <WEB_AUTH_TOKEN>`。例如 `curl -u admin http://127.0.0.1:8088/api/status` 会提示输入密码。写请求须使用 JSON，不接受跨站写请求。

## 快速开始

### 下载运行

从 [Releases](https://github.com/oficcejo/2pa-agent-rust/releases) 下载产物。v0.4.0 提供 Windows x64 程序及 SHA-256 校验文件；Linux/macOS 可从源码编译。

在独立目录运行 `okx-2pa-agent.exe`，打开 <http://127.0.0.1:8088/>，取得密码并登录。在「系统配置」填写兼容 OpenAI 的模型接口和 OKX 凭据。首次缺少配置时程序会尝试打开浏览器；保存后写入运行目录 `.env` 并更新内存配置。

也可复制 [.env.example](.env.example) 为 `.env`，自行配置。以下示例没有可用凭据：

```ini
WEB_AUTH_TOKEN=
LLM_API_KEY=
LLM_BASE_URL=https://api.deepseek.com
LLM_MODEL=deepseek-v4-flash
LLM_THINKING=false
TRADING_SYSTEM=2pa_trend
OKX_API_KEY=
OKX_SECRET_KEY=
OKX_PASSPHRASE=
OKX_DEMO_TRADING=true
OKX_AUTO_TRADING_ENABLED=false
OKX_LIVE_TRADING_ACKNOWLEDGED=false
```

先在模拟盘验证。配置凭据不等于开启交易，仍须在 Web 自动交易面板启用；实盘还需要明确配置实盘确认条件。

### 从源码构建

本版本在 Rust/Cargo **1.91.1** 上通过 Windows 构建，不声明旧版 Rust 最低兼容性。

```bash
git clone https://github.com/oficcejo/2pa-agent-rust.git
cd 2pa-agent-rust
git checkout v0.4.0
cargo build --release --locked
```

Windows 产物为 `target/release/okx-2pa-agent.exe`，Linux/macOS 为 `target/release/okx-2pa-agent`。运行发布程序无需 Python 或 Node.js。默认监听 `127.0.0.1:8088`，可用 `--host`、`--port` 调整。

### Docker Compose

先将 `.env.example` 复制为 `.env`，确保 `.env` 是文件，并创建 `config`、`records` 目录：

```bash
docker compose up -d --build
docker compose logs -f
```

Compose 持久化 `/app/.env`、`/app/config`、`/app/records`，监听主机 8088 端口。保留 `config` 挂载才能保留自动生成的登录密码。v0.4.0 未单独验证 Linux/Docker 构建。

## 策略选择

| 策略 | 入场条件概要 | 目标 |
|---|---|---|
| `2pa_trend` | 本周期和高周期趋势同向，程序确认 H2/L2 二次入场或突破回踩 | 前方已确认支撑/阻力 |
| `dog_reversion` | 反向偏离至少 2.5 ATR 后，二次极值测试、推动减速并收回/跌破 SMA14，排除强逆向趋势 | SMA170 与前方结构中更近的一处 |
| `dog_trend` | SMA170 同向斜率、高周期同向、均线附近回踩及收盘确认 | 前方已确认支撑/阻力 |
| `adaptive` | 仅观察，不自动选择策略开新仓 | 不适用 |
| `alpha_pilot` | 原生因果滚动因子与 SuperTrend 引擎，无 LLM 调用 | 按原生引擎规则 |

机械 H2/L2 是人工形态的严格子集。大偏离仅代表观察机会，不能单独触发回归交易；遛狗顺势回踩不要求回归策略的偏离距离。

高周期映射：`1/3/5/15m → 1h`、`30m/1h → 4h`。三个新策略在没有有效结构化高周期数据时拒绝新开仓，其他周期当前会等待。建议先用单标的、15m/1h 分别研究各策略。

## 风险与执行规则

- 仅使用已收盘、连续且预热足够的行情；新开仓信号时效与已有挂单寿命分别检查。
- 入场距确认收盘不超过 0.25 ATR。止损覆盖结构失效点，外加至少 0.2 ATR 缓冲；总距离介于 `max(0.8 ATR, 入场价×0.3%)` 与 `3 ATR`。
- 净盈亏比至少 **1.5**。合约手续费基线每边 0.05%，现货每边 0.1%，另预留每边滑点 0.02%；尚未计入资金费预测。
- 下单前在最新市价及价格取整后复核，按风险额度与真实可用保证金定仓；不足最小手数时跳过，不上调仓位。
- 只执行 `take_profit_price` 一个目标，不分批止盈。止损只能收紧，保本须覆盖成本，止盈不得不断推远。
- 自动清理只处理本程序拥有的入场挂单。保护单须唯一且归属匹配，修改失败保留原保护，不撤单重建。
- 订单“已提交”不等于“已成交”，账户观测曲线不等于策略收益曲线。

不合格提案转为 WAIT/HOLD，原提案和理由存入 `rejected_proposal`、`program_validation`。新记录保存策略 ID、版本及证据；模型信心度不等于实测胜率。

完整规则见 [策略协议](prompt_engineering/strategy_v1.txt) 和 [改造说明](reports/2026-09-09-strategy-upgrade.md)。协议编译嵌入程序，修改后须重新构建。旧策略文档及经验库保留用于研究，不再加载到当前三个策略的提示词中。

## Web 与 API

Web 提供行情图表、策略分析、账户/持仓/委托、合约换算、交易时段及历史审计。遛狗策略显示 SMA14/SMA170，2PA 显示 EMA20。历史列表区分新旧策略，决策面板显示程序计算的净盈亏比。

所有路由要求身份验证。常用接口：

| 方法 | 路由 | 用途 |
|---|---|---|
| GET | `/api/status` | 状态、策略与自动交易配置 |
| POST | `/api/trading_system` | 切换策略 |
| POST | `/api/analyze` | 分析，可传 `trading_system`；执行另受开关和风控限制 |
| GET | `/api/account` | 账户、持仓和挂单 |
| GET / POST | `/api/config` / `/api/config/save_env` | 脱敏读取 / 保存配置 |
| POST | `/api/automation` | 配置自动交易 |
| GET | `/api/history/decisions` / `/api/history/trades` | 决策记录 / 交易审计 |

## 验证与项目结构

```bash
cargo test --all-targets --locked
node --check static/app.js
```

本地 **66 项测试通过**，覆盖三个策略多空正例、无确认拒绝、高周期校验、费用和结构约束、持仓管理、市价漂移、价格取整、两阶段模型接口模拟、风险定仓及执行保护。模型和交易接口测试使用本机模拟服务，没有完成样本外收益回测或真实成交滑点/资金费评估。

```text
src/strategies.rs                  策略证据、净成本、入场及管理校验
src/orchestrator/                  两阶段分析与 AlphaPilot 编排
src/okx/                          OKX 客户端、定仓和执行
src/web/                          认证、账户、持仓管理及 Web API
prompt_engineering/strategy_v1.txt 当前统一策略协议
static/                           Web 页面及图表
tests/                            单元与集成测试
config/                           运行配置及登录口令
records/                          决策与交易审计（不提交）
```

## 文档与许可

[使用文档](https://doc.zhongdu.net) · [Discord 社区](https://discord.gg/jk4mnW53gK)

本项目按 [GNU AGPL v3](LICENSE) 开源，包声明为 `AGPL-3.0-or-later`。软件用于策略研究，交易可能损失本金；应先验证模拟盘执行及独立策略的成交表现。
