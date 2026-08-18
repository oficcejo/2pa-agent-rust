# OKX 2PA Agent (Rust 高性能版)

[![Rust](https://img.shields.io/badge/language-Rust%201.75+-orange.svg)](https://www.rust-lang.org/)
[![Release](https://img.shields.io/badge/Release-v0.3.0-blue.svg)](https://github.com/oficcejo/2pa-agent-rust/releases/tag/v0.3.0)
[![Docker](https://img.shields.io/badge/Docker-Ready-2496ED?logo=docker&logoColor=white)](https://www.docker.com/)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Discord](https://img.shields.io/badge/Discord-Join%20Community-5865F2?logo=discord&logoColor=white)](https://discord.gg/jk4mnW53gK)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20Linux%20%7C%20macOS%20%7C%20Docker-blue.svg)]()

**OKX 2PA Agent** 是一款基于 **Al Brooks 价格行为学（Price Action）** 与 **均线偏离回归动力学（🐕 遛狗系统）**、结合 **大语言模型（LLM）两阶段智能推理** 的高频/日内量化交易系统。

本项目采用 **纯 Rust** 进行重构与极致性能优化，支持编译为**单文件独立可执行程序**（Windows `.exe` / Linux 二进制，右侧 Releases 可直接下载运行）或通过 **Docker / Docker Compose** 容器化一键部署，内嵌 Web 控制台与全部交易策略 Prompt，实现零外部运行时依赖（无需 Python、Node.js 等环境）即开即用。

🎮 **Discord 交流群**：[https://discord.gg/jk4mnW53gK](https://discord.gg/jk4mnW53gK)

---

## 🌟 核心特性与功能亮点

### 1. 🤖 双 AI 交易系统自由切换 (Dual Trading Systems)
系统原生集成两套经实战打磨的独立交易决策系统，支持在 Web 界面一键无缝切换：

- 📊 **2PA 价格行为系统 (Al Brooks 经典价格行为学)**：
  - **经典八态周期**：Spike、Tight Channel、Broad Channel、Trading Range 等状态精确诊断。
  - **几何形态引擎**：原生识别 EMA20 支撑阻力、二次入场 (H2/L2)、楔形反转 (Wedge)、突破测试 (Breakout Test)、跳空缺口 (Gap Bar) 等。
  - **二元决策树**：严格的 Gate 闸门检验与胜率/盈亏比交易者方程计算。
- 🐕 **遛狗系统 (SMA 14/170 均线偏离与回归系统)**：
  - **动力学物理隐喻**：将 **170 SMA（蓝色）** 视为缓步前行的「主人 / 长期价值重心中枢」，将 **14 SMA（橙色）** 视为敏捷波动的「狗绳 / 短期趋势均线」，将 **K 线** 视为欢脱奔跑的「小狗」。
  - **偏离极限衰竭回归（Mean-Reversion）**：当小狗过度远离主人（偏离度 Dev% 达到极值）且动能耗尽拐头（破 14 均线 / 衰竭 Pinbar / 吞没）时，绳索拉力迫使其发生向主人均线的快速回弹，**止盈目标 TP2 强制锚定 SMA 170 均线**。
  - **170 主人均线回踩顺势（Trend-Continuation）**：在平缓趋势中回踩 170 主人均线获得强支撑/阻力并收出反转信号时顺势入场。
  - **动态图表与图例**：选择遛狗系统时，K 线图表自动绘制 **SMA 14（橙色狗绳）** 与 **SMA 170（蓝色主人中枢）**，并实时呈现双均线数值图例。

---

### 2. ⚡ 「撤旧换新 (Cancel-Replace)」防堆积挂单机制
- **自动清理旧挂单**：新委托发出前，系统自动识别并撤销同品种此前未成交的旧限价挂单与旧突破挂单。
- **杜绝挂单无限叠加**：保证 OKX 交易所账户中同品种永远只保留最新、最精准的有效入场委托，彻底告别历史挂单堆积问题。
- **Web 挂单管理与一键全撤**：账户面板清晰呈现挂单明细，支持单笔撤单与「一键全撤」快速清空所有冗余挂单。

---

### 3. 纯 Rust 原生极速架构与单文件分发
- **极致性能**：毫秒级响应、超低内存占用（< 20MB），基于 Tokio + Axum 高并发异步运行时。
- **单文件独立发布（Zero Dependencies）**：通过 `rust-embed` 将 Web 控制台静态资源与 Prompt 工程规则库完全编译进二进制产物，单文件即可在任何 Windows/Linux 环境独立运行。
- **本地系统时间日志**：控制台日志自动根据服务器/本机时区输出易读的本地时间。

---

### 4. 智能环境配置向导 (Web Setup Wizard)
- **首次运行自动检测与浏览器唤起**：双击启动 `okx-2pa-agent.exe` 时，若未检测到 `.env` 或 API 密钥未配置，控制台输出提示并**自动调用系统默认浏览器**打开配置向导。
- **一键持久化至 `.env`**：在 Web 界面中填入大模型 API Key（默认模型 `deepseek-v4-flash`、默认接口 `https://api.deepseek.com`、默认关闭思考模式）和 OKX 凭证后，点击保存将**自动写入程序根目录 `.env`**，并在内存中即时热加载生效，下次直接启动无需重复配置。
- **顶部配置按钮**：Web 顶部栏提供「⚙️ 系统配置」入口，随时可点击修改密钥并即时生效；内置「🔗 注册okx用户」快捷开户入口。

---

### 5. 📐 合约规格与张数双向换算器 (Contract Calculator)
针对 OKX 永续合约按「张数」下单难以直观把握资金价值的问题，内置专属换算工作台：
- **常见热门品种 1 张面值速查表**：
  - 覆盖加密主流（BTC 1张=0.01BTC、ETH 1张=0.1ETH、SOL 1张=1SOL、DOGE 1张=1000DOGE 等）、贵金属（XAU 黄金、XAG 白银）及美股 TradFi（AAPL、TSLA、NVDA、SPX）。
  - 实时展示 **1 张合约面值**、**实时最新市价**、**1 张折合 USDT 价值**及最小下单张数。
- **支持实时查询任意合约品种**：顶部搜索框输入任意 OKX 合约（如 `PEPE-USDT-SWAP`），即时拉取其面值并计算。
- **双向智能下单换算**：
  - **💰 模式一（按投入 USDT 算张数）**：输入目标 USDT 仓位价值 + 杠杆倍数 $\rightarrow$ 自动计算**建议下单张数**、实际交易金额、所需保证金及标的币数。
  - **📊 模式二（按张数算所需 USDT）**：输入计划下单张数 + 杠杆倍数 $\rightarrow$ 自动计算**仓位总价值 (USDT)**、占用保证金及标的代币总量。

---

### 6. 🛡️ 结构化止盈止损策略与交易所原子挂单
- **保护性止损（Stop Loss）**：
  - 做多：信号棒低点 − 1 Tick；做空：信号棒高点 + 1 Tick。
  - 动态风控：止损距离 $> 2 \times \text{ATR14}$ 或超过通道宽度的 50% 时判定风险过大直接放弃。
- **分级止盈（Take Profit）**：
  - **TP1（主目标 / 保守止盈）**：最近结构边界或 14 均线，严格遵循**交易者方程**与**盈亏比 $\text{RR} \ge 1.0$**。
  - **TP2（延伸目标）**：遛狗系统强制锚定 SMA 170 主人均线；2PA 系统锚定等距测量移动（Measured Move）。
- **OKX 原子性 `attachAlgoOrds` 挂单**：
  - 下单时同时附带 `slTriggerPx` 与 `tpTriggerPx`，主订单成交后交易所端自动激活双向条件平仓单，本地无需保持开机。

---

### 7. 🤖 自动交易调度与多时段管理
- **单排等宽导航栏**：【决策】 | 【账户】 | 【📐 合约换算】 | 【🤖 自动交易】，专属翠绿发光高亮。
- **系统状态与本地记忆联动**：切换交易系统即时持久化到本地 `localStorage`，并在后台与监控面板联动展示。
- **多时区时段过滤器**：支持全天候（always）、美股常规盘（us_regular）、美股开盘窗口（us_open）、伦敦时段（london）、亚洲时段（asia）与自定义时区时段。

---

## 📂 项目目录结构

```text
2pa-agent-rust/
├── .cargo/                 # Cargo 编译与高速镜像源配置
├── config/                 # 运行时配置目录
│   └── settings.json       # 系统参数配置文件
├── experience/             # 历史价格行为经验库
├── prompt_engineering/     # 提示词工程规则库
│   ├── 00_人设与思维方式.txt
│   ├── 01_市场诊断框架.txt
│   ├── 02_交易决策策略.txt
│   ├── 遛狗系统_人设与思维方式.txt  # 🐕 遛狗系统人设与动力学公理
│   ├── 遛狗系统_市场诊断框架.txt    # 🐕 遛狗系统阶段一诊断框架
│   └── 遛狗系统_交易决策策略.txt    # 🐕 遛狗系统阶段二决策策略
├── records/                # 运行时持久化记录与交易审计日志
│   ├── pending/            # 待跟踪决策记录
│   └── trade_audit.jsonl   # 交易执行审计流水
├── src/                    # Rust 核心源代码
│   ├── ai/                 # OpenAI 兼容客户端、双系统 Prompt 组装器、JSON 校验与自愈
│   ├── config/             # 配置管理、环境变量加载与路径管理
│   ├── data/               # K 线数据模型、几何特征引擎、指标快照
│   ├── indicators/         # EMA20、ATR14、SMA14/170 原生算法实现
│   ├── okx/                # OKX v5 REST 客户端、HMAC 签名、安全交易执行器
│   ├── orchestrator/       # 两阶段分析编排器与重试闭环
│   ├── records/            # 决策记录与经验库读取
│   ├── util/               # 密钥脱敏、时间戳工具
│   ├── web/                # Axum Web 服务、API 控制器、交易时段、静态资源内嵌
│   ├── lib.rs              # 库模块入口
│   └── main.rs             # CLI 启动入口、本地时间日志与浏览器自动唤起
├── static/                 # 前端 Web 控制台页面（HTML / CSS / JS，支持双均线与双系统图表）
├── tests/                  # 单元测试与集成测试（19 个测试全部 PASS）
├── .dockerignore           # Docker 忽略文件
├── .env.example            # 环境变量示例文件
├── .gitignore              # Git 忽略配置
├── Cargo.toml              # Rust 项目构建清单与依赖
├── Dockerfile              # 多阶段生产级 Docker 镜像构建文件
├── docker-compose.yml      # Docker Compose 一键启动编排配置
├── Makefile                # 常用构建命令集
├── start.bat               # Windows 一键启动脚本
└── start.sh                # Linux/macOS 一键启动脚本
```

---

## 🚀 快速开始

### 1. 环境准备
编译构建仅需安装 Rust 工具链（推荐 1.75+）：
- Windows: 访问 [rustup.rs](https://rustup.rs/) 下载安装。
- Linux/macOS: 运行 `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`

### 2. 启动与配置
直接运行启动程序：
- **Windows**: 双击 `start.bat` 或运行 `cargo run`
- **Linux**: 执行 `./start.sh`

> 💡 **提示**：若根目录未创建 `.env`，程序启动后会**自动打开浏览器配置窗口**，填写 API Key 后点击保存即可自动生成 `.env` 并立即生效！

您也可以手动创建 `.env` 文件（参考 `.env.example`）：
```ini
# 大模型配置 (默认 DeepSeek 官方或兼容接口)
LLM_API_KEY=sk-your-api-key
LLM_BASE_URL=https://api.deepseek.com
LLM_MODEL=deepseek-v4-flash
LLM_THINKING=false

# 默认交易系统选择 (2pa 或 dog_walking)
TRADING_SYSTEM=2pa

# OKX API 凭证 (选填：开启自动交易必须配置)
OKX_API_KEY=your-okx-api-key
OKX_SECRET_KEY=your-okx-secret-key
OKX_PASSPHRASE=your-okx-passphrase
OKX_DEMO_TRADING=true  # true 为模拟盘，false 为实盘
```

### 3. 编译发布独立 Release 程序
```bash
cargo build --release
```
编译产物位于 `target/release/`：
- **Windows**: `target/release/okx-2pa-agent.exe`（单文件独立运行）
- **Linux**: `target/release/okx-2pa-agent`

---

## 🐳 Docker 容器化极速部署 (推荐)

系统原生支持 Docker 与 Docker Compose 极速部署，自动配置多阶段构建与最小化运行环境：

### 1. 使用 Docker Compose 一键启动 (最推荐)
```bash
# 启动服务并在后台运行
docker compose up -d

# 查看实时日志
docker compose logs -f

# 停止服务
docker compose down
```

### 2. 手动构建与运行 Docker 镜像
```bash
# 1. 构建 Docker 镜像
docker build -t okx-2pa-agent:latest .

# 2. 运行容器 (挂载配置文件与记录目录)
docker run -d \
  --name okx-2pa-agent \
  -p 8088:8088 \
  -e TZ=Asia/Shanghai \
  -v $(pwd)/.env:/app/.env \
  -v $(pwd)/config:/app/config \
  -v $(pwd)/records:/app/records \
  --restart unless-stopped \
  okx-2pa-agent:latest
```

启动完成后直接在浏览器中打开 `http://<服务器IP或127.0.0.1>:8088/` 即可使用！

---

## 🌐 Web 控制台工作区

启动后访问 `http://127.0.0.1:8088/` 即可进入 Web 控制台：

1. **交易系统切换器**：顶部工具栏在 **`📊 2PA 价格行为`** 与 **`🐕 遛狗系统 (SMA 14/170)`** 间自由切换。
2. **行情与指标图表**：实时绘制 OKX K 线图，遛狗模式叠加 SMA 14 / SMA 170 双均线与数值图例，2PA 模式叠加 EMA 20 与 ATR 14。
3. **决策面板 (Decision)**：一键运行 AI 两阶段诊断，展示订单方向、入场价、止损价、TP1/TP2 止盈价（遛狗系统 TP2 锚定 170 均线）、胜率估计与逻辑推理。
4. **账户总览 (Account)**：实时读取 OKX 账户总资产、可用保证金、未实现盈亏、持仓列表与历史权益曲线。
5. **📐 合约换算 (Contract Calculator)**：热门品种合约面值对照表、任意品种实时搜索、资金量/张数双向智能换算器。
6. **🤖 自动交易 (Automation)**：开启后台新 K 线闭合自动扫描与交易调度，设置分析时段窗口与下单确认。

---

## 🧪 自动化测试套件

运行全部单元测试与集成测试：
```bash
cargo test -- --nocapture
```

测试覆盖范围（19 项测试全部通过）：
- `test_sma_calculation`：SMA 原生算法全量与增量计算校验
- `test_dog_walking_prompts`：🐕 遛狗系统两阶段 Prompt 组装与指标渲染测试
- `test_trading_system_switch`：双交易系统动态热切换与状态一致性测试
- `test_ema_calculation`：EMA 指标平滑与预热测试
- `test_atr_calculation`：Wilder ATR 真实波幅算法校验
- `test_geometry_features`：Al Brooks K 线几何特征提取测试
- `test_broker_tag_constant`：硬编码 Broker Tag 校验
- `test_build_request_contains_broker_tag`：下单参数与经纪商标签集成验证
- `test_session_presets`：交易时区与时段窗口过滤器测试
- `test_json_validator`：提示词 Markdown 剥离、JSON 外层提取与自愈校验

---

## 📡 REST API 概览

| 请求方法 | 路由路径 | 说明 |
| :--- | :--- | :--- |
| `GET` | `/` | 渲染 Web 控制台主页 |
| `GET` | `/api/status` | 获取系统运行状态、当前交易系统、Broker Tag 及自动化配置 |
| `POST` | `/api/trading_system` | 实时切换当前活跃的交易系统 (`2pa` / `dog_walking`) |
| `GET` | `/api/config` | 获取当前脱敏后的系统配置信息 |
| `POST` | `/api/config/save_env` | 保存配置至根目录 `.env` 并即时热加载 |
| `GET` | `/api/contract/specs` | 获取 OKX 永续合约面值规格与实时折合 USDT 汇算 |
| `GET` | `/api/instruments?inst_type=SWAP` | 查询 OKX 可交易合约/现货交易对列表 |
| `GET` | `/api/candles?inst_id=BTC-USDT&timeframe=15m&limit=300` | 查询已计算指标的 K 线数据 (支持 300 根深度) |
| `GET` | `/api/account` | 查询账户总览、资金余额、持仓及挂单 |
| `POST` | `/api/trade/cancel` | 撤销指定的普通限价单或策略条件单 (`ord_id` / `algo_id`) |
| `POST` | `/api/trade/cancel_all` | 一键撤销指定品种或所有活跃挂单与条件委托 |
| `POST` | `/api/analyze` | 触发单次 K 线两阶段 AI 诊断与决策 (支持传入 `trading_system`) |
| `POST` | `/api/automation` | 开启/关闭后台新 K 线闭合自动交易调度 |
| `GET` | `/api/history/decisions` | 查询历史 AI 诊断与决策记录列表 |
| `DELETE`| `/api/history/decisions/:id` | 删除指定的历史决策记录 |
| `GET` | `/api/history/trades` | 查询交易审计流水历史 |
| `DELETE`| `/api/history/trades/:id` | 删除指定的交易审计记录 |

---

## 💬 社区与交流群

欢迎加入 Discord 官方讨论组，与量化开发者及交易员共同交流 Al Brooks 价格行为学策略、遛狗均线回归策略与系统使用心得：

- 🎮 **Discord 交流群**：[https://discord.gg/jk4mnW53gK](https://discord.gg/jk4mnW53gK)

---

## ⚠️ 风险免责声明

1. 本软件仅供量化策略研究与价格行为学教学交流使用，不构成任何投资建议或财务指导。
2. 加密货币与衍生品交易具有极高的市场风险，可能导致本金全部损失。
3. 请在充分理解 Al Brooks 价格行为学原理并在**模拟盘（Demo Trading）**中充分测试后，再行决定是否使用实盘交易。开发者不对任何因程序运行或交易决策产生的经济损失承担责任。

---

## 📄 开源许可证

本项目基于 [MIT License](LICENSE) 开源发布。
