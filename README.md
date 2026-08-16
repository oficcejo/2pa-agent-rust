# OKX 2PA Agent (Rust 高性能版)

[![Rust](https://img.shields.io/badge/language-Rust%201.75+-orange.svg)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Discord](https://img.shields.io/badge/Discord-Join%20Community-5865F2?logo=discord&logoColor=white)](https://discord.gg/jk4mnW53gK)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20Linux%20%7C%20macOS-blue.svg)]()

**OKX 2PA Agent** 是一款基于 **Al Brooks 价格行为学（Price Action）理论** 与 **大语言模型（LLM）两阶段智能推理** 的高频/日内量化交易系统。

本项目采用 **纯 Rust** 进行重构与极致性能优化，支持编译为**单文件独立可执行程序**（Windows `.exe` / Linux 二进制），内嵌 Web 控制台与全部交易策略 Prompt，实现零外部运行时依赖（无需 Python、Node.js 等环境）即开即用。

---

## 🌟 核心特性与功能亮点

### 1. 纯 Rust 原生极速架构与单文件分发
- **极致性能**：毫秒级响应、超低内存占用（< 20MB），基于 Tokio + Axum 高并发异步运行时。
- **单文件独立发布（Zero Dependencies）**：通过 `rust-embed` 将 Web 控制台静态资源与 Prompt 工程规则库完全编译进二进制产物，单文件即可在任何 Windows/Linux 环境独立运行。


### 2. 智能环境配置向导 (Web Setup Wizard)
- **首次运行自动检测与浏览器唤起**：双击启动 `okx-2pa-agent.exe` 时，若未检测到 `.env` 或 API 密钥未配置，控制台输出提示并**自动调用系统默认浏览器**打开配置向导。
- **一键持久化至 `.env`**：在 Web 界面中填入大模型 API Key（默认模型 `deepseek-v4-flash`、默认接口 `https://api.deepseek.com`、默认关闭思考模式）和 OKX 凭证后，点击保存将**自动写入程序根目录 `.env`**，并在内存中即时热加载生效，下次直接启动无需重复配置。
- **顶部配置按钮**：Web 顶部栏提供「⚙️ 系统配置」入口，随时可点击修改密钥并即时生效；内置「🔗 注册okx用户」快捷开户入口。

### 3. 📐 合约规格与张数双向换算器 (Contract Calculator)
针对 OKX 永续合约按「张数」下单难以直观把握资金价值的问题，内置专属换算工作台：
- **常见热门品种 1 张面值速查表**：
  - 覆盖加密主流（BTC 1张=0.01BTC、ETH 1张=0.1ETH、SOL 1张=1SOL、DOGE 1张=1000DOGE 等）、贵金属（XAU 黄金、XAG 白银）及美股 TradFi（AAPL、TSLA、NVDA、SPX）。
  - 实时展示 **1 张合约面值**、**实时最新市价**、**1 张折合 USDT 价值**及最小下单张数。
- **支持实时查询任意合约品种**：顶部搜索框输入任意 OKX 合约（如 `PEPE-USDT-SWAP`），即时拉取其面值并计算。
- **双向智能下单换算**：
  - **💰 模式一（按投入 USDT 算张数）**：输入目标 USDT 仓位价值 + 杠杆倍数 $\rightarrow$ 自动计算**建议下单张数**、实际交易金额、所需保证金及标的币数。
  - **📊 模式二（按张数算所需 USDT）**：输入计划下单张数 + 杠杆倍数 $\rightarrow$ 自动计算**仓位总价值 (USDT)**、占用保证金及标的代币总量。

### 4. 🧠 Al Brooks 价格行为学两阶段 AI 推理
- **原生几何特征引擎**：原生计算 EMA20、True Range 与 ATR14（Wilder 递归平滑），自动提取 K 线重叠度、内部线序列（ii / iii）、反转序列（ioi）、微双顶底、跳空缺口（Gap Bar）、5 根 K 线突破质量与跟进力度。
- **阶段一（市场诊断）**：研判周期位置（Spike / Channel / Range）、多空主导力量及入场前置 Gate 校验。
- **阶段二（交易决策）**：动态路由专业策略规则书与经验库，输出确定性订单方向、挂单类型及精确价格。
- **JSON 自愈与逻辑重试**：内置 Markdown 剥离、格式修复与三价单调性校验，遇异常自动反馈重试自纠错。

### 5. 🛡️ 结构化止盈止损策略与交易所原子挂单
- **保护性止损（Stop Loss）**：
  - 做多：信号棒低点 − 1 Tick；做空：信号棒高点 + 1 Tick。
  - 动态风控：止损距离 $> 2 \times \text{ATR14}$ 或超过通道宽度的 50% 时判定风险过大直接放弃。
- **分级止盈（Take Profit）**：
  - **TP1（主目标 / 保守止盈）**：最近结构边界，严格遵循**交易者方程**与**盈亏比 $\text{RR} \ge 1.0$**。
  - **TP2（延伸目标）**：等距测量移动（Measured Move，MM）与区间 1:1 翻测。
- **OKX 原子性 `attachAlgoOrds` 挂单**：
  - 下单时同时附带 `slTriggerPx` 与 `tpTriggerPx`，主订单成交后交易所端自动激活双向条件平仓单，本地无需保持开机。

### 6. 🤖 自动交易调度与多时段管理
- **单排等宽导航栏**：【决策】 | 【账户】 | 【📐 合约换算】 | 【🤖 自动交易】，专属翠绿发光高亮。
- **多时区时段过滤器**：支持全天候（always）、美股常规盘（us_regular）、美股开盘窗口（us_open）、伦敦时段（london）、亚洲时段（asia）与自定义时区时段。

---

## 📂 项目目录结构

```text
2pa-agent-rust/
├── .cargo/                 # Cargo 编译与高速镜像源配置
├── config/                 # 运行时配置目录
│   └── settings.json       # 系统参数配置文件
├── experience/             # 历史价格行为经验库
├── prompt_engineering/     # Al Brooks 价格行为学提示词规则库
├── records/                # 运行时持久化记录与交易审计日志
│   ├── pending/            # 待跟踪决策记录
│   └── trade_audit.jsonl   # 交易执行审计流水
├── src/                    # Rust 核心源代码
│   ├── ai/                 # OpenAI 兼容客户端、Prompt 组装器、JSON 校验与自愈
│   ├── config/             # 配置管理、环境变量加载与路径管理
│   ├── data/               # K 线数据模型、几何特征引擎、指标快照
│   ├── indicators/         # EMA20、ATR14 原生算法实现
│   ├── okx/                # OKX v5 REST 客户端、HMAC 签名、安全交易执行器
│   ├── orchestrator/       # 两阶段分析编排器与重试闭环
│   ├── records/            # 决策记录与经验库读取
│   ├── util/               # 密钥脱敏、时间戳工具
│   ├── web/                # Axum Web 服务、API 控制器、交易时段、静态资源内嵌
│   ├── lib.rs              # 库模块入口
│   └── main.rs             # CLI 启动入口与浏览器自动唤起
├── static/                 # 前端 Web 控制台页面（HTML / CSS / JS）
├── tests/                  # 单元测试与集成测试
├── .env.example            # 环境变量示例文件
├── .gitignore              # Git 忽略配置
├── Cargo.toml              # Rust 项目构建清单与依赖
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

## 🌐 Web 控制台工作区

启动后访问 `http://127.0.0.1:8088/` 即可进入 Web 控制台：

1. **行情与指标图表**：实时绘制 OKX K 线图，叠加 EMA20、ATR14 与最新市场报价。
2. **决策面板 (Decision)**：一键运行 AI 两阶段诊断，展示订单方向（做多/做空）、入场价、止损价、止盈价、胜率估计与判断依据。
3. **账户总览 (Account)**：实时读取 OKX 账户总资产、可用保证金、未实现盈亏、持仓列表与历史权益曲线。
4. **📐 合约换算 (Contract Calculator)**：热门品种合约面值对照表、任意品种实时搜索、资金量/张数双向智能换算器。
5. **🤖 自动交易 (Automation)**：开启后台新 K 线闭合自动扫描与交易调度，设置分析时段窗口与下单确认。

---

## 🧪 自动化测试套件

运行全部单元测试与集成测试：
```bash
cargo test -- --nocapture
```

测试覆盖范围：
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
| `GET` | `/api/status` | 获取系统运行状态、Broker Tag 及自动化时段配置 |
| `GET` | `/api/config` | 获取当前脱敏后的系统配置信息 |
| `POST` | `/api/config/save_env` | 保存配置至根目录 `.env` 并即时热加载 |
| `GET` | `/api/contract/specs` | 获取 OKX 永续合约面值规格与实时折合 USDT 汇算 |
| `GET` | `/api/instruments?inst_type=SWAP` | 查询 OKX 可交易合约/现货交易对列表 |
| `GET` | `/api/candles?inst_id=BTC-USDT&timeframe=15m&limit=120` | 查询已计算指标的 K 线数据 |
| `GET` | `/api/account` | 查询账户总览、资金余额、持仓及挂单 |
| `POST` | `/api/analyze` | 触发单次 K 线两阶段 AI 诊断与决策 |
| `POST` | `/api/automation` | 开启/关闭后台新 K 线闭合自动交易调度 |
| `GET` | `/api/history/decisions` | 查询历史 AI 诊断与决策记录列表 |
| `DELETE`| `/api/history/decisions/:id` | 删除指定的历史决策记录 |
| `GET` | `/api/history/trades` | 查询交易审计流水历史 |
| `DELETE`| `/api/history/trades/:id` | 删除指定的交易审计记录 |

---

## 💬 社区与交流群

欢迎加入 Discord 官方讨论组，与量化开发者及交易员共同交流 Al Brooks 价格行为学策略与系统使用心得：

- 🎮 **Discord 交流群**：[https://discord.gg/jk4mnW53gK](https://discord.gg/jk4mnW53gK)

---

## ⚠️ 风险免责声明

1. 本软件仅供量化策略研究与价格行为学教学交流使用，不构成任何投资建议或财务指导。
2. 加密货币与衍生品交易具有极高的市场风险，可能导致本金全部损失。
3. 请在充分理解 Al Brooks 价格行为学原理并在**模拟盘（Demo Trading）**中充分测试后，再行决定是否使用实盘交易。开发者不对任何因程序运行或交易决策产生的经济损失承担责任。

---

## 📄 开源许可证

本项目基于 [MIT License](LICENSE) 开源发布。
