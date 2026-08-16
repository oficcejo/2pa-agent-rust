const $ = (id) => document.getElementById(id);

let candles = [];
let equityPoints = [];
let statusData = {};
let instrumentOptions = [];
let visibleInstrumentValues = [];
let highlightedInstrumentIndex = -1;
let decisionRecords = [];
let tradeRecords = [];
let sessionPresetOptions = [];
let sessionDirty = false;

const instrumentGroups = [
  {
    label: "常见加密货币",
    symbols: new Set(["BTC", "ETH", "SOL"]),
  },
  {
    label: "美股",
    symbols: new Set(["AAPL", "TSLA", "NVDA", "MSFT", "AMZN", "META", "GOOGL", "AMD", "NFLX", "COIN", "MSTR"]),
  },
  {
    label: "大宗商品与指数",
    symbols: new Set(["XAU", "XAUT", "XAG", "XPT", "XPD", "CL", "NG", "SPX", "QQQ"]),
  },
];

const instrumentNames = {
  BTC: "比特币", ETH: "以太坊", SOL: "Solana", XRP: "XRP", DOGE: "Dogecoin",
  ADA: "Cardano", BNB: "BNB", AVAX: "Avalanche", LINK: "Chainlink",
  AAPL: "Apple", TSLA: "Tesla", NVDA: "NVIDIA", MSFT: "Microsoft", AMZN: "Amazon",
  META: "Meta", GOOGL: "Alphabet", AMD: "AMD", NFLX: "Netflix", COIN: "Coinbase", MSTR: "Strategy",
  XAU: "黄金", XAUT: "黄金", XAG: "白银", XPT: "铂金", XPD: "钯金",
  CL: "原油", NG: "天然气", SPX: "标普 500", QQQ: "纳斯达克 100",
};

const fmt = (value, digits = 8) => {
  if (value === null || value === undefined || value === "") return "—";
  return Number(value).toLocaleString(undefined, { maximumFractionDigits: digits });
};

const fmtMoney = (value) => {
  if (value === null || value === undefined || value === "") return "—";
  return Number(value).toLocaleString(undefined, {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  });
};

const escapeHtml = (value) => String(value ?? "")
  .replaceAll("&", "&amp;")
  .replaceAll("<", "&lt;")
  .replaceAll(">", "&gt;")
  .replaceAll('"', "&quot;")
  .replaceAll("'", "&#039;");

function toast(message) {
  const element = $("toast");
  element.textContent = message;
  element.classList.add("show");
  setTimeout(() => element.classList.remove("show"), 2600);
}

async function api(path, options = {}) {
  const response = await fetch(path, {
    headers: { "Content-Type": "application/json" },
    ...options,
  });
  const body = await response.json().catch(() => ({}));
  if (!response.ok) throw new Error(body.detail || `HTTP ${response.status}`);
  return body;
}

function setSessionWeekdays(weekdays) {
  const selected = new Set((weekdays || []).map(Number));
  document.querySelectorAll(".session-weekday").forEach((input) => {
    input.checked = selected.has(Number(input.value));
  });
}

function selectedSessionWeekdays() {
  return [...document.querySelectorAll(".session-weekday:checked")].map((input) => Number(input.value));
}

const defaultSessionPresets = [
  { key: "always", label: "全天候", timezone: "UTC", start: "00:00", end: "00:00", weekdays: [0,1,2,3,4,5,6], description: "全天运行，适合 7×24 小时市场" },
  { key: "us_regular", label: "美股常规盘", timezone: "America/New_York", start: "09:30", end: "16:00", weekdays: [0,1,2,3,4], description: "周一至周五，美东时间 09:30-16:00" },
  { key: "us_open", label: "美股开盘窗口", timezone: "America/New_York", start: "09:30", end: "11:30", weekdays: [0,1,2,3,4], description: "周一至周五，美东时间开盘后两小时" },
  { key: "london", label: "伦敦时段", timezone: "Europe/London", start: "08:00", end: "16:30", weekdays: [0,1,2,3,4], description: "周一至周五，伦敦当地时间 08:00-16:30" },
  { key: "asia", label: "亚洲时段", timezone: "Asia/Shanghai", start: "09:00", end: "16:00", weekdays: [0,1,2,3,4], description: "周一至周五，北京时间 09:00-16:00" },
];

function populateSessionPresets(options) {
  const opts = Array.isArray(options) && options.length ? options : defaultSessionPresets;
  sessionPresetOptions = opts;
  const select = $("sessionPreset");
  if (select.dataset.loaded === "true") return;
  select.replaceChildren();
  [...opts, { key: "custom", label: "自定义" }].forEach((option) => {
    const element = document.createElement("option");
    element.value = option.key;
    element.textContent = option.label;
    select.appendChild(element);
  });
  select.dataset.loaded = "true";
  select.disabled = false;
}

function formatNextSessionOpen(value, timezoneName) {
  if (!value) return "下次开放时间：持续开放";
  try {
    const formatted = new Intl.DateTimeFormat("zh-CN", {
      timeZone: timezoneName,
      month: "2-digit",
      day: "2-digit",
      weekday: "short",
      hour: "2-digit",
      minute: "2-digit",
      hour12: false,
    }).format(new Date(value));
    return `下次开放时间：${formatted} (${timezoneName})`;
  } catch (_error) {
    return `下次开放时间：${value}`;
  }
}

function renderAutomationSession(state, forceControls = false) {
  populateSessionPresets(state.automation_session_presets);
  const session = state.automation_session || {};
  const enabled = Boolean(state.auto_trading_enabled);
  const activeElement = $("sessionActive");
  activeElement.textContent = !enabled ? "自动交易关闭" : session.active ? "时段内" : "时段外暂停";
  activeElement.classList.toggle("paused", !enabled || !session.active);
  $("sessionDescription").textContent = session.description || "—";
  $("sessionNextOpen").textContent = formatNextSessionOpen(session.next_open_at, session.timezone || "UTC");
  if (sessionDirty && !forceControls) return;
  $("sessionPreset").value = session.preset || "always";
  $("sessionTimezone").value = session.timezone || "UTC";
  $("sessionStart").value = session.start || "00:00";
  $("sessionEnd").value = session.end || "00:00";
  setSessionWeekdays(session.weekdays || []);
  $("customSessionFields").hidden = session.preset !== "custom";
}

function automationRequestBody(enabled) {
  return {
    enabled,
    inst_id: $("symbol").value.trim(),
    timeframe: $("timeframe").value,
    confirmation: $("confirmation").value,
    session_preset: $("sessionPreset").value,
    session_timezone: $("sessionTimezone").value.trim(),
    session_start: $("sessionStart").value,
    session_end: $("sessionEnd").value,
    session_weekdays: selectedSessionWeekdays(),
  };
}

