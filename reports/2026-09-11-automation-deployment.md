# 自动运行无新增记录：排查与部署进度

## 已确认原因

2026-09-10 服务器 Docker 日志显示：ETH-USDT-SWAP / 15m / 2pa_trend 在每根收盘 K 线正常触发，但第一阶段调用返回 HTTP 403：`model MiniMax-M3.0 is not allowed in your plan`。因此未完成决策，也未进入交易执行。旧记录仍在持久化目录，交易审计当时有 377 条。

同一接口的模型列表包含 `MiniMax-M3`，不包含配置中的 `MiniMax-M3.0`。上一轮已将服务器模型配置改为 `MiniMax-M3`，并把 `LLM_THINKING=ture` 更正为 `false`。模型列表存在不代表实际调用验收通过。

## 实施的代码修复

- 状态接口及页面显示后台阶段、最近成功、具体错误和重试时间。
- 自动分析失败写入错误记录；未成功分析的 K 线不标记完成，退避后可重试。
- 决策保存失败向上报告，停止后续执行。
- 历史列表每 30 秒刷新，错误展示读取 message。

## 验证

上一轮完整测试 67 项通过。2026-09-11 增强自动化回归用例，覆盖 403 留痕、退避抑制请求、同一 K 线恢复生成 WAIT 决策、成功后不重复模型调用；`cargo test --test test_web_service --locked` 3 项通过。此用例的交易所 mock 只收到 GET 请求。前端通过 `node --check static/app.js`，diff 检查通过。

## 部署与线上验收完成（2026-09-11 19:47，Asia/Shanghai）

服务器：172.245.111.20，项目 `/www/wwwroot/2pa-agent-rust`，Compose 服务 `okx-agent`，容器 `okx-2pa-agent`。挂载 `.env`、`config`、`records`。

用户通过服务器控制台确认 RSA 指纹后，以固定指纹校验连接。核查发现上一轮构建未完成，线上仍为旧容器。服务器 Cargo.toml 仍是 0.3.3，已与本地 0.4.0 及 Cargo.lock 同步；源码核对一致。采用 `--locked -j 1` 构建，旧镜像保留为 `okx-2pa-agent:before-20260911`，部署资料和日志在 `/root/2pa-deploy-backup-20260911`。

MiniMax-M3 实际调用 HTTP 200。第一轮线上分析恢复后还发现第二个原因：高周期参数 1h/4h 被 OKX 返回 51000 Parameter bar error，1H/4H 正常返回 220 根行情；修复自动分析的高周期映射，并新增对应回归断言。相关 3 项测试通过，重新构建并部署。

最终镜像 `okx-2pa-agent:fix-20260911b`（同时标记 latest），镜像 ID `sha256:17cfb4334ffb863dd55590c20f865a53273cf429468335f4a73ecacef597fd79`，容器 running。HTTPS 鉴权、运行状态字段和新版前端均验证通过。

最终分析使用 ETH-USDT-SWAP / 15m / 2pa_trend，execute=false，19:47:12 两阶段成功且决策落盘；网站历史接口确认可见，无 exception。高周期文本存在，程序候选失败原因已变为“K1 尚未收盘突破前棒且形成同向实体”，结果 WAIT。交易审计仍为 377 条，本次未下单。

重启后自动交易关闭，已恢复自动面板原标的 ETH-USDT-SWAP、15m、2pa_trend、全天时段，执行开关保持关闭；继续实盘自动交易需要用户在页面确认开启。
