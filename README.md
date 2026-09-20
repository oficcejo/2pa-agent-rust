# OKX 2PA Agent

[![Rust](https://img.shields.io/badge/language-Rust-orange.svg)](https://www.rust-lang.org/)
[![Release](https://img.shields.io/badge/Release-v0.5.0-blue.svg)](https://github.com/oficcejo/2pa-agent-rust/releases/tag/v0.5.0)
[![License](https://img.shields.io/badge/license-AGPL--3.0-blue.svg)](LICENSE)
[![OKX 邀请注册](https://img.shields.io/badge/OKX-邀请注册-black.svg)](https://www.topzhjdgxcb.com/join/6746503)

基于 Rust、Axum 和 Tokio 的 OKX 交易研究与执行工具，支持 LLM 两阶段分析、Web 控制台、自动交易时段，以及默认关闭的自进化闭环。模型负责解释和提出方案，程序根据已收盘行情、结构、成本和账户额度决定是否允许执行。

当前版本 **v0.5.0**，策略协议 **2026-09-v1**。规则阈值是研究基线，测试通过不代表策略已经实现盈利。

## 自进化

程序会观察自己每一笔已提交订单的**真实结局**，把结果沉淀为结构化经验，并让策略提示词的改动**先证明不劣于当前版本**才能生效。整条链路可审计、可回滚，**默认关闭**。

- **可归因** —— 每笔结果都能追回到哪次决策、哪个提示词版本、哪个市场形态；`signal_id` 既是去重键也是回执键。
- **可验证** —— 改提示词不再凭感觉，而是比较各版本在**真实成交结果**上的期望 R、胜率与盈亏比。
- **可回滚** —— 提示词是带版本的 artifact，发布与激活分离，可一键回退。
- **有边界** —— 不训练权重、不自动热切换上线、不把估算盈亏当成对账数据。

配置项、资格判定与操作步骤见下文《自进化（持续学习闭环）》。

## v0.5.0 更新与升级说明

- 新增**自进化闭环**（`src/learning/`）：以 `signal_id` 为回执对账交易所成交结果，生成结构化反馈（R 倍数、MFE/MAE、持仓 K 线数、费用、出场原因），并自动写入经验库。
- 策略提示词改为**带版本的 artifact**（`prompt_engineering/artifacts/`）：发布与激活分离、支持回滚，改阈值不再需要重新编译，每条决策可追溯到确切的提示词版本与哈希。
- 新增**选择性发布校验**：候选提示词版本须在真实成交结果上不劣于当前版本才可激活；样本不足时明确拒绝结论，不给乐观默认值。
- 修复**经验库读取链路**：此前 `ExperienceReader` 从未被调用，经验库实际处于失效状态；现已接入阶段二提示词装配。
- 修复**品种下拉菜单**：SPOT 与 SWAP 请求相互隔离并各带一次重试，单个市场偶发失败不再清空整个列表；同时修复状态接口失败导致启动链中断的问题。
- 新增「🧠 持续学习」Web 面板与 7 个学习接口；测试由 66 项增加到 128 项。

**升级要点**：自进化闭环**默认关闭**，`LEARNING_ENABLED=false` 时行为与 v0.4.0 一致。首次运行会在 `prompt_engineering/artifacts/` 播种提示词 `v1`（该目录已被 Git 忽略，属运行态）。旧 `experience/` 经验库默认仍不注入提示词。

## v0.4.0 更新与升级说明

- 拆分 `2pa_trend`、`dog_reversion`、`dog_trend`，分别记录策略归属和程序校验结果。
- 校验入场确认、高周期支持、结构止损及费用后净盈亏比；无有效信号时等待，不强制生成挂单。
- 移除无成交统计支持的胜率估计；只执行一个止盈目标，禁止逐棒推远止盈。
- 所有 Web 页面及 API 增加身份验证；修复风险定仓、最小手数上调、信号过期、重复执行和保护单修改中的相关问题。

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

从 [Releases](https://github.com/oficcejo/2pa-agent-rust/releases) 下载产物。v0.5.0 提供 Windows x64 程序及 SHA-256 校验文件；Linux/macOS 可从源码编译。

在独立目录运行 `okx-2pa-agent.exe`，打开 <http://127.0.0.1:8088/>，取得密码并登录。在「系统配置」填写兼容 OpenAI 的模型接口和 OKX 凭据（尚未拥有 OKX 账户的用户，可通过 [OKX 专属邀请注册链接](https://www.topzhjdgxcb.com/join/6746503) 开户并获取 API 凭证）。首次缺少配置时程序会尝试打开浏览器；保存后写入运行目录 `.env` 并更新内存配置。

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
LEARNING_ENABLED=false
LEARNING_READ_EXPERIENCE=false
LEARNING_WRITE_EXPERIENCE=true
```

先在模拟盘验证。配置凭据不等于开启交易，仍须在 Web 自动交易面板启用；实盘还需要明确配置实盘确认条件。

### 从源码构建

本版本在 Rust/Cargo **1.91.1** 上通过 Windows 构建，不声明旧版 Rust 最低兼容性。

```bash
git clone https://github.com/oficcejo/2pa-agent-rust.git
cd 2pa-agent-rust
git checkout v0.5.0
cargo build --release --locked
```

Windows 产物为 `target/release/okx-2pa-agent.exe`，Linux/macOS 为 `target/release/okx-2pa-agent`。运行发布程序无需 Python 或 Node.js。默认监听 `127.0.0.1:8088`，可用 `--host`、`--port` 调整。

### Docker Compose

先将 `.env.example` 复制为 `.env`，确保 `.env` 是文件，并创建 `config`、`records` 目录：

```bash
docker compose up -d --build
docker compose logs -f
```

Compose 持久化 `/app/.env`、`/app/config`、`/app/records`，监听主机 8088 端口。保留 `config` 挂载才能保留自动生成的登录密码。Linux/Docker 构建未单独验证。

## 策略选择

| 策略 | 入场条件概要 | 目标 |
|---|---|---|
| `2pa_trend` | 本周期和高周期趋势同向，程序确认 H2/L2 二次入场或突破回踩 | 前方已确认支撑/阻力 |
| `dog_reversion` | 反向偏离至少 2.5 ATR 后，二次极值测试、推动减速并收回/跌破 SMA14，排除强逆向趋势 | SMA170 与前方结构中更近的一处 |
| `dog_trend` | SMA170 同向斜率、高周期同向、均线附近回踩及收盘确认 | 前方已确认支撑/阻力 |
| `adaptive` | 仅观察，不自动选择策略开新仓 | 不适用 |

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

完整规则见 [策略协议](prompt_engineering/strategy_v1.txt) 和 [改造说明](reports/2026-09-09-strategy-upgrade.md)。协议以带版本的 artifact 管理（`prompt_engineering/artifacts/strategy_v1/`），首次运行时自动以内置副本播种 `v1`；内置副本仅作回退，运行中读取 artifact。旧策略文档及经验库保留用于研究，**默认不加载**到当前三个策略的提示词中。

## 自进化（持续学习闭环）

借鉴 [reef](https://github.com/Human-Agent-Society/reef) 的 Serve → Observe → Grow → Commit 四步循环，把「决策 → 成交 → 结果 → 经验 → 版本」接成闭环：

| 环节 | 实现 |
|---|---|
| Serve | 已有：分析产生决策，执行器提交订单并写入 `records/trade_audit.jsonl` |
| Observe | `src/learning/`：以 `signal_id` 为回执，对账交易所成交状态，生成结构化结果（R 倍数、MFE/MAE、持仓 K 线数、费用、出场原因），存入 `records/outcomes/` |
| Grow | 合格结果自动写入经验库 `experience/<周期>/{success,failure}_cases/`，取代手工维护的用例文件 |
| Commit | 策略提示词版本化为 artifact；候选版本须先发布、再经选择性发布校验后才可显式激活 |

**默认关闭**。`LEARNING_ENABLED=false` 时不对账、不写经验；`LEARNING_READ_EXPERIENCE=false` 时经验库不会注入提示词——开启对账不会悄悄改变模型看到的内容。

```ini
LEARNING_ENABLED=false                 # 总开关：交易结果对账 + 经验入库
LEARNING_READ_EXPERIENCE=false         # 是否把经验库注入阶段二提示词
LEARNING_WRITE_EXPERIENCE=true         # 是否把合格结果写入经验库
LEARNING_RECONCILE_INTERVAL_SECONDS=60
LEARNING_MAX_HOLD_BARS=96              # 超过该 K 线数仍未触发的交易按到期结算
```

资格判定（默认）：必须真实成交、初始风险大于零、持仓不少于 1 根 K 线、|R| 不超过 25。未成交订单仍会留存结果，但不会进经验库。

### 如何开启与操作

1. 设 `LEARNING_ENABLED=true` 并重启。**先保持 `LEARNING_READ_EXPERIENCE=false`**，只做观察，不改变模型输入。
2. 在模拟盘正常交易。每笔成交后程序按 `LEARNING_RECONCILE_INTERVAL_SECONDS` 自动对账；也可在「🧠 持续学习」面板点「立即对账」。
3. 检查面板：「当前策略提示词版本」应显示 `v1` / 来源「版本化 artifact」；「最近结算结果」应显示 R 倍数、MFE/MAE、持仓根数，并标注「盈亏=模型估算」。
4. 合格样本会出现在「经验库」计数中，同时落盘到 `experience/<周期>/success_cases/`（或 `failure_cases/`）。
5. 要改策略：把新的提示词**全文** POST 到 `/api/learning/prompts/publish` 发布为候选（此时不生效）→ 累积该版本的真实结果 → 在面板点「启用」。未通过选择性发布校验会被拒绝并说明理由；确认无误可用 `force` 跳过统计校验。
6. 确认经验注入确有帮助后，再把 `LEARNING_READ_EXPERIENCE=true` 打开。

面板只提供对账、启用与回退按钮；发布候选需调用 API。`force` 的含义与「盈亏=模型估算」的标注都直接显示在界面上，不藏在文档里。

### 明确的边界

- **不做在线权重训练。** 本项目是执行端，不是训练框架。
- **不允许策略自动热切换。** 候选版本发布后必须显式激活，激活前可依据历史结果做选择性发布校验；程序不会自行把新策略投入实盘。
- `realized_pnl_usd` 标注 `pnl_source: model_estimate` 时表示由价格与仓位推导，**不是交易所对账数据**；R 倍数由价格推导，是主要学习信号。
- 期望 R、胜率等指标只在合格样本上统计，样本不足时明确拒绝结论，不给出乐观默认值。

### 为什么这样设计

- **执行端不承担训练职责**：引入训练栈会同时引入依赖、算力与不可解释性，而本项目的价值在于确定性的执行与风控。
- **持仓期间静默变更策略是风险，不是特性**：因此「发布」与「激活」拆成两个动作，激活永远由人发起。
- **宁可回答「样本不足：2 / 20，无法比较」**，也不给乐观默认值——这与「测试通过不代表已盈利」是同一条原则。
- **估算与对账必须分离**：估算盈亏可用于相对比较，但不能被当作可对外宣称的收益。

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
| GET | `/api/learning/report` | 学习闭环状态、指标与提示词版本对比 |
| POST | `/api/learning/reconcile` | 立即对账一次交易结果 |
| GET | `/api/learning/outcomes` | 已结算结果列表 |
| GET | `/api/learning/prompts` | 提示词版本列表 |
| POST | `/api/learning/prompts/publish` | 发布候选提示词（不激活） |
| POST | `/api/learning/prompts/activate` | 激活提示词版本（受发布校验约束） |
| POST | `/api/learning/prompts/rollback` | 回退到上一提示词版本 |

## 验证与项目结构

```bash
cargo test --all-targets --locked
node --check static/app.js
```

本地 **128 项测试通过**，覆盖三个策略多空正例、无确认拒绝、高周期校验、费用和结构约束、持仓管理、市价漂移、价格取整、两阶段模型接口模拟、风险定仓及执行保护，以及学习闭环的回执对账、R/MFE/MAE 计算、资格判定、经验库读写、提示词版本发布校验和鉴权路由。模型和交易接口测试使用本机模拟服务，没有完成样本外收益回测或真实成交滑点/资金费评估。

```text
src/strategies.rs                  策略证据、净成本、入场及管理校验
src/orchestrator/                  两阶段分析与策略编排
src/learning/                      回执对账、结构化反馈、经验库、提示词版本化
src/okx/                          OKX 客户端、定仓和执行
src/web/                          认证、账户、持仓管理及 Web API
prompt_engineering/strategy_v1.txt 当前统一策略协议
static/                           Web 页面及图表
tests/                            单元与集成测试
config/                           运行配置及登录口令
records/                          决策与交易审计（不提交）
```

## 文档与许可

[使用文档](https://doc.zhongdu.net) · [Discord 社区](https://discord.gg/jk4mnW53gK) · [OKX 邀请注册](https://www.topzhjdgxcb.com/join/6746503)

本项目按 [GNU AGPL v3](LICENSE) 开源，包声明为 `AGPL-3.0-or-later`。软件用于策略研究，交易可能损失本金；应先验证模拟盘执行及独立策略的成交表现。