function renderAutomationStatus(state, forceSessionControls = false) {
  const enabled = Boolean(state.auto_trading_enabled);
  const switchElement = $("automationSwitch");
  if (switchElement) switchElement.checked = enabled;
  const statusElement = $("automationStatus");
  if (statusElement) statusElement.textContent = enabled ? "运行中" : "已关闭";
  const messageElement = $("automationMessage");
  if (messageElement) {
    if (!enabled) messageElement.textContent = "自动交易未启用";
    else if (!state.automation_session?.active) messageElement.textContent = "自动交易已启用，当前不在分析时段，已暂停分析与交易";
    else if (state.can_execute) messageElement.textContent = "自动分析与执行已生效";
    else messageElement.textContent = "已启用，但执行条件未全部满足";
  }
  renderAutomationSession(state, forceSessionControls);
}

async function loadStatus() {
  statusData = await api("/api/status");
  const modeBadge = $("modeBadge");
  if (modeBadge) {
    modeBadge.textContent = statusData.mode === "demo" ? "模拟交易" : "实盘交易";
    modeBadge.className = `badge ${statusData.mode}`;
  }
  const credentialBadge = $("credentialBadge");
  if (credentialBadge) {
    credentialBadge.textContent = statusData.credentials_configured ? "API 已配置" : "API 未配置";
    credentialBadge.className = `badge ${statusData.credentials_configured ? "good" : "muted"}`;
  }
  const brokerTagElement = $("brokerTag");
  if (brokerTagElement) brokerTagElement.textContent = statusData.broker_tag;
  renderAutomationStatus(statusData);
  const isSwap = String(statusData.symbol || "").endsWith("-SWAP");
  const autoSymbol = $("autoSymbol");
  if (autoSymbol) {
    autoSymbol.innerHTML = isSwap
      ? `<span style="color:var(--green)">${escapeHtml(statusData.symbol || "—")} (合约)</span>`
      : `<span style="color:var(--amber)">${escapeHtml(statusData.symbol || "—")} (现货)</span>`;
  }
  const riskConfidence = $("riskConfidence");
  if (riskConfidence) riskConfidence.textContent = `${statusData.confidence_threshold}%`;
  const orderSizeUnit = isSwap ? "contracts (张)" : "base units (个)";
  const orderSize = $("orderSize");
  if (orderSize) orderSize.textContent = `${statusData.default_order_size} ${orderSizeUnit}`;
  const tradeMode = $("tradeMode");
  if (tradeMode) tradeMode.textContent = `${statusData.trade_mode} / ${statusData.position_mode} / ${fmt(statusData.default_leverage, 2)}x`;
  const confirmation = $("confirmation");
  if (confirmation) {
    const code = statusData.mode === "demo" ? "ENABLE DEMO" : "ENABLE LIVE";
    confirmation.placeholder = `请在此输入 ${code} 确认开启`;
  }
  updateInstTypeBadge();
  if (statusData.latest) renderDecision(statusData.latest);
}

function updateInstTypeBadge() {
  const symbol = $("symbol").value.trim().toUpperCase();
  const isSwap = symbol.endsWith("-SWAP") || $("instType").value === "SWAP";
  const badge = $("instTypeBadge");
  if (badge) {
    if (isSwap) {
      badge.textContent = "⚡ 永续合约 (按张数/支持杠杆)";
      badge.className = "inst-type-badge swap";
    } else {
      badge.textContent = "🪙 现货币币 (全额现货/需足额现金)";
      badge.className = "inst-type-badge spot";
    }
  }
}

async function loadInstruments() {
  instrumentOptions = [];
  visibleInstrumentValues = [];
  closeInstrumentMenu();
  $("instType").disabled = true;
  $("symbol").disabled = true;
  $("symbolToggle").disabled = true;
  try {
    const markets = await Promise.all(["SPOT", "SWAP"].map(async (productType) => ({
      productType,
      rows: await api(`/api/instruments?inst_type=${productType}`),
    })));
    const current = $("symbol").value;
    const groupOrder = ["常见加密货币", "美股", "大宗商品与指数", "其他 USDT 现货", "其他 USDT 永续"];
    instrumentOptions = markets.flatMap(({ productType, rows }) => rows
      .filter((item) => {
        const id = String(item.instId || "");
        return productType === "SPOT" ? id.endsWith("-USDT") : id.endsWith("-USDT-SWAP");
      })
      .map((item) => {
        const id = String(item.instId || "");
        const root = id.split("-")[0];
        const knownGroup = instrumentGroups.find((group) => group.symbols.has(root));
        const group = knownGroup?.label || (productType === "SPOT" ? "其他 USDT 现货" : "其他 USDT 永续");
        const name = instrumentNames[root] || "";
        const marketLabel = productType === "SPOT" ? "现货" : "永续";
        return {
          id, productType, group, name, marketLabel,
          search: `${id} ${name} ${group} ${marketLabel}`.toUpperCase(),
        };
      }));
    instrumentOptions.sort((left, right) => {
      const groupDifference = groupOrder.indexOf(left.group) - groupOrder.indexOf(right.group);
      if (groupDifference) return groupDifference;
      const group = instrumentGroups.find((item) => item.label === left.group);
      if (group) {
        const priority = [...group.symbols];
        const rootDifference = priority.indexOf(left.id.split("-")[0]) - priority.indexOf(right.id.split("-")[0]);
        if (rootDifference) return rootDifference;
        if (left.productType !== right.productType) return left.productType === "SPOT" ? -1 : 1;
      }
      return left.id.localeCompare(right.id);
    });

    const preferred = $("instType").value === "SWAP" ? "BTC-USDT-SWAP" : "BTC-USDT";
    const available = new Set(instrumentOptions.map((item) => item.id));
    $("symbol").value = available.has(current)
      ? current
      : available.has(preferred) ? preferred : String(instrumentOptions[0]?.id || "");
    closeInstrumentMenu();
  } catch (error) {
    toast(error.message);
  } finally {
    $("instType").disabled = false;
    $("symbol").disabled = false;
    $("symbolToggle").disabled = false;
  }
}

function closeInstrumentMenu() {
  $("instrumentMenu").hidden = true;
  $("symbol").setAttribute("aria-expanded", "false");
  highlightedInstrumentIndex = -1;
}

function renderInstrumentMenu(query = "") {
  const normalized = query.trim().toUpperCase();
  const matches = instrumentOptions
    .filter((item) => !normalized || item.search.includes(normalized))
    .slice(0, normalized ? 120 : 300);
  visibleInstrumentValues = matches.map((item) => item.id);
  highlightedInstrumentIndex = matches.length ? 0 : -1;

  if (!matches.length) {
    $("instrumentMenu").innerHTML = '<div class="instrument-empty">未找到匹配品种</div>';
  } else {
    const groups = new Map();
    matches.forEach((item) => {
      if (!groups.has(item.group)) groups.set(item.group, []);
      groups.get(item.group).push(item);
    });
    $("instrumentMenu").innerHTML = [...groups.entries()].map(([label, items]) => `
      <section class="instrument-menu-group">
        <div class="instrument-menu-label">${escapeHtml(label)}</div>
        ${items.map((item, index) => `
          <button class="instrument-option${index === 0 && label === matches[0].group ? " active" : ""}"
            type="button" role="option" data-value="${escapeHtml(item.id)}">
            <strong>${escapeHtml(item.id)}</strong>
            <small>${escapeHtml(`${item.name}${item.name ? " · " : ""}${item.marketLabel}`)}</small>
          </button>`).join("")}
      </section>`).join("");
  }
  $("instrumentMenu").hidden = false;
  $("symbol").setAttribute("aria-expanded", "true");
}

function chooseInstrument(value) {
  const selected = instrumentOptions.find((item) => item.id === value);
  if (selected) $("instType").value = selected.productType;
  $("symbol").value = value;
  closeInstrumentMenu();
  loadCandles();
}

function moveInstrumentHighlight(direction) {
  const options = [...$("instrumentMenu").querySelectorAll(".instrument-option")];
  if (!options.length) return;
  highlightedInstrumentIndex = (highlightedInstrumentIndex + direction + options.length) % options.length;
  options.forEach((option, index) => option.classList.toggle("active", index === highlightedInstrumentIndex));
  options[highlightedInstrumentIndex].scrollIntoView({ block: "nearest" });
}

async function changeInstrumentType() {
  const productType = $("instType").value;
  const preferred = productType === "SWAP" ? "BTC-USDT-SWAP" : "BTC-USDT";
  const current = instrumentOptions.find((item) => item.id === $("symbol").value);
  if (!current || current.productType !== productType) {
    const fallback = instrumentOptions.find((item) => item.id === preferred)
      || instrumentOptions.find((item) => item.productType === productType);
    $("symbol").value = fallback?.id || "";
  }
  await loadCandles();
}

async function loadCandles() {
  updateInstTypeBadge();
  const symbol = $("symbol").value.trim().toUpperCase();
  const timeframe = $("timeframe").value;
  $("chartEmpty").style.display = "grid";
  try {
    candles = await api(`/api/candles?inst_id=${encodeURIComponent(symbol)}&timeframe=${timeframe}&limit=160`);
    if ($("symbol").value.trim().toUpperCase() !== symbol || $("timeframe").value !== timeframe) return;
    candles.sort((a, b) => a.ts_open - b.ts_open);
    const last = candles.at(-1);
    if (last) {
      $("lastPrice").textContent = fmt(last.close);
      $("periodChange").textContent = `${((last.close / last.open - 1) * 100).toFixed(2)}%`;
      $("periodChange").style.color = last.close >= last.open ? "var(--green)" : "var(--red)";
      $("highPrice").textContent = fmt(last.high);
      $("lowPrice").textContent = fmt(last.low);
      $("volume").textContent = fmt(last.volume);
    }
    drawChart();
  } catch (error) {
    $("chartEmpty").textContent = error.message;
    toast(error.message);
  }
}

function drawChart() {
  const canvas = $("chartCanvas");
  const wrap = $("chart");
  const dpr = window.devicePixelRatio || 1;
  const width = wrap.clientWidth;
  const height = wrap.clientHeight;
  canvas.width = width * dpr;
  canvas.height = height * dpr;
  const context = canvas.getContext("2d");
  context.scale(dpr, dpr);
  context.clearRect(0, 0, width, height);
  if (!candles.length) return;

  $("chartEmpty").style.display = "none";
  const padding = { left: 12, right: 68, top: 16, bottom: 26 };
  const data = candles.slice(-120);
  const high = Math.max(...data.map((item) => item.high));
  const low = Math.min(...data.map((item) => item.low));
  const range = high - low || 1;
  const plotWidth = width - padding.left - padding.right;
  const plotHeight = height - padding.top - padding.bottom;
  const y = (value) => padding.top + ((high - value) / range) * plotHeight;
  const x = (index) => padding.left + (index + 0.5) * plotWidth / data.length;

  context.strokeStyle = "#202628";
  context.fillStyle = "#7c878a";
  context.font = "11px Segoe UI";
  for (let index = 0; index <= 5; index += 1) {
    const yy = padding.top + index * plotHeight / 5;
    const value = high - index * range / 5;
    context.beginPath();
    context.moveTo(padding.left, yy);
    context.lineTo(width - padding.right, yy);
    context.stroke();
    context.fillText(fmt(value), width - padding.right + 7, yy + 4);
  }

  const candleWidth = Math.max(2, plotWidth / data.length * 0.62);
  data.forEach((bar, index) => {
    const color = bar.close >= bar.open ? "#1fc48d" : "#f05b67";
    const xx = x(index);
    context.strokeStyle = color;
    context.fillStyle = color;
    context.beginPath();
    context.moveTo(xx, y(bar.high));
    context.lineTo(xx, y(bar.low));
    context.stroke();
    const top = y(Math.max(bar.open, bar.close));
    const bodyHeight = Math.max(1, Math.abs(y(bar.open) - y(bar.close)));
    context.fillRect(xx - candleWidth / 2, top, candleWidth, bodyHeight);
  });
}

function renderDecision(result) {
  const decision = result.decision || {};
  const direction = decision.order_direction || "不下单";
  $("decisionDirection").textContent = direction;
  $("decisionDirection").className = `direction ${direction === "做多" ? "long" : direction === "做空" ? "short" : "neutral"}`;
  $("confidence").textContent = `信心 ${decision.trade_confidence ?? "—"}%`;
  $("orderType").textContent = decision.order_type || "—";
  $("entryPrice").textContent = fmt(decision.entry_price);
  $("stopPrice").textContent = fmt(decision.stop_loss_price);
  $("targetPrice").textContent = fmt(decision.take_profit_price);
  $("target2Price").textContent = fmt(decision.take_profit_price_2);
  $("winRate").textContent = decision.estimated_win_rate == null ? "—" : `${decision.estimated_win_rate}%`;
  $("reasoning").textContent = decision.reasoning || result.exception?.message || "无交易决策";
  $("executionResult").textContent = result.execution ? JSON.stringify(result.execution, null, 2) : "未提交订单";
}

function formatHistoryTime(timestamp) {
  if (!timestamp) return "时间未知";
  return new Date(Number(timestamp)).toLocaleString("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function directionMeta(value) {
  if (value === "做多" || value === "buy") return { label: value === "buy" ? "买入" : "做多", className: "long" };
  if (value === "做空" || value === "sell") return { label: value === "sell" ? "卖出" : "做空", className: "short" };
  return { label: "不下单", className: "neutral" };
}

function renderDecisionHistory() {
  $("decisionHistoryCount").textContent = `${decisionRecords.length} 条`;
  $("decisionHistoryState").hidden = decisionRecords.length > 0;
  $("decisionHistoryState").textContent = decisionRecords.length ? "" : "暂无决策记录";
  $("decisionHistory").innerHTML = decisionRecords.map((record) => {
    const symbol = record.symbol || record.meta?.symbol || "未知品种";
    const timeframe = record.timeframe || record.meta?.timeframe || "—";
    const timestamp = record.timestamp_ms || record.meta?.timestamp_local_ms || 0;
    const directionVal = record.direction || record.stage2_decision?.decision?.order_direction || record.stage2_decision?.order_direction || "不下单";
    const orderTypeVal = record.order_type || record.stage2_decision?.decision?.order_type || record.stage2_decision?.order_type || "无订单";
    const confidenceVal = record.confidence ?? record.stage2_decision?.decision?.trade_confidence ?? record.stage2_decision?.trade_confidence;

    const direction = directionMeta(directionVal);
    const exceptionText = typeof record.exception === "string" ? record.exception : (record.exception?.message || "");
    const exception = exceptionText ? `<span class="history-error">${escapeHtml(exceptionText)}</span>` : "";
    const confidence = confidenceVal == null ? "信心 —" : `信心 ${escapeHtml(confidenceVal)}%`;
    const id = record.id || `${symbol}_${timeframe}_${timestamp}`;

    return `
      <div class="history-row" data-record-id="${escapeHtml(id)}">
        <button class="history-main" type="button" data-action="open-decision" data-id="${escapeHtml(id)}">
          <span class="history-line">
            <strong>${escapeHtml(symbol)}</strong>
            <span class="history-direction ${direction.className}">${direction.label}</span>
            <b>${confidence}</b>
          </span>
          <span class="history-detail">${formatHistoryTime(timestamp)} · ${escapeHtml(timeframe)} · ${escapeHtml(orderTypeVal)}</span>
          ${exception}
        </button>
        <button class="history-delete" type="button" data-action="delete-decision" data-id="${escapeHtml(id)}" title="删除决策记录" aria-label="删除决策记录">×</button>
      </div>`;
  }).join("");
}

async function loadDecisionHistory(silent = false) {
  if (!silent) {
    $("decisionHistoryState").hidden = false;
    $("decisionHistoryState").textContent = "正在读取记录";
  }
  try {
    decisionRecords = await api("/api/history/decisions?limit=50");
    renderDecisionHistory();
  } catch (error) {
    if (!silent) {
      $("decisionHistoryState").hidden = false;
      $("decisionHistoryState").textContent = error.message;
    }
  }
}

function showDecisionRecord(recordId) {
  const record = decisionRecords.find((item) => item.id === recordId || `${item.symbol || item.meta?.symbol}_${item.timeframe || item.meta?.timeframe}_${item.timestamp_ms || item.meta?.timestamp_local_ms}` === recordId);
  if (!record) return;

  const innerDec = record.stage2_decision?.decision || record.stage2_decision || {};
  const decisionData = {
    order_direction: record.direction || innerDec.order_direction,
    order_type: record.order_type || innerDec.order_type,
    trade_confidence: record.confidence ?? innerDec.trade_confidence,
    entry_price: record.entry_price ?? innerDec.entry_price,
    stop_loss_price: record.stop_loss_price ?? innerDec.stop_loss_price,
    take_profit_price: record.take_profit_price ?? innerDec.take_profit_price,
    take_profit_price_2: record.take_profit_price_2 ?? innerDec.take_profit_price_2,
    estimated_win_rate: record.estimated_win_rate ?? innerDec.estimated_win_rate,
    reasoning: record.reasoning || innerDec.reasoning || innerDec.narrative,
  };

  renderDecision({
    decision: decisionData,
    stage1: record.stage1_diagnosis,
    stage2: record.stage2_decision,
    exception: record.exception ? { message: typeof record.exception === "string" ? record.exception : (record.exception.message || JSON.stringify(record.exception)) } : null,
    execution: null,
  });
  document.querySelectorAll("#decisionHistory .history-row").forEach((row) => {
    row.classList.toggle("selected", row.dataset.recordId === recordId);
  });
}

function formatTradeReason(reason) {
  if (!reason) return "";
  const r = String(reason).trim();
  if (r === "signal expired") return "信号已过期 (K线生成时间已超时)";
  if (r === "duplicate signal") return "重复信号 (当前K线周期已处理或挂单)";
  if (r.includes("open position exists for") || r.includes("adding to position is disabled")) {
    const match = r.match(/open position exists for (.*?);/);
    const inst = match ? match[1] : "";
    return `${inst} 已存在活跃持仓，系统已启用持仓互斥保护（禁止同向加仓）`;
  }
  if (r.includes("decision does not contain an executable order")) {
    return "决策为不下单或不包含可执行订单";
  }
  if (r.includes("is below threshold")) {
    return r.replace(/trade confidence (\d+) is below threshold (\d+)/, "交易信心度 $1% 低于设定风控门槛 $2%");
  }
  if (r.includes("is below minSz")) {
    return r.replace(/order size (.+) is below minSz (.+)/, "下单数量 $1 低于交易所最小下单量 $2");
  }
  if (r.includes("must satisfy stop < entry < target")) {
    return "做多价格关系异常：必须满足 止损价 < 入场价 < 止盈价";
  }
  if (r.includes("must satisfy target < entry < stop")) {
    return "做空价格关系异常：必须满足 止盈价 < 入场价 < 止损价";
  }
  if (r.includes("missing entry_price")) return "缺少入场价 (entry_price)";
  if (r.includes("missing stop_loss_price")) return "缺少止损价 (stop_loss_price)";
  if (r.includes("missing take_profit_price")) return "缺少止盈价 (take_profit_price)";
  if (r.includes("OKX instrument not found")) return r.replace("OKX instrument not found:", "未找到 OKX 合约产品规格:");
  if (r.includes("OKX API error")) {
    return `OKX 接口错误: ${r.replace(/OKX API error \[\d+\]: /, "")}`;
  }
  return r;
}

function renderTradeHistory() {
  $("tradeHistoryCount").textContent = `${tradeRecords.length} 条`;
  $("tradeHistoryState").hidden = tradeRecords.length > 0;
  $("tradeHistoryState").textContent = tradeRecords.length ? "" : "暂无交易记录";
  const orderTypes = { limit: "限价", market: "市价", trigger: "触发" };
  $("tradeHistory").innerHTML = tradeRecords.map((record) => {
    const direction = directionMeta(record.direction);
    const statusClass = record.submitted ? "submitted" : "rejected";
    const statusLabel = record.submitted ? "已提交" : "未提交";
    const price = record.price == null ? "市价" : fmt(record.price);
    const detail = [
      formatHistoryTime(record.timestamp_ms),
      record.timeframe || "—",
      orderTypes[record.order_type] || record.order_type || "无订单",
      `数量 ${fmt(record.size)}`,
      `价格 ${price}`,
    ].join(" · ");
    return `
      <div class="history-row">
        <div class="history-main trade-history-main">
          <span class="history-line">
            <strong>${escapeHtml(record.instrument || "未知品种")}</strong>
            <span class="history-direction ${direction.className}">${direction.label}</span>
            <b class="history-status ${statusClass}">${statusLabel}</b>
          </span>
          <span class="history-detail">${escapeHtml(detail)}</span>
          ${record.reason ? `<span class="history-error">${escapeHtml(formatTradeReason(record.reason))}</span>` : ""}
        </div>
        <button class="history-delete" type="button" data-action="delete-trade" data-id="${escapeHtml(record.id)}" title="删除交易记录" aria-label="删除交易记录">×</button>
      </div>`;
  }).join("");
}

async function loadTradeHistory(silent = false) {
  if (!silent) {
    $("tradeHistoryState").hidden = false;
    $("tradeHistoryState").textContent = "正在读取记录";
  }
  try {
    tradeRecords = await api("/api/history/trades?limit=50");
    renderTradeHistory();
  } catch (error) {
    if (!silent) {
      $("tradeHistoryState").hidden = false;
      $("tradeHistoryState").textContent = error.message;
    }
  }
}

async function deleteHistoryRecord(kind, recordId) {
  const label = kind === "decisions" ? "决策" : "交易";
  if (!window.confirm(`确定删除这条${label}记录？`)) return;
  try {
    await api(`/api/history/${kind}/${encodeURIComponent(recordId)}`, { method: "DELETE" });
    if (kind === "decisions") await loadDecisionHistory(true);
    else await loadTradeHistory(true);
    toast(`${label}记录已删除`);
  } catch (error) {
    toast(error.message);
  }
}

async function analyze() {
  const button = $("analyzeButton");
  button.disabled = true;
  $("analysisState").textContent = "正在获取行情并运行两阶段 AI…";
  try {
    const result = await api("/api/analyze", {
      method: "POST",
      body: JSON.stringify({
        inst_id: $("symbol").value.trim(),
        timeframe: $("timeframe").value,
        bar_count: 100,
        execute: $("executeAfterAnalysis").checked,
      }),
    });
    renderDecision(result);
    loadDecisionHistory(true);
    if ($("executeAfterAnalysis").checked) loadTradeHistory(true);
    $("analysisState").textContent = result.exception ? `失败：${result.exception.message}` : "分析完成";
    toast(result.execution?.submitted ? "订单已提交" : "分析完成");
  } catch (error) {
    $("analysisState").textContent = `失败：${error.message}`;
    toast(error.message);
  } finally {
    button.disabled = false;
  }
}

function pnlClass(value) {
  const number = Number(value || 0);
  return number > 0 ? "positive" : number < 0 ? "negative" : "";
}

function formatAccountTime(timestamp) {
  if (!timestamp) return "—";
  return new Date(Number(timestamp)).toLocaleString(undefined, {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function renderBalances(rows) {
  $("balanceCount").textContent = rows.length;
  $("balancesBody").innerHTML = rows.length
    ? rows.map((row) => `
      <tr>
        <td><strong>${escapeHtml(row.currency)}</strong></td>
        <td>${fmt(row.equity)}</td>
        <td>${fmt(row.available)}</td>
        <td>${fmt(row.frozen)}</td>
      </tr>`).join("")
    : '<tr class="empty-row"><td colspan="4">无非零资产</td></tr>';
}

function renderPositions(rows) {
  $("positionCount").textContent = rows.length;
  $("positionsBody").innerHTML = rows.length
    ? rows.map((row) => {
      const label = row.direction === "short" ? "空" : "多";
      const ratio = row.unrealized_pnl_ratio == null ? "" : `${(Number(row.unrealized_pnl_ratio) * 100).toFixed(2)}%`;
      return `
        <tr>
          <td><strong>${escapeHtml(row.instrument)}</strong><span class="side-label ${escapeHtml(row.direction)}">${label}</span></td>
          <td>${row.base_size == null ? fmt(row.size) : `${fmt(row.base_size)} ${escapeHtml(row.base_size_currency)}`}<small>${row.contract_size == null ? "" : `${fmt(row.contract_size)} contracts`} ${row.leverage ? `/ ${fmt(row.leverage, 2)}x` : escapeHtml(row.margin_mode)}</small></td>
          <td>${fmt(row.average_price)}<small>${fmt(row.mark_price)}</small></td>
          <td class="${pnlClass(row.unrealized_pnl)}">${fmtMoney(row.unrealized_pnl)}<small>${ratio}</small></td>
        </tr>`;
    }).join("")
    : '<tr class="empty-row"><td colspan="4">暂无仓位</td></tr>';
}

function renderOrders(rows) {
  $("orderCount").textContent = rows.length;
  const typeLabels = { limit: "限价", market: "市价", trigger: "触发", oco: "TP/SL" };
  $("ordersBody").innerHTML = rows.length
    ? rows.map((row) => {
      const direction = row.direction === "sell" ? "short" : "long";
      const directionLabel = row.direction === "sell" ? "卖" : "买";
      return `
        <tr>
          <td><strong>${escapeHtml(row.instrument)}</strong><span class="side-label ${direction}">${directionLabel}</span></td>
          <td>${escapeHtml(typeLabels[row.order_type] || row.order_type || "—")}<small>${escapeHtml(row.state)}</small></td>
          <td>${fmt(row.size)}<small>已成 ${fmt(row.filled_size)}</small></td>
          <td>${row.order_type === "oco" ? `TP ${fmt(row.take_profit_price)}<small>SL ${fmt(row.stop_loss_price)}</small>` : fmt(row.price)}</td>
        </tr>`;
    }).join("")
    : '<tr class="empty-row"><td colspan="4">暂无挂单</td></tr>';
}

function drawEquityChart() {
  const canvas = $("equityCanvas");
  const wrap = canvas.parentElement;
  const placeholder = $("equityEmpty");
  const width = wrap.clientWidth;
  const height = wrap.clientHeight;
  const dpr = window.devicePixelRatio || 1;
  canvas.width = width * dpr;
  canvas.height = height * dpr;
  const context = canvas.getContext("2d");
  context.scale(dpr, dpr);
  context.clearRect(0, 0, width, height);

  if (!equityPoints.length) {
    placeholder.style.display = "grid";
    return;
  }
  placeholder.style.display = "none";

  const points = equityPoints.slice(-160);
  const values = points.map((point) => Number(point.value));
  const rawMin = Math.min(...values);
  const rawMax = Math.max(...values);
  const paddingValue = Math.max((rawMax - rawMin) * 0.12, Math.max(Math.abs(rawMax), 1) * 0.002);
  const low = rawMin - paddingValue;
  const high = rawMax + paddingValue;
  const range = high - low || 1;
  const padding = { left: 8, right: 54, top: 14, bottom: 24 };
  const plotWidth = width - padding.left - padding.right;
  const plotHeight = height - padding.top - padding.bottom;
  const x = (index) => padding.left + (points.length === 1 ? plotWidth / 2 : index * plotWidth / (points.length - 1));
  const y = (value) => padding.top + ((high - value) / range) * plotHeight;

  context.strokeStyle = "#252b2e";
  context.fillStyle = "#7f898d";
  context.font = "10px Segoe UI";
  [rawMax, (rawMax + rawMin) / 2, rawMin].forEach((value) => {
    const yy = y(value);
    context.beginPath();
    context.moveTo(padding.left, yy);
    context.lineTo(width - padding.right, yy);
    context.stroke();
    context.fillText(fmtMoney(value), width - padding.right + 7, yy + 3);
  });

  const rising = values.at(-1) >= values[0];
  const color = rising ? "#1fc48d" : "#f05b67";
  context.beginPath();
  points.forEach((point, index) => {
    const xx = x(index);
    const yy = y(Number(point.value));
    if (index === 0) context.moveTo(xx, yy);
    else context.lineTo(xx, yy);
  });
  context.lineWidth = 2;
  context.strokeStyle = color;
  context.stroke();

  context.lineTo(x(points.length - 1), padding.top + plotHeight);
  context.lineTo(x(0), padding.top + plotHeight);
  context.closePath();
  context.fillStyle = rising ? "rgba(31,196,141,.08)" : "rgba(240,91,103,.08)";
  context.fill();

  const last = points.at(-1);
  context.beginPath();
  context.arc(x(points.length - 1), y(Number(last.value)), 3, 0, Math.PI * 2);
  context.fillStyle = color;
  context.fill();

  context.fillStyle = "#737d81";
  context.fillText(formatAccountTime(points[0].ts), padding.left, height - 7);
  const endLabel = formatAccountTime(last.ts);
  const endWidth = context.measureText(endLabel).width;
  context.fillText(endLabel, width - padding.right - endWidth, height - 7);
}

function renderAccount(account) {
  const summary = account.summary || {};
  $("totalEquity").textContent = fmtMoney(summary.total_equity_usd);
  $("availableEquity").textContent = fmtMoney(summary.available_equity_usd);
  $("accountUpl").textContent = fmtMoney(summary.unrealized_pnl);
  $("accountUpl").className = pnlClass(summary.unrealized_pnl);
  $("accountUpdated").textContent = formatAccountTime(summary.updated_at_ms || Date.now());

  equityPoints = account.equity_curve || [];
  $("equityRange").textContent = equityPoints.length ? `${equityPoints.length} 点` : "—";
  renderBalances(account.balances || []);
  renderPositions(account.positions || []);
  renderOrders(account.orders || []);
  drawEquityChart();
}

async function loadAccount(silent = false) {
  const state = $("accountState");
  const dashboard = $("accountDashboard");
  if (!silent && dashboard.hidden) {
    state.textContent = "正在读取账户";
    state.style.display = "block";
  }
  try {
    const account = await api("/api/account");
    if (!account.configured) {
      dashboard.hidden = true;
      state.textContent = "OKX API 未配置";
      state.style.display = "block";
      return;
    }
    dashboard.hidden = false;
    state.style.display = "none";
    renderAccount(account);
  } catch (error) {
    if (!silent) {
      dashboard.hidden = true;
      state.textContent = error.message;
      state.style.display = "block";
    }
  }
}

async function toggleAutomation() {
  const switchElement = $("automationSwitch");
  const expectedCode = statusData?.mode === "demo" ? "ENABLE DEMO" : "ENABLE LIVE";
  const confirmVal = $("confirmation").value.trim().toUpperCase();

  if (switchElement.checked && confirmVal !== expectedCode) {
    switchElement.checked = false;
    $("confirmation").focus();
    $("confirmation").classList.add("confirm-input-pulse");
    setTimeout(() => $("confirmation").classList.remove("confirm-input-pulse"), 1500);
    toast(`⚠️ 开启失败：必须在上方输入框输入「${expectedCode}」！`);
    return;
  }

  try {
    const state = await api("/api/automation", {
      method: "POST",
      body: JSON.stringify(automationRequestBody(switchElement.checked)),
    });
    statusData = state;
    sessionDirty = false;
    renderAutomationStatus(state, true);
    toast($("automationMessage").textContent);
  } catch (error) {
    switchElement.checked = !switchElement.checked;
    toast(error.message);
  }
}

function changeSessionPreset() {
  const preset = $("sessionPreset").value;
  const option = sessionPresetOptions.find((item) => item.key === preset);
  if (option) {
    $("sessionTimezone").value = option.timezone;
    $("sessionStart").value = option.start;
    $("sessionEnd").value = option.end;
    setSessionWeekdays(option.weekdays);
    $("sessionDescription").textContent = option.description;
  } else {
    $("sessionDescription").textContent = "按自定义时区、时间和星期运行";
  }
  $("customSessionFields").hidden = preset !== "custom";
  sessionDirty = true;
}

async function applyAutomationSession() {
  const button = $("sessionApply");
  button.disabled = true;
  try {
    const state = await api("/api/automation", {
      method: "POST",
      body: JSON.stringify(automationRequestBody($("automationSwitch").checked)),
    });
    statusData = state;
    sessionDirty = false;
    renderAutomationStatus(state, true);
    toast("分析时段已应用");
  } catch (error) {
    toast(error.message);
  } finally {
    button.disabled = false;
  }
}

document.querySelectorAll(".tabs button").forEach((button) => button.addEventListener("click", () => {
  document.querySelectorAll(".tabs button,.tab-content").forEach((item) => item.classList.remove("active"));
  button.classList.add("active");
  $(`${button.dataset.tab}Tab`).classList.add("active");
  if (button.dataset.tab === "account") loadAccount();
  if (button.dataset.tab === "contract") loadContractSpecs();
  if (button.dataset.tab === "decision") loadDecisionHistory();
  if (button.dataset.tab === "automation") loadTradeHistory();
}));

$("instType").addEventListener("change", changeInstrumentType);
$("symbolToggle").addEventListener("click", () => {
  if ($("instrumentMenu").hidden) renderInstrumentMenu();
  else closeInstrumentMenu();
  $("symbol").focus();
});
$("symbol").addEventListener("focus", () => {
  if ($("instrumentMenu").hidden) renderInstrumentMenu();
});
$("symbol").addEventListener("input", () => renderInstrumentMenu($("symbol").value));
$("symbol").addEventListener("keydown", (event) => {
  if (event.key === "ArrowDown" || event.key === "ArrowUp") {
    event.preventDefault();
    if ($("instrumentMenu").hidden) renderInstrumentMenu($("symbol").value);
    moveInstrumentHighlight(event.key === "ArrowDown" ? 1 : -1);
  } else if (event.key === "Enter") {
    event.preventDefault();
    const value = visibleInstrumentValues[highlightedInstrumentIndex] || $("symbol").value.trim().toUpperCase();
    if (value) chooseInstrument(value);
  } else if (event.key === "Escape") {
    closeInstrumentMenu();
  }
});
$("instrumentMenu").addEventListener("mousedown", (event) => {
  const option = event.target.closest(".instrument-option");
  if (!option) return;
  event.preventDefault();
  chooseInstrument(option.dataset.value);
});
document.addEventListener("mousedown", (event) => {
  if (!event.target.closest(".symbol-control")) closeInstrumentMenu();
});
$("timeframe").addEventListener("change", loadCandles);
$("refreshButton").addEventListener("click", loadCandles);
$("analyzeButton").addEventListener("click", analyze);
$("accountRefresh").addEventListener("click", () => loadAccount());
$("decisionHistoryRefresh").addEventListener("click", () => loadDecisionHistory());
$("tradeHistoryRefresh").addEventListener("click", () => loadTradeHistory());
$("sessionPreset").addEventListener("change", changeSessionPreset);
$("customSessionFields").addEventListener("input", () => { sessionDirty = true; });
$("sessionApply").addEventListener("click", applyAutomationSession);
$("decisionHistory").addEventListener("click", (event) => {
  const button = event.target.closest("button[data-action]");
  if (!button) return;
  if (button.dataset.action === "open-decision") showDecisionRecord(button.dataset.id);
  if (button.dataset.action === "delete-decision") deleteHistoryRecord("decisions", button.dataset.id);
});
$("tradeHistory").addEventListener("click", (event) => {
  const button = event.target.closest("button[data-action='delete-trade']");
  if (button) deleteHistoryRecord("trades", button.dataset.id);
});
$("automationSwitch").addEventListener("change", toggleAutomation);
window.addEventListener("resize", () => {
  drawChart();
  if (!$("accountDashboard").hidden) drawEquityChart();
});

// ==================== 系统配置向导交互 ====================
async function openConfigModal() {
  const modal = $("configModal");
  $("configSaveMsg").textContent = "";
  modal.hidden = false;
  try {
    const cfg = await api("/api/config");
    $("cfgLlmBaseUrl").value = cfg.llm_base_url || "https://api.deepseek.com";
    $("cfgLlmModel").value = cfg.llm_model || "deepseek-v4-flash";
    $("cfgLlmThinking").checked = Boolean(cfg.llm_thinking);
    if (cfg.okx_base_url) $("cfgOkxBaseUrl").value = cfg.okx_base_url;
    $("cfgOkxDemoTrading").value = String(cfg.okx_demo_trading !== false);
    if (cfg.okx_default_order_size) $("cfgOkxOrderSize").value = cfg.okx_default_order_size;
    if (cfg.okx_default_leverage) $("cfgOkxLeverage").value = cfg.okx_default_leverage;
    if (cfg.okx_trade_mode) $("cfgOkxTradeMode").value = cfg.okx_trade_mode;
  } catch (err) {
    console.warn("加载配置失败:", err);
  }
}

function closeConfigModal() {
  $("configModal").hidden = true;
}

async function handleSaveConfig(event) {
  event.preventDefault();
  const btn = $("saveConfigBtn");
  btn.disabled = true;
  $("configSaveMsg").textContent = "正在保存到 .env 并热加载...";
  try {
    const payload = {
      llm_api_key: $("cfgLlmApiKey").value.trim(),
      llm_base_url: $("cfgLlmBaseUrl").value.trim(),
      llm_model: $("cfgLlmModel").value.trim(),
      llm_thinking: $("cfgLlmThinking").checked,
      okx_api_key: $("cfgOkxApiKey").value.trim(),
      okx_secret_key: $("cfgOkxSecretKey").value.trim(),
      okx_passphrase: $("cfgOkxPassphrase").value.trim(),
      okx_base_url: "https://www.okx.com",
      okx_demo_trading: $("cfgOkxDemoTrading").value === "true",
      okx_default_order_size: Number($("cfgOkxOrderSize").value) || 1.0,
      okx_default_leverage: Number($("cfgOkxLeverage").value) || 3.0,
      okx_trade_mode: $("cfgOkxTradeMode").value,
      okx_position_mode: "net",
    };

    await api("/api/config/save_env", {
      method: "POST",
      body: JSON.stringify(payload),
    });

    toast("✅ 配置已成功保存至根目录 .env！");
    closeConfigModal();
    await loadStatus();
    if ($("accountTab").classList.contains("active")) loadAccount();
  } catch (error) {
    $("configSaveMsg").textContent = `保存失败: ${error.message}`;
    toast(error.message);
  } finally {
    btn.disabled = false;
  }
}

$("configButton").addEventListener("click", openConfigModal);
$("closeConfigBtn").addEventListener("click", closeConfigModal);
$("cancelConfigBtn").addEventListener("click", closeConfigModal);
$("configForm").addEventListener("submit", handleSaveConfig);
$("configModal").addEventListener("click", (e) => {
  if (e.target === $("configModal")) closeConfigModal();
});

// ==================== 合约规格与张数换算 ====================
let contractSpecsList = [];
let currentContractSpec = null;
let contractCalcMode = "usdt";

function renderContractSpecCard(spec) {
  if (!spec) return;
  currentContractSpec = spec;
  $("specInstId").textContent = spec.inst_id;
  $("specLastPrice").textContent = spec.last_price > 0 ? `${fmt(spec.last_price, 4)} USDT` : "暂无报价";
  
  const ccy = spec.ct_val_ccy || (spec.inst_id.split("-")[0]);
  $("specCtVal").textContent = `${spec.ct_val} ${ccy}`;
  
  const usdtVal = spec.usdt_per_contract || (spec.ct_val * spec.last_price);
  $("specUsdtPerCt").textContent = `≈ ${fmtMoney(usdtVal)} USDT`;
  
  $("specMinSz").textContent = `${spec.min_sz} 张`;
  $("specLotSz").textContent = `${spec.lot_sz} 张`;
  $("specMaxLev").textContent = `${spec.max_leverage}x`;
  $("specTickSz").textContent = `${spec.tick_sz}`;
  
  recalculateContractValues();
}

function recalculateContractValues() {
  if (!currentContractSpec) return;
  const lastPrice = currentContractSpec.last_price || 0;
  const ctVal = currentContractSpec.ct_val || 1;
  const usdtPerCt = currentContractSpec.usdt_per_contract || (ctVal * lastPrice);
  const minSz = currentContractSpec.min_sz || 1;
  const lotSz = currentContractSpec.lot_sz || 1;

  if (contractCalcMode === "usdt") {
    const targetUsdt = Math.max(0, Number($("inputTargetUsdt").value) || 0);
    const leverage = Math.max(1, Number($("inputLeverage1").value) || 1);

    if (usdtPerCt > 0 && targetUsdt > 0) {
      let rawContracts = Math.floor((targetUsdt / usdtPerCt) / lotSz) * lotSz;
      if (rawContracts < minSz) rawContracts = minSz;
      
      const actualUsdt = rawContracts * usdtPerCt;
      const margin = actualUsdt / leverage;
      const coinAmt = rawContracts * ctVal;

      $("calcResContracts").textContent = `${rawContracts} 张`;
      $("calcResActualUsdt").textContent = `${fmtMoney(actualUsdt)} USDT`;
      $("calcResMargin").textContent = `${fmtMoney(margin)} USDT`;
      $("calcResCoins").textContent = `${fmt(coinAmt, 4)} ${currentContractSpec.ct_val_ccy || ""}`;
    } else {
      $("calcResContracts").textContent = "0 张";
      $("calcResActualUsdt").textContent = "0.00 USDT";
      $("calcResMargin").textContent = "0.00 USDT";
      $("calcResCoins").textContent = "0.00";
    }
  } else {
    const targetContracts = Math.max(0, Number($("inputTargetContracts").value) || 0);
    const leverage = Math.max(1, Number($("inputLeverage2").value) || 1);

    if (usdtPerCt > 0 && targetContracts > 0) {
      const totalUsdt = targetContracts * usdtPerCt;
      const margin = totalUsdt / leverage;
      const coinAmt = targetContracts * ctVal;

      $("calcResTotalUsdt").textContent = `${fmtMoney(totalUsdt)} USDT`;
      $("calcResMargin2").textContent = `${fmtMoney(margin)} USDT`;
      $("calcResCoins2").textContent = `${fmt(coinAmt, 4)} ${currentContractSpec.ct_val_ccy || ""}`;
      $("calcResSingleVal").textContent = `${fmtMoney(usdtPerCt)} USDT`;
    } else {
      $("calcResTotalUsdt").textContent = "0.00 USDT";
      $("calcResMargin2").textContent = "0.00 USDT";
      $("calcResCoins2").textContent = "0.00";
      $("calcResSingleVal").textContent = "0.00 USDT";
    }
  }
}

function renderPopularSpecsList() {
  const container = $("popularSpecsList");
  if (!container) return;
  if (!contractSpecsList.length) {
    container.innerHTML = `<div class="empty-state">暂无合约数据</div>`;
    return;
  }

  container.innerHTML = contractSpecsList.map((spec) => {
    const rawSymbol = spec.inst_id.split("-")[0];
    const name = instrumentNames[rawSymbol] || spec.uly || "";
    const ccy = spec.ct_val_ccy || rawSymbol;
    const usdtVal = spec.usdt_per_contract || (spec.ct_val * spec.last_price);
    return `
      <div class="spec-row-item" data-inst-id="${escapeHtml(spec.inst_id)}" title="点击填入上方换算器">
        <div class="spec-row-left">
          <strong>${escapeHtml(spec.inst_id)} ${name ? `(${escapeHtml(name)})` : ""}</strong>
          <small>市价: ${spec.last_price > 0 ? fmt(spec.last_price, 4) : "—"} USDT · 最小 ${spec.min_sz} 张</small>
        </div>
        <div class="spec-row-right">
          <span class="ct-val-badge">1张 = ${spec.ct_val} ${escapeHtml(ccy)}</span>
          <span class="usdt-val">≈ ${fmtMoney(usdtVal)} USDT</span>
        </div>
      </div>`;
  }).join("");
}

async function loadContractSpecs(query = "") {
  const loading = $("popularSpecsLoading");
  if (loading) loading.hidden = false;
  try {
    const url = query ? `/api/contract/specs?symbol=${encodeURIComponent(query)}` : "/api/contract/specs";
    const data = await api(url);
    contractSpecsList = data.specs || [];
    if (loading) loading.hidden = true;
    renderPopularSpecsList();

    if (contractSpecsList.length) {
      const searchTarget = (query || $("calcSymbolInput").value || "").trim().toUpperCase();
      const match = contractSpecsList.find(s => s.inst_id.toUpperCase() === searchTarget) || contractSpecsList[0];
      $("calcSymbolInput").value = match.inst_id;
      renderContractSpecCard(match);
    }
  } catch (error) {
    if (loading) {
      loading.hidden = false;
      loading.textContent = `加载失败: ${error.message}`;
    }
  }
}

// 换算器交互绑定
$("calcModeUsdtBtn").addEventListener("click", () => {
  contractCalcMode = "usdt";
  $("calcModeUsdtBtn").classList.add("active");
  $("calcModeContractsBtn").classList.remove("active");
  $("calcModeUsdtPanel").hidden = false;
  $("calcModeContractsPanel").hidden = true;
  recalculateContractValues();
});

$("calcModeContractsBtn").addEventListener("click", () => {
  contractCalcMode = "contracts";
  $("calcModeContractsBtn").classList.add("active");
  $("calcModeUsdtBtn").classList.remove("active");
  $("calcModeContractsPanel").hidden = false;
  $("calcModeUsdtPanel").hidden = true;
  recalculateContractValues();
});

$("inputTargetUsdt").addEventListener("input", recalculateContractValues);
$("inputLeverage1").addEventListener("input", recalculateContractValues);
$("inputTargetContracts").addEventListener("input", recalculateContractValues);
$("inputLeverage2").addEventListener("input", recalculateContractValues);

$("calcSymbolSearchBtn").addEventListener("click", () => {
  loadContractSpecs($("calcSymbolInput").value.trim());
});

$("calcSymbolInput").addEventListener("keydown", (e) => {
  if (e.key === "Enter") {
    e.preventDefault();
    loadContractSpecs($("calcSymbolInput").value.trim());
  }
});

$("contractRefreshBtn").addEventListener("click", () => {
  loadContractSpecs($("calcSymbolInput").value.trim());
});

$("popularSpecsList").addEventListener("click", (e) => {
  const item = e.target.closest(".spec-row-item");
  if (!item) return;
  const instId = item.dataset.instId;
  const match = contractSpecsList.find(s => s.inst_id === instId);
  if (match) {
    $("calcSymbolInput").value = match.inst_id;
    renderContractSpecCard(match);
    toast(`已载入 ${match.inst_id} 规格`);
  }
});

Promise.all([loadStatus(), loadInstruments(), loadCandles(), loadDecisionHistory(), loadTradeHistory()])
  .then(() => {
    // 若尚未配置 AI 或未检测到 .env，自动弹出向导
    if (statusData && (!statusData.is_ai_configured || !statusData.has_env_file)) {
      openConfigModal();
    }
  })
  .catch((error) => toast(error.message));

setInterval(loadCandles, 30000);
setInterval(() => loadStatus().catch((error) => toast(error.message)), 15000);
setInterval(() => {
  if ($("accountTab").classList.contains("active")) loadAccount(true);
  if ($("contractTab").classList.contains("active")) loadContractSpecs($("calcSymbolInput").value.trim());
}, 60000);


