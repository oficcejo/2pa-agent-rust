function historySystemLabel(id) { return ({"2pa":"旧版 2PA", "dog_walking":"旧版遛狗", "alpha_pilot":"历史 AlphaPilot (已开源独立)", "legacy_unknown":"旧版未归属"})[id] || systemLabel(id); }
function canonicalSystem(id) { if (id === "alpha_pilot") return "2pa_trend"; return ({"2pa":"2pa_trend", "dog_walking":"dog_reversion"})[id] || id; }
function systemLabel(id) { return ({"2pa_trend":"2PA 趋势确认", "dog_reversion":"遛狗偏离回归", "dog_trend":"遛狗顺势回踩", "adaptive":"自适应观察（不开新仓）"})[canonicalSystem(id)] || id || "旧版未归属"; }
function isDogSystem(id) { return ["dog_reversion", "dog_trend"].includes(canonicalSystem(id)); }
const $ = (id) => document.getElementById(id);

let candles = [];
let equityPoints = [];
let statusData = {};
let instrumentOptions = [];
let visibleInstrumentValues = [];
let highlightedInstrumentIndex = -1;
let decisionRecords = [];
let tradeRecords = [];
let learningReport = null;
let learningOutcomes = [];
let sessionPresetOptions = [];
let sessionDirty = false;
let currentTradingSystem = "2pa_trend";
try {
  const savedSys = localStorage.getItem("okx_trading_system");
  if (["dog_walking", "2pa", "2pa_trend", "dog_reversion", "dog_trend", "adaptive", "alpha_pilot"].includes(savedSys)) {
    currentTradingSystem = canonicalSystem(savedSys);
    if (savedSys === "alpha_pilot") {
      try { localStorage.setItem("okx_trading_system", "2pa_trend"); } catch {}
    }
  }
} catch {}

function computeSMA(bars, period) {
  const result = new Array(bars.length).fill(null);
  let sum = 0;
  for (let i = 0; i < bars.length; i++) {
    sum += bars[i].close;
    if (i >= period) {
      sum -= bars[i - period].close;
    }
    if (i >= period - 1) {
      result[i] = sum / period;
    } else if (bars.length < period && i >= 3) {
      result[i] = sum / (i + 1);
    }
  }
  return result;
}

function computeEMA(bars, period) {
  const result = new Array(bars.length).fill(null);
  if (bars.length < period) return result;
  const k = 2 / (period + 1);
  let seed = 0;
  for (let i = 0; i < period; i++) seed += bars[i].close;
  let prev = seed / period;
  result[period - 1] = prev;
  for (let i = period; i < bars.length; i++) {
    prev = bars[i].close * k + prev * (1 - k);
    result[i] = prev;
  }
  return result;
}

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
  const text = await response.text();
  let body;
  try { body = JSON.parse(text); } catch { body = { detail: text }; }
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
    trading_system: currentTradingSystem || $("tradingSystemSelect")?.value || "2pa",
  };
}

function renderAutomationStatus(state, forceSessionControls = false) {
  const enabled = Boolean(state.auto_trading_enabled);
  const switchElement = $("automationSwitch");
  if (switchElement) switchElement.checked = enabled;
  const statusElement = $("automationStatus");
  const runtime = state.automation_runtime || {};
  const phaseLabels = { error: "运行异常", analyzing: "分析中", waiting_bar: "等待收盘", checking_orders: "检查挂单", checking_candles: "检查行情", outside_session: "时段外暂停" };
  if (statusElement) statusElement.textContent = enabled ? (phaseLabels[runtime.phase] || "等待检查") : "已关闭";
  const messageElement = $("automationMessage");
  if (messageElement) {
    if (!enabled) messageElement.textContent = "自动交易未启用";
    else if (runtime.phase === "error") messageElement.textContent = `${runtime.last_error || runtime.message}${runtime.next_retry_ms ? `；下次重试：${formatHistoryTime(runtime.next_retry_ms)}` : ""}`;
    else if (!state.automation_session?.active) messageElement.textContent = "自动交易已启用，当前不在分析时段，已暂停分析与交易";
    else if (state.can_execute) messageElement.textContent = `${runtime.message || "自动交易已开启，等待后台检查"}${runtime.last_success_ms ? `；最近成功：${formatHistoryTime(runtime.last_success_ms)}` : "；尚无成功分析"}`;
    else messageElement.textContent = "已启用，但执行条件未全部满足";
  }
  const autoSystem = $("autoSystem");
  if (autoSystem) {
    const sys = state.trading_system || currentTradingSystem;
    autoSystem.textContent = systemLabel(sys);
  }
  renderAutomationSession(state, forceSessionControls);
}

async function loadStatus() {
  try {
    statusData = await api("/api/status");
  } catch (error) {
    // Surface the failure instead of leaving the badge stuck on 连接中 and
    // letting the rejection abort the rest of the startup chain.
    const failedBadge = $("modeBadge");
    if (failedBadge) {
      failedBadge.textContent = "状态读取失败";
      failedBadge.className = "badge muted";
    }
    toast(`读取系统状态失败：${error.message}`);
    return;
  }
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
  const orderSizeUnit = isSwap ? "张" : "个";
  const orderSize = $("orderSize");
  if (orderSize) {
    if (statusData.auto_order_sizing) {
      orderSize.innerHTML = `<span style="color:var(--green);font-weight:700">动态自适应算量</span> <small>(风控 ${statusData.risk_percent || 2}% / 顶格 ${statusData.max_margin_percent || 25}%)</small>`;
    } else {
      orderSize.textContent = `${statusData.default_order_size} ${orderSizeUnit}`;
    }
  }
  const tradeMode = $("tradeMode");
  if (tradeMode) tradeMode.textContent = `${statusData.trade_mode} / ${statusData.position_mode} / ${fmt(statusData.default_leverage, 2)}x`;
  const confirmation = $("confirmation");
  if (confirmation) {
    const code = statusData.mode === "demo" ? "ENABLE DEMO" : "ENABLE LIVE";
    confirmation.placeholder = `请在此输入 ${code} 确认开启`;
  }
  if (statusData.trading_system) {
    currentTradingSystem = canonicalSystem(statusData.trading_system);
    try {
      localStorage.setItem("okx_trading_system", currentTradingSystem);
    } catch {}
    updateSystemUI();
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

const instrumentMarketLabel = (productType) => (productType === "SPOT" ? "现货" : "永续");

// Fetch one product type in isolation, with a single retry.
//
// The OKX public instruments endpoint is occasionally reset mid-flight. A
// retry costs 400ms and removes the most common cause of an empty menu.
async function fetchInstrumentMarket(productType) {
  let lastError = "";
  for (let attempt = 0; attempt < 2; attempt += 1) {
    try {
      const rows = await api(`/api/instruments?inst_type=${productType}`);
      return { productType, rows: Array.isArray(rows) ? rows : [], failed: false, error: "" };
    } catch (error) {
      lastError = error.message || String(error);
      if (attempt === 0) await new Promise((resolve) => setTimeout(resolve, 400));
    }
  }
  return { productType, rows: [], failed: true, error: lastError };
}

async function loadInstruments() {
  visibleInstrumentValues = [];
  closeInstrumentMenu();
  $("instType").disabled = true;
  $("symbol").disabled = true;
  $("symbolToggle").disabled = true;
  const current = $("symbol").value;
  try {
    // SPOT and SWAP are independent requests; isolating them means one failing
    // market can never blank out the other's instruments.
    const markets = await Promise.all(["SPOT", "SWAP"].map(fetchInstrumentMarket));
    const groupOrder = ["常见加密货币", "美股", "大宗商品与指数", "其他 USDT 现货", "其他 USDT 永续"];
    const loaded = markets
      .filter((market) => !market.failed)
      .flatMap(({ productType, rows }) => rows
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
          const marketLabel = instrumentMarketLabel(productType);
          return {
            id, productType, group, name, marketLabel,
            search: `${id} ${name} ${group} ${marketLabel}`.toUpperCase(),
          };
        }));

    // Only replace the list once something actually arrived, so a failed
    // refresh never destroys a previously usable menu.
    if (loaded.length) {
      instrumentOptions = loaded;
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
    }

    const preferred = $("instType").value === "SWAP" ? "BTC-USDT-SWAP" : "BTC-USDT";
    const available = new Set(instrumentOptions.map((item) => item.id));
    if (available.size) {
      $("symbol").value = available.has(current)
        ? current
        : available.has(preferred) ? preferred : instrumentOptions[0].id;
    } else if (current) {
      // Never blank out what the user already had on a failed load.
      $("symbol").value = current;
    }

    const failed = markets.filter((market) => market.failed);
    if (failed.length) {
      const labels = failed.map((market) => instrumentMarketLabel(market.productType)).join("、");
      toast(`${labels}品种加载失败（${failed[0].error}）；已展示其余品种，可点「刷新行情」重试`);
    }
  } catch (error) {
    toast(error.message);
  } finally {
    $("instType").disabled = false;
    $("symbol").disabled = false;
    $("symbolToggle").disabled = false;
    closeInstrumentMenu();
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
    // Distinguish "nothing matched your search" from "the list never loaded",
    // which the old message conflated and left unexplained.
    $("instrumentMenu").innerHTML = instrumentOptions.length
      ? '<div class="instrument-empty">未找到匹配品种</div>'
      : '<div class="instrument-empty">品种列表未加载，请点「刷新行情」(↻) 重试</div>';
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
    candles = await api(`/api/candles?inst_id=${encodeURIComponent(symbol)}&timeframe=${timeframe}&limit=300`);
    if ($("symbol").value.trim().toUpperCase() !== symbol || $("timeframe").value !== timeframe) return;
    candles.sort((a, b) => a.ts_open - b.ts_open);
    const viewKey = `${symbol}|${timeframe}`;
    if (chartView.symbolKey !== viewKey) resetChartView(symbol, timeframe);
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

function updateSystemUI() {
  const title = $("analysisTitle");
  if (title) {
    title.textContent = systemLabel(currentTradingSystem) + "分析";
  }
  const btn = $("analyzeButton");
  if (btn) {
    btn.textContent = "运行 AI 分析";
  }
  const autoSys = $("autoSystem");
  if (autoSys) {
    autoSys.textContent = systemLabel(currentTradingSystem);
  }
  const select = $("tradingSystemSelect");
  if (select && select.value !== currentTradingSystem) {
    select.value = currentTradingSystem;
  }
}

// ---- K 线视口：拖动平移 / 滚轮缩放 / 十字光标 ----
const chartView = {
  visibleCount: 120,
  endOffset: 0,
  hover: null,
  dragging: false,
  dragStartX: 0,
  dragStartEndOffset: 0,
  symbolKey: "",
};

function clampChartEndOffset(value) {
  const total = candles.length;
  const maxOffset = Math.max(0, total - 15);
  return Math.min(maxOffset, Math.max(0, Math.round(value)));
}

function resetChartView(symbol, timeframe) {
  chartView.symbolKey = `${symbol}|${timeframe}`;
  chartView.visibleCount = 120;
  chartView.endOffset = 0;
  chartView.hover = null;
}

function formatChartTime(ts) {
  const date = new Date(Number(ts));
  return date.toLocaleString("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function setupChartInteraction() {
  const wrap = $("chart");
  const canvas = $("chartCanvas");
  if (!wrap || !canvas || wrap.dataset.bound === "1") return;
  wrap.dataset.bound = "1";
  wrap.style.cursor = "crosshair";

  wrap.addEventListener("mousedown", (event) => {
    if (event.button !== 0) return;
    chartView.dragging = true;
    chartView.dragStartX = event.clientX;
    chartView.dragStartEndOffset = chartView.endOffset;
    wrap.style.cursor = "grabbing";
    event.preventDefault();
  });

  window.addEventListener("mousemove", (event) => {
    const rect = wrap.getBoundingClientRect();
    const x = event.clientX - rect.left;
    const y = event.clientY - rect.top;
    const inside = x >= 0 && y >= 0 && x <= rect.width && y <= rect.height;

    if (chartView.dragging) {
      const padding = { left: 12, right: 68 };
      const plotWidth = Math.max(1, rect.width - padding.left - padding.right);
      const barWidth = plotWidth / chartView.visibleCount;
      const bars = (event.clientX - chartView.dragStartX) / barWidth;
      chartView.endOffset = clampChartEndOffset(chartView.dragStartEndOffset + bars);
      drawChart();
      return;
    }

    if (inside) {
      chartView.hover = { x, y };
      drawChart();
    } else if (chartView.hover) {
      chartView.hover = null;
      drawChart();
    }
  });

  window.addEventListener("mouseup", () => {
    if (!chartView.dragging) return;
    chartView.dragging = false;
    wrap.style.cursor = "crosshair";
  });

  wrap.addEventListener("mouseleave", () => {
    chartView.hover = null;
    if (!chartView.dragging) drawChart();
  });

  wrap.addEventListener("wheel", (event) => {
    event.preventDefault();
    if (!candles.length) return;
    const rect = wrap.getBoundingClientRect();
    const padding = { left: 12, right: 68, top: 32, bottom: 26 };
    const plotWidth = Math.max(1, rect.width - padding.left - padding.right);
    const oldCount = chartView.visibleCount;
    const factor = event.deltaY > 0 ? 1.12 : 0.89;
    let newCount = Math.round(oldCount * factor);
    newCount = Math.min(320, Math.max(20, newCount));
    if (newCount === oldCount) return;

    const total = candles.length;
    const oldEnd = total - chartView.endOffset;
    const oldStart = oldEnd - oldCount;
    const mouseX = event.clientX - rect.left;
    const absIndex = oldStart + ((mouseX - padding.left) / plotWidth) * oldCount;
    const newBar = plotWidth / newCount;
    const newStart = absIndex - (mouseX - padding.left) / newBar;
    const newEnd = newStart + newCount;
    chartView.visibleCount = newCount;
    chartView.endOffset = clampChartEndOffset(total - newEnd);
    drawChart();
  }, { passive: false });

  wrap.addEventListener("dblclick", () => {
    chartView.visibleCount = 120;
    chartView.endOffset = 0;
    drawChart();
  });
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
  const padding = { left: 12, right: 68, top: 32, bottom: 26 };

  // Calculate indicator arrays on the full sorted candle series
  const sma14All = computeSMA(candles, 14);
  const sma170All = computeSMA(candles, 170);
  const ema20All = computeEMA(candles, 20);

  const total = candles.length;
  chartView.visibleCount = Math.min(chartView.visibleCount, Math.max(20, total));
  chartView.endOffset = clampChartEndOffset(chartView.endOffset);
  const endIndex = total - chartView.endOffset;
  const startIndex = Math.max(0, endIndex - chartView.visibleCount);
  const data = candles.slice(startIndex, endIndex);
  const sma14 = sma14All.slice(startIndex, endIndex);
  const sma170 = sma170All.slice(startIndex, endIndex);
  const ema20 = ema20All.slice(startIndex, endIndex);

  let high = Math.max(...data.map((item) => item.high));
  let low = Math.min(...data.map((item) => item.low));

  // Include visible indicator lines in min/max bounds so lines are never clipped
  if (isDogSystem(currentTradingSystem)) {
    sma14.forEach((val) => { if (val != null) { high = Math.max(high, val); low = Math.min(low, val); } });
    sma170.forEach((val) => { if (val != null) { high = Math.max(high, val); low = Math.min(low, val); } });
  } else {
    ema20.forEach((val) => { if (val != null) { high = Math.max(high, val); low = Math.min(low, val); } });
  }

  const range = (high - low) || 1;
  const plotWidth = width - padding.left - padding.right;
  const plotHeight = height - padding.top - padding.bottom;
  const y = (value) => padding.top + ((high - value) / range) * plotHeight;
  const x = (index) => padding.left + (index + 0.5) * plotWidth / data.length;

  // Grid lines and price axis
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

  // Draw Candlesticks
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

  // Draw Indicator Curves & Top Legend
  if (isDogSystem(currentTradingSystem)) {
    // 1. Draw SMA 170 (Blue Line - Owner)
    context.save();
    context.strokeStyle = "#38bdf8";
    context.lineWidth = 2.2;
    context.beginPath();
    let started170 = false;
    for (let i = 0; i < data.length; i++) {
      if (sma170[i] != null) {
        const xx = x(i);
        const yy = y(sma170[i]);
        if (!started170) { context.moveTo(xx, yy); started170 = true; }
        else { context.lineTo(xx, yy); }
      }
    }
    if (started170) context.stroke();
    context.restore();

    // 2. Draw SMA 14 (Orange Line - Dog Leash)
    context.save();
    context.strokeStyle = "#fb923c";
    context.lineWidth = 1.8;
    context.beginPath();
    let started14 = false;
    for (let i = 0; i < data.length; i++) {
      if (sma14[i] != null) {
        const xx = x(i);
        const yy = y(sma14[i]);
        if (!started14) { context.moveTo(xx, yy); started14 = true; }
        else { context.lineTo(xx, yy); }
      }
    }
    if (started14) context.stroke();
    context.restore();

    // 3. Draw Legend at Top-Left (Matching Reference Image)
    const last14 = sma14.filter((v) => v != null).at(-1);
    const last170 = sma170.filter((v) => v != null).at(-1);
    context.font = "bold 12px Segoe UI, sans-serif";
    context.fillStyle = "#cbd5e1";
    context.fillText("双移动平均线 14 170 Simple", padding.left + 4, 18);
    let offset = padding.left + 175;
    if (last14 != null) {
      context.fillStyle = "#fb923c";
      context.fillText(fmt(last14), offset, 18);
      offset += 75;
    }
    if (last170 != null) {
      context.fillStyle = "#38bdf8";
      context.fillText(fmt(last170), offset, 18);
    }
  } else {
    // 2PA Mode: Draw EMA 20 (Cyan Line)
    context.save();
    context.strokeStyle = "#22d3ee";
    context.lineWidth = 1.8;
    context.beginPath();
    let startedEma = false;
    for (let i = 0; i < data.length; i++) {
      if (ema20[i] != null) {
        const xx = x(i);
        const yy = y(ema20[i]);
        if (!startedEma) { context.moveTo(xx, yy); startedEma = true; }
        else { context.lineTo(xx, yy); }
      }
    }
    if (startedEma) context.stroke();
    context.restore();

    const lastEma = ema20.filter((v) => v != null).at(-1);
    context.font = "bold 12px Segoe UI, sans-serif";
    context.fillStyle = "#cbd5e1";
    context.fillText("指数移动平均线 EMA 20", padding.left + 4, 18);
    if (lastEma != null) {
      context.fillStyle = "#22d3ee";
      context.fillText(fmt(lastEma), padding.left + 155, 18);
    }
  }

  // Time axis labels
  const labelStep = Math.max(1, Math.ceil(data.length / 6));
  context.fillStyle = "#7c878a";
  context.font = "11px Segoe UI";
  context.textAlign = "center";
  for (let i = 0; i < data.length; i += labelStep) {
    context.fillText(formatChartTime(data[i].ts_open), x(i), height - 8);
  }
  context.textAlign = "left";

  // Crosshair + OHLC tooltip
  const hover = chartView.hover;
  if (hover && data.length) {
    const plotLeft = padding.left;
    const plotRight = width - padding.right;
    const plotTop = padding.top;
    const plotBottom = height - padding.bottom;
    const clampedX = Math.min(plotRight, Math.max(plotLeft, hover.x));
    const clampedY = Math.min(plotBottom, Math.max(plotTop, hover.y));
    const idx = Math.min(data.length - 1, Math.max(0, Math.floor((clampedX - plotLeft) / plotWidth * data.length)));
    const bar = data[idx];
    const cx = x(idx);
    const cy = clampedY;
    const price = high - ((cy - plotTop) / plotHeight) * range;

    context.save();
    context.strokeStyle = "rgba(137,147,151,0.55)";
    context.lineWidth = 1;
    context.setLineDash([4, 4]);
    context.beginPath();
    context.moveTo(cx, plotTop);
    context.lineTo(cx, plotBottom);
    context.moveTo(plotLeft, cy);
    context.lineTo(plotRight, cy);
    context.stroke();
    context.setLineDash([]);
    context.restore();

    context.fillStyle = "#2a3236";
    context.fillRect(width - padding.right + 2, cy - 9, padding.right - 4, 18);
    context.fillStyle = "#edf1f2";
    context.font = "11px Segoe UI";
    context.fillText(fmt(price), width - padding.right + 6, cy + 4);

    context.fillStyle = "#2a3236";
    context.fillRect(cx - 52, height - padding.bottom + 2, 104, 16);
    context.fillStyle = "#edf1f2";
    context.textAlign = "center";
    context.fillText(formatChartTime(bar.ts_open), cx, height - padding.bottom + 13);
    context.textAlign = "left";

    const isUp = bar.close >= bar.open;
    const ohlc = `${formatChartTime(bar.ts_open)}  开 ${fmt(bar.open, 2)}  高 ${fmt(bar.high, 2)}  低 ${fmt(bar.low, 2)}  收 ${fmt(bar.close, 2)}  量 ${fmt(bar.volume, 2)}`;
    context.font = "12px Segoe UI";
    const textWidth = context.measureText(ohlc).width;
    context.fillStyle = "rgba(14,17,18,0.88)";
    context.fillRect(padding.left + 4, 24, textWidth + 12, 20);
    context.fillStyle = isUp ? "#1fc48d" : "#f05b67";
    context.fillText(ohlc, padding.left + 10, 38);
  } else if (data.length) {
    context.fillStyle = "#5c666a";
    context.font = "11px Segoe UI";
    context.fillText("拖动平移 · 滚轮缩放 · 双击复位", padding.left + 4, height - 8);
  }
}

function renderDecision(result) {
  const decision = result.decision || {};
  const action = decision.action || "";
  const orderType = decision.order_type || "";
  let direction = decision.order_direction || "不下单";

  if (action === "MOVE_STOP_LOSS" || orderType === "修改止损") {
    direction = "🛡️ 移动止损";
  } else if (action === "CLOSE_EARLY" || orderType === "平仓") {
    direction = "🚪 主动平仓";
  } else if (action === "HOLD" || orderType === "持有") {
    direction = "💎 继续持有";
  }

  const sys = result.trading_system || result.meta?.trading_system || currentTradingSystem;
  $("decisionDirection").textContent = direction;
  
  let dirClass = "neutral";
  if (direction === "做多" || direction.includes("做多")) dirClass = "long";
  else if (direction === "做空" || direction.includes("做空")) dirClass = "short";
  else if (action === "MOVE_STOP_LOSS" || orderType === "修改止损") dirClass = "long";
  else if (action === "CLOSE_EARLY" || orderType === "平仓") dirClass = "short";
  else if (action === "HOLD" || orderType === "持有") dirClass = "long";
  
  $("decisionDirection").className = `direction ${dirClass}`;
  $("confidence").textContent = `信心 ${decision.trade_confidence ?? "—"}%`;
  $("orderType").textContent = decision.order_type || (action ? action : "—");
  $("entryPrice").textContent = fmt(decision.entry_price);
  
  const stopDisplay = decision.new_stop_loss_price != null 
    ? `${fmt(decision.new_stop_loss_price)} (新止损)` 
    : fmt(decision.stop_loss_price);
  $("stopPrice").textContent = stopDisplay;
  
  $("targetPrice").textContent = decision.take_profit_price != null ? `${fmt(decision.take_profit_price)}` : "—";
  $("target2Price").textContent = fmt(decision.take_profit_price_2);
  $("netRiskReward").textContent = decision.strategy_version && decision.risk_reward_ratio != null ? fmt(decision.risk_reward_ratio) : "—";
  $("winRate").textContent = decision.estimated_win_rate == null ? "无统计数据" : `${decision.estimated_win_rate}%`;

  // Render Position Context Badge
  const posBadge = $("positionContextBadge");
  const posCtx = result.position_context || result.position;
  if (posBadge) {
    if (posCtx && posCtx.has_position) {
      const isLong = (posCtx.pos_side || "").toLowerCase() === "long";
      posBadge.hidden = false;
      posBadge.className = `position-context-badge ${isLong ? "long" : "short"}`;
      const pnlText = posCtx.unrealized_pnl_ratio != null 
        ? `${posCtx.unrealized_pnl_ratio >= 0 ? "+" : ""}${Number(posCtx.unrealized_pnl_ratio).toFixed(2)}%` 
        : "—";
      const pnlUsdt = posCtx.unrealized_pnl != null 
        ? ` (${posCtx.unrealized_pnl >= 0 ? "+" : ""}${Number(posCtx.unrealized_pnl).toFixed(2)} U)` 
        : "";
      const slText = posCtx.current_sl ? fmt(posCtx.current_sl) : "未设";
      posBadge.innerHTML = `<strong>🛡️ 当前实盘持仓：</strong>${isLong ? "做多" : "做空"} <b>${posCtx.pos_size}</b> 张 | 均价 <b>${fmt(posCtx.open_avg_px)}</b> | 浮盈 <b style="color:${posCtx.unrealized_pnl_ratio >= 0 ? 'var(--green)' : 'var(--red)'}">${pnlText}${pnlUsdt}</b> | 挂设止损 <b>${slText}</b>`;
    } else {
      posBadge.hidden = true;
      posBadge.innerHTML = "";
    }
  }

  const reasoningPrefix = isDogSystem(sys)
    ? "【🐕 遛狗系统决策】"
    : (sys === "adaptive"
        ? "【🧠 智能自适应决策】"
        : "【📊 2PA 价格行为决策】");
  $("reasoning").textContent = decision.reasoning ? `${reasoningPrefix}\n${decision.reasoning}` : (result.exception?.message || "无交易决策");
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
          <span class="history-detail">${formatHistoryTime(timestamp)} · ${escapeHtml(timeframe)} · ${escapeHtml(historySystemLabel(record.trading_system || record.meta?.trading_system))} · ${escapeHtml(orderTypeVal)}</span>
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
    strategy_version: innerDec.strategy_version,
    risk_reward_ratio: innerDec.risk_reward_ratio,
    reasoning: record.reasoning || innerDec.reasoning || innerDec.narrative,
  };

  renderDecision({
    decision: decisionData,
    trading_system: record.trading_system || record.meta?.trading_system,
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
      historySystemLabel(record.strategy_id),
      record.strategy_version || "旧版本",
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

function formatR(value) {
  if (value == null || !Number.isFinite(Number(value))) return "—";
  const n = Number(value);
  return `${n >= 0 ? "+" : ""}${n.toFixed(2)}R`;
}

function formatPercent(value) {
  if (value == null || !Number.isFinite(Number(value))) return "—";
  return `${(Number(value) * 100).toFixed(1)}%`;
}

function renderLearning() {
  const report = learningReport || {};
  const outcomes = Array.isArray(learningOutcomes) ? learningOutcomes : [];
  const overall = report.overall || {};
  const counts = report.outcomes || {};
  const prompt = report.active_prompt || {};

  $("learningState").hidden = true;
  $("learningDashboard").hidden = false;

  $("learningOutcomeTotal").textContent = String(counts.total ?? 0);
  $("learningQualified").textContent = String(counts.qualified ?? 0);
  $("learningExpectancy").textContent = (overall.samples ? formatR(overall.expectancy_r) : "—");
  $("learningWinRate").textContent = (overall.samples ? formatPercent(overall.win_rate) : "—");

  const cb = statusData?.circuit_breaker || {};
  const shadow = statusData?.shadow_trading || {};
  if ($("drawdownCurrent")) $("drawdownCurrent").textContent = `$${(cb.current_drawdown_usd || 0).toFixed(2)}`;
  if ($("drawdownLimit")) $("drawdownLimit").textContent = `$${(cb.max_loss_usd || 500).toFixed(2)}`;
  if ($("shadowActiveCount")) $("shadowActiveCount").textContent = `${shadow.active_positions || 0} 笔`;
  if ($("shadowClosedCount")) $("shadowClosedCount").textContent = `${shadow.closed_outcomes || 0} 笔`;
  if ($("lifecycleHookHint")) $("lifecycleHookHint").textContent = shadow.enabled ? "影子模式: 开启" : "影子模式: 关闭 (真实/模拟执行)";
  if ($("circuitBreakerBadge")) {
    const tripped = !!cb.tripped;
    $("circuitBreakerBadge").textContent = tripped ? "已熔断" : "正常";
    $("circuitBreakerBadge").classList.toggle("rejected", tripped);
    $("circuitBreakerBadge").classList.toggle("good", !tripped);
  }
  if ($("circuitBreakerNote")) {
    $("circuitBreakerNote").textContent = cb.tripped
      ? `已触发单日回撤熔断（累计亏损 $${(cb.current_drawdown_usd || 0).toFixed(2)}，已超 $${(cb.max_loss_usd || 500).toFixed(2)} 限额），自动交易及开仓已暂停`
      : `单日累计亏损 $${(cb.current_drawdown_usd || 0).toFixed(2)} / 限额 $${(cb.max_loss_usd || 500).toFixed(2)}，风控与影子撮合通道正常`;
  }

  // Multidimensional metrics (strategy and symbol breakdown)
  const byStrategy = report.by_strategy || {};
  const bySymbol = report.by_symbol || {};
  const multidimRows = [];
  for (const [strat, m] of Object.entries(byStrategy)) {
    if (!m.samples) continue;
    multidimRows.push(`
      <div class="history-row">
        <div class="history-main">
          <span class="history-line">
            <strong>策略 · ${escapeHtml(strat)}</strong>
            <b class="history-status ${m.expectancy_r >= 0 ? 'submitted' : 'rejected'}">期望 ${formatR(m.expectancy_r)}</b>
          </span>
          <span class="history-detail">样本 ${m.samples} · 胜率 ${formatPercent(m.win_rate)} · 总盈亏 $${(m.total_pnl_usd || 0).toFixed(2)} · 平均持仓 ${m.avg_hold_bars?.toFixed(1) || 0} 根</span>
        </div>
      </div>`);
  }
  for (const [sym, m] of Object.entries(bySymbol)) {
    if (!m.samples) continue;
    multidimRows.push(`
      <div class="history-row">
        <div class="history-main">
          <span class="history-line">
            <strong>品种 · ${escapeHtml(sym)}</strong>
            <b class="history-status ${m.expectancy_r >= 0 ? 'submitted' : 'rejected'}">期望 ${formatR(m.expectancy_r)}</b>
          </span>
          <span class="history-detail">样本 ${m.samples} · 胜率 ${formatPercent(m.win_rate)} · 总盈亏 $${(m.total_pnl_usd || 0).toFixed(2)} · 平均持仓 ${m.avg_hold_bars?.toFixed(1) || 0} 根</span>
        </div>
      </div>`);
  }
  if ($("multidimMetrics")) {
    $("multidimMetrics").innerHTML = multidimRows.length ? multidimRows.join("") : `<div class="history-state">暂无多维切片统计数据</div>`;
  }
  if ($("multidimMetricsCount")) {
    $("multidimMetricsCount").textContent = `${multidimRows.length} 项切片`;
  }

  $("learningPromptVersion").textContent = prompt.version || "—";
  $("learningPromptSource").textContent = prompt.source === "artifact" ? "版本化 artifact" : "内置回退";
  $("learningPromptHash").textContent = prompt.hash || "—";
  $("learningReadExperience").textContent = report.read_experience ? "已开启" : "关闭（默认）";

  const enabled = !!report.enabled;
  const badge = $("learningEnabledBadge");
  badge.textContent = enabled ? "已开启" : "已关闭";
  badge.classList.toggle("inactive", !enabled);

  const runtime = report.runtime || {};
  const notes = [];
  if (!enabled) notes.push("学习闭环未启用，需将 LEARNING_ENABLED 设为 true");
  if (runtime.last_reconcile_ms) notes.push(`上次对账 ${formatHistoryTime(runtime.last_reconcile_ms)}`);
  notes.push(`成交 ${counts.filled ?? 0} 笔 · 已写入经验 ${counts.qualified ?? 0} 笔`);
  if (runtime.last_error) notes.push(`最近错误：${runtime.last_error}`);
  $("learningRuntimeNote").textContent = notes.join(" · ");
  $("learningPromptHint").textContent = enabled ? "对账与经验写入进行中" : "未启用";

  const versions = Array.isArray(report.versions) ? report.versions : [];
  $("learningVersionCount").textContent = `${versions.length} 个`;
  $("learningVersions").innerHTML = versions.length ? versions.map((entry) => {
    const metrics = entry.metrics || {};
    const verdict = entry.verdict;
    const isActive = entry.version === prompt.version;
    const statusText = isActive ? "当前启用" : (verdict ? (verdict.accepted ? "可发布" : "未通过") : "无对比数据");
    const statusClass = (isActive || (verdict && verdict.accepted)) ? "submitted" : "rejected";
    const detail = [
      `样本 ${metrics.samples ?? 0}`,
      `期望 ${formatR(metrics.expectancy_r)}`,
      `胜率 ${metrics.samples ? formatPercent(metrics.win_rate) : "—"}`,
      `期望差 ${verdict ? formatR(verdict.expectancy_delta_r) : "—"}`,
    ].join(" · ");
    const reason = (verdict && !verdict.accepted && Array.isArray(verdict.reasons) && verdict.reasons.length)
      ? `<span class="history-error">${escapeHtml(verdict.reasons.join("；"))}</span>`
      : "";
    const action = isActive ? ""
      : `<button class="small-action" type="button" data-action="activate-prompt" data-version="${escapeHtml(entry.version)}">启用</button>`;
    return `
      <div class="history-row">
        <div class="history-main">
          <span class="history-line">
            <strong>${escapeHtml(entry.version)}</strong>
            <b class="history-status ${statusClass}">${statusText}</b>
          </span>
          <span class="history-detail">${escapeHtml(detail)}</span>
          ${reason}
        </div>
        ${action}
      </div>`;
  }).join("") : `<div class="history-state">尚无按版本统计的结果</div>`;

  $("learningOutcomeCount").textContent = `${outcomes.length} 条`;
  $("learningOutcomes").innerHTML = outcomes.length ? outcomes.map((outcome) => {
    const direction = directionMeta(outcome.side === "short" ? "做空" : "做多");
    const statusClass = Number(outcome.r_multiple) > 0 ? "submitted" : "rejected";
    const detail = [
      formatHistoryTime(outcome.resolved_ms || outcome.created_ms),
      outcome.strategy_id || "—",
      outcome.prompt_version || "—",
      outcome.timeframe || "—",
      `持仓 ${outcome.hold_bars ?? 0} 根`,
      `MFE ${formatR(outcome.mfe_r)}`,
      `MAE ${formatR(outcome.mae_r)}`,
      outcome.pnl_source === "model_estimate" ? "盈亏=模型估算" : "盈亏=成交对账",
    ].join(" · ");
    const note = outcome.qualified ? "" : `<span class="history-error">未入库：${escapeHtml(outcome.qualification_reason || "")}</span>`;
    return `
      <div class="history-row">
        <div class="history-main">
          <span class="history-line">
            <strong>${escapeHtml(outcome.symbol || "—")}</strong>
            <span class="history-direction ${direction.className}">${direction.label}</span>
            <b class="history-status ${statusClass}">${formatR(outcome.r_multiple)}</b>
          </span>
          <span class="history-detail">${escapeHtml(detail)}</span>
          ${note}
        </div>
        <button class="small-action" type="button" data-action="solidify-episode" data-signal="${escapeHtml(outcome.signal_id)}" title="将已结算样本固化为离线基准切片">固化切片</button>
      </div>`;
  }).join("") : `<div class="history-state">暂无已结算结果</div>`;

  const cycles = (report.experience && Array.isArray(report.experience.cycles)) ? report.experience.cycles : [];
  $("learningExperienceCount").textContent = `${cycles.length} 类`;
  $("learningExperience").innerHTML = cycles.length ? cycles.map((cycle) => `
      <div class="history-row">
        <div class="history-main">
          <span class="history-line"><strong>${escapeHtml(cycle.cycle_position)}</strong></span>
          <span class="history-detail">成功 ${cycle.success_cases} 条 · 失败 ${cycle.failure_cases} 条</span>
        </div>
      </div>`).join("") : `<div class="history-state">经验库为空</div>`;
}

async function loadLearning() {
  try {
    learningReport = await api("/api/learning/report");
    learningOutcomes = await api("/api/learning/outcomes?limit=30");
    renderLearning();
    await loadBenchmarkEpisodes();
  } catch (error) {
    $("learningState").hidden = false;
    $("learningState").textContent = error.message;
  }
}

async function loadBenchmarkEpisodes() {
  const container = $("benchmarkEpisodes");
  if (!container) return;
  try {
    const episodes = await api("/api/learning/benchmark_episodes");
    const countEl = $("benchmarkEpisodeCount");
    if (countEl) countEl.textContent = `${episodes.length} 个`;
    if (!episodes.length) {
      container.innerHTML = `<div class="history-state">暂无离线基准切片</div>`;
      return;
    }
    container.innerHTML = episodes.map(ep => {
      const isLong = ep.expected_action?.includes("LONG") || ep.expected_action?.includes("做多");
      const dirCls = isLong ? "long" : (ep.expected_action?.includes("SHORT") || ep.expected_action?.includes("做空") ? "short" : "neutral");
      return `
        <div class="history-row">
          <div class="history-main">
            <span class="history-line">
              <strong>${escapeHtml(ep.episode_id || "切片")}</strong>
              <span class="history-direction ${dirCls}">${escapeHtml(ep.expected_action || "—")}</span>
              <b class="history-status submitted">基准 ${formatR(ep.benchmark_r)}</b>
            </span>
            <span class="history-detail">${escapeHtml(ep.symbol || "")} · ${escapeHtml(ep.market_regime || ep.regime || "")} · ${(ep.kline_data?.length || ep.bars?.length || 0)} 根K线 · ${escapeHtml(ep.description || "")}</span>
          </div>
        </div>`;
    }).join("");
  } catch (err) {
    container.innerHTML = `<div class="history-state">读取切片失败: ${escapeHtml(err.message)}</div>`;
  }
}

async function reconcileLearning() {
  try {
    const report = await api("/api/learning/reconcile", { method: "POST" });
    toast(`对账完成：已解决 ${report.resolved ?? 0}，仍持有 ${report.still_open ?? 0}，写入经验 ${report.experiences_written ?? 0}`);
    await loadLearning();
  } catch (error) {
    toast(error.message);
  }
}

async function activatePromptVersion(version, force = false) {
  return api("/api/learning/prompts/activate", {
    method: "POST",
    body: JSON.stringify({ version, force }),
  });
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
  const sysName = systemLabel(currentTradingSystem);
  $("analysisState").textContent = `正在获取行情并运行【${sysName}】两阶段 AI…`;
  try {
    const result = await api("/api/analyze", {
      method: "POST",
      body: JSON.stringify({
        inst_id: $("symbol").value.trim(),
        timeframe: $("timeframe").value,
        bar_count: 100,
        execute: $("executeAfterAnalysis").checked,
        trading_system: currentTradingSystem,
      }),
    });
    renderDecision(result);
    loadDecisionHistory(true);
    if ($("executeAfterAnalysis").checked) loadTradeHistory(true);
    $("analysisState").textContent = result.exception ? `失败：${result.exception.message}` : `【${sysName}】分析完成`;
    toast(result.execution?.submitted ? "订单已提交" : `【${sysName}】分析完成`);
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
  const typeLabels = { limit: "限价", market: "市价", trigger: "触发", oco: "TP/SL", conditional: "条件" };
  $("ordersBody").innerHTML = rows.length
    ? rows.map((row) => {
      const instId = row.instId || row.instrument || "";
      const side = row.side || row.direction || "buy";
      const isSell = String(side).toLowerCase() === "sell";
      const direction = isSell ? "short" : "long";
      const directionLabel = isSell ? "卖" : "买";
      const ordType = row.ordType || row.order_type || "limit";
      const state = row.state || "挂单中";
      const size = row.sz != null ? row.sz : row.size;
      const filled = row.accFillSz != null ? row.accFillSz : (row.filled_size || 0);
      const price = row.px || row.price || row.triggerPx || "—";
      const ordId = row.ordId || row.order_id || "";
      const algoId = row.algoId || row.algo_id || "";

      let priceDisplay = fmt(price);
      if (row.tpTriggerPx || row.slTriggerPx) {
        priceDisplay = `TP ${fmt(row.tpTriggerPx || '—')}<small>SL ${fmt(row.slTriggerPx || '—')}</small>`;
      }

      return `
        <tr>
          <td><strong>${escapeHtml(instId)}</strong><span class="side-label ${direction}">${directionLabel}</span></td>
          <td>${escapeHtml(typeLabels[ordType] || ordType)}<small>${escapeHtml(state)}</small></td>
          <td>${fmt(size)}<small>已成 ${fmt(filled)}</small></td>
          <td>${priceDisplay}</td>
          <td><button class="table-action-button" style="padding:2px 8px;font-size:11px;" data-action="cancel-order" data-inst-id="${escapeHtml(instId)}" data-ord-id="${escapeHtml(ordId)}" data-algo-id="${escapeHtml(algoId)}">撤单</button></td>
        </tr>`;
    }).join("")
    : '<tr class="empty-row"><td colspan="5">暂无挂单</td></tr>';
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
  $("equityRange").textContent = equityPoints.length ? `本次运行 ${equityPoints.length} 个观测点` : "—";
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
  if (button.dataset.tab === "learning") loadLearning();
  if (button.dataset.tab === "backtest" && currentBacktestReport) {
    requestAnimationFrame(() => drawBacktestDualChart(currentBacktestReport));
  }
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
$("learningRefresh").addEventListener("click", () => loadLearning());
$("learningReconcile").addEventListener("click", reconcileLearning);
$("learningRollback").addEventListener("click", async () => {
  if (!window.confirm("确定回退到上一个提示词版本？")) return;
  try {
    const result = await api("/api/learning/prompts/rollback", { method: "POST" });
    toast(`已回退到 ${result.active}`);
    await loadLearning();
  } catch (error) {
    toast(error.message);
  }
});
$("learningVersions").addEventListener("click", async (event) => {
  const button = event.target.closest('[data-action="activate-prompt"]');
  if (!button) return;
  const version = button.dataset.version;
  if (!window.confirm(`确定启用提示词版本 ${version}？启用后的新分析将使用该版本。`)) return;
  try {
    await activatePromptVersion(version, false);
    toast(`已启用 ${version}`);
    await loadLearning();
  } catch (error) {
    if (!window.confirm(`${error.message}\n\n是否跳过统计校验、强制启用？`)) return;
    try {
      await activatePromptVersion(version, true);
      toast(`已强制启用 ${version}`);
      await loadLearning();
    } catch (forceError) {
      toast(forceError.message);
    }
  }
});
$("toggleShadowBtn").addEventListener("click", async () => {
  try {
    const res = await api("/api/learning/shadow_trading/toggle", { method: "POST" });
    toast(res.note || "影子交易模式已切换");
    await loadStatus();
    await loadLearning();
  } catch (err) {
    toast(`切换影子模式失败: ${err.message}`);
  }
});
$("resetCircuitBreakerBtn").addEventListener("click", async () => {
  try {
    const res = await api("/api/learning/circuit_breaker/reset", { method: "POST" });
    toast(res.note || "风控熔断器已手动重置");
    await loadStatus();
    await loadLearning();
  } catch (err) {
    toast(`重置失败: ${err.message}`);
  }
});
$("learningProposeBtn").addEventListener("click", async () => {
  const btn = $("learningProposeBtn");
  btn.disabled = true;
  try {
    toast("正在对账样本归因并生成反思突变候选版本…");
    const res = await api("/api/learning/propose", { method: "POST" });
    toast(`已生成反思候选版本 ${res.published_version}（覆盖 ${res.attributions_count || 0} 条归因，解决 ${res.addressed_modes?.length || 0} 种模式）`);
    await loadLearning();
  } catch (err) {
    toast(`反思突变失败: ${err.message}`);
  } finally {
    btn.disabled = false;
  }
});
$("learningOutcomes").addEventListener("click", async (event) => {
  const button = event.target.closest('[data-action="solidify-episode"]');
  if (!button) return;
  const signalId = button.dataset.signal;
  try {
    button.disabled = true;
    const res = await api("/api/learning/solidify_episode", {
      method: "POST",
      body: JSON.stringify({ signal_id: signalId }),
    });
    toast(`实盘样本已固化为离线基准切片: ${signalId}`);
    await loadLearning();
  } catch (err) {
    toast(`固化切片失败: ${err.message}`);
  } finally {
    button.disabled = false;
  }
});
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
$("ordersBody").addEventListener("click", async (event) => {
  const button = event.target.closest("button[data-action='cancel-order']");
  if (!button) return;
  const instId = button.dataset.instId;
  const ordId = button.dataset.ordId;
  const algoId = button.dataset.algoId;
  if (!confirm(`确认撤销 ${instId} 挂单？`)) return;
  try {
    await api("/api/trade/cancel", {
      method: "POST",
      body: JSON.stringify({ inst_id: instId, ord_id: ordId || undefined, algo_id: algoId || undefined }),
    });
    toast(`已提交撤销 ${instId} 挂单`);
    await loadAccount(true);
  } catch (err) {
    toast(`撤单失败: ${err.message}`);
  }
});
$("cancelAllOrdersBtn").addEventListener("click", async () => {
  if (!confirm("确认一键撤销所有当前挂单与条件委托？")) return;
  try {
    const res = await api("/api/trade/cancel_all", {
      method: "POST",
      body: JSON.stringify({}),
    });
    toast(`已成功撤销 ${res.cancelled_count || 0} 个挂单`);
    await loadAccount(true);
  } catch (err) {
    toast(`一键撤单失败: ${err.message}`);
  }
});
$("tradingSystemSelect").addEventListener("change", async (e) => {
  currentTradingSystem = e.target.value;
  try {
    localStorage.setItem("okx_trading_system", currentTradingSystem);
  } catch {}
  updateSystemUI();
  drawChart();
  const name = systemLabel(currentTradingSystem);
  toast(`已切换交易系统为: ${name}`);
  try {
    const updatedStatus = await api("/api/trading_system", {
      method: "POST",
      body: JSON.stringify({ trading_system: currentTradingSystem }),
    });
    statusData = updatedStatus;
    renderAutomationStatus(updatedStatus);
  } catch (err) {
    console.warn("同步交易系统到后端失败:", err);
  }
});
$("automationSwitch").addEventListener("change", toggleAutomation);
setupChartInteraction();
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
    if ($("cfgTradingSystem")) $("cfgTradingSystem").value = canonicalSystem(cfg.trading_system || currentTradingSystem || "2pa_trend");
    if (cfg.okx_base_url) $("cfgOkxBaseUrl").value = cfg.okx_base_url;
    $("cfgOkxDemoTrading").value = String(cfg.okx_demo_trading !== false);
    if ($("cfgOkxAutoOrderSizing")) $("cfgOkxAutoOrderSizing").checked = cfg.okx_auto_order_sizing !== false;
    if ($("cfgOkxRiskPercent")) $("cfgOkxRiskPercent").value = cfg.okx_risk_percent || 2.0;
    if ($("cfgOkxMaxMarginPercent")) $("cfgOkxMaxMarginPercent").value = cfg.okx_max_margin_percent || 25.0;
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
      trading_system: $("cfgTradingSystem") ? $("cfgTradingSystem").value : currentTradingSystem,
      okx_api_key: $("cfgOkxApiKey").value.trim(),
      okx_secret_key: $("cfgOkxSecretKey").value.trim(),
      okx_passphrase: $("cfgOkxPassphrase").value.trim(),
      okx_base_url: "https://www.okx.com",
      okx_demo_trading: $("cfgOkxDemoTrading").value === "true",
      okx_auto_order_sizing: $("cfgOkxAutoOrderSizing") ? $("cfgOkxAutoOrderSizing").checked : true,
      okx_risk_percent: $("cfgOkxRiskPercent") ? Number($("cfgOkxRiskPercent").value) : 2.0,
      okx_max_margin_percent: $("cfgOkxMaxMarginPercent") ? Number($("cfgOkxMaxMarginPercent").value) : 25.0,
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

// --- 📊 策略全流程回测 (Backtest) 模块 ---
let currentBacktestReport = null;
let currentBacktestFilter = "all";
let backtestPollingTimer = null;

function renderBacktestKPIs(metrics) {
  if (!metrics) return;
  $("btKpiSection").hidden = false;

  const netPnl = metrics.net_profit || 0;
  const netPnlPct = metrics.net_profit_pct || 0;
  $("btKpiNetProfit").textContent = `${netPnl >= 0 ? "+" : ""}${fmtMoney(netPnl)} USDT`;
  $("btKpiNetProfit").className = netPnl >= 0 ? "positive" : "negative";
  $("btKpiNetProfitPct").textContent = `${netPnlPct >= 0 ? "+" : ""}${fmt(netPnlPct, 2)}%`;
  $("btKpiNetProfitPct").className = netPnlPct >= 0 ? "bt-kpi-sub positive" : "bt-kpi-sub negative";

  const maxDd = metrics.max_drawdown_amount || 0;
  const maxDdPct = metrics.max_drawdown_pct || 0;
  $("btKpiMaxDd").textContent = `-${fmtMoney(maxDd)} USDT`;
  $("btKpiMaxDd").className = "negative";
  $("btKpiMaxDdPct").textContent = `最大回撤: -${fmt(maxDdPct, 2)}%`;

  $("btKpiSharpe").textContent = fmt(metrics.sharpe_ratio, 2);
  $("btKpiSortino").textContent = `索提诺: ${fmt(metrics.sortino_ratio, 2)}`;

  $("btKpiWinRate").textContent = `${fmt(metrics.win_rate, 1)}%`;
  $("btKpiTradeCounts").textContent = `${metrics.winning_trades} 胜 / ${metrics.losing_trades} 负 (共 ${metrics.total_trades} 笔)`;

  const expR = metrics.expectancy_r || 0;
  $("btKpiExpectancyR").textContent = `${expR >= 0 ? "+" : ""}${fmt(expR, 2)} R`;
  $("btKpiExpectancyR").className = expR >= 0 ? "positive" : "negative";
  $("btKpiProfitFactor").textContent = `盈亏比: ${fmt(metrics.profit_factor, 2)}`;

  const totalBars = metrics.total_bars_processed || 1;
  const gatedBars = metrics.gated_bars_skipped || 0;
  const gatedPct = ((gatedBars / totalBars) * 100) || 0;
  $("btKpiGated").textContent = `${fmt(gatedPct, 1)}%`;
  $("btKpiCacheHits").textContent = `短路 ${gatedBars} 根 · 命中缓存 ${metrics.cache_hits || 0}`;
}

function drawBacktestDualChart(report) {
  const canvas = $("btDualCanvas");
  if (!canvas) return;
  const wrap = canvas.parentElement;
  const placeholder = $("btChartEmpty");
  const width = wrap.clientWidth || 380;
  const height = wrap.clientHeight || 240;
  const dpr = window.devicePixelRatio || 1;

  canvas.width = width * dpr;
  canvas.height = height * dpr;
  const ctx = canvas.getContext("2d");
  ctx.scale(dpr, dpr);
  ctx.clearRect(0, 0, width, height);

  const points = report?.equity_curve || [];
  if (points.length < 2) {
    if (placeholder) placeholder.style.display = "grid";
    return;
  }
  if (placeholder) placeholder.style.display = "none";

  const padding = { left: 10, right: 65, top: 12, bottom: 20 };
  const plotWidth = width - padding.left - padding.right;

  // Track 1 (top 65%): Equity Curve
  // Track 2 (bottom 35%): Underwater Drawdown %
  const splitY = padding.top + (height - padding.top - padding.bottom) * 0.65;
  const eqHeight = splitY - padding.top - 8;
  const ddTop = splitY + 8;
  const ddHeight = height - padding.bottom - ddTop;

  // 1. Draw track 1: Equity Curve
  const equities = points.map(p => p.equity);
  const minEq = Math.min(...equities);
  const maxEq = Math.max(...equities);
  const eqPad = Math.max((maxEq - minEq) * 0.08, 1);
  const lowEq = minEq - eqPad;
  const highEq = maxEq + eqPad;
  const eqRange = highEq - lowEq || 1;

  const getX = (idx) => padding.left + (idx * plotWidth) / (points.length - 1);
  const getEqY = (val) => padding.top + ((highEq - val) / eqRange) * eqHeight;

  // Grid lines for Equity
  ctx.strokeStyle = "#1d2326";
  ctx.lineWidth = 1;
  ctx.fillStyle = "#6e797d";
  ctx.font = "10px Segoe UI, sans-serif";

  [highEq - eqPad, (highEq + lowEq) / 2, lowEq + eqPad].forEach((val) => {
    const y = getEqY(val);
    ctx.beginPath();
    ctx.moveTo(padding.left, y);
    ctx.lineTo(width - padding.right, y);
    ctx.stroke();
    ctx.fillText(fmtMoney(val), width - padding.right + 6, y + 3);
  });

  // Draw Equity Path
  ctx.beginPath();
  points.forEach((p, idx) => {
    const x = getX(idx);
    const y = getEqY(p.equity);
    if (idx === 0) ctx.moveTo(x, y);
    else ctx.lineTo(x, y);
  });
  ctx.lineWidth = 2;
  const rising = equities.at(-1) >= equities[0];
  ctx.strokeStyle = rising ? "#1fc48d" : "#f05b67";
  ctx.stroke();

  // Equity Gradient Fill
  ctx.lineTo(getX(points.length - 1), splitY - 8);
  ctx.lineTo(getX(0), splitY - 8);
  ctx.closePath();
  const eqGrad = ctx.createLinearGradient(0, padding.top, 0, splitY);
  if (rising) {
    eqGrad.addColorStop(0, "rgba(31, 196, 141, 0.22)");
    eqGrad.addColorStop(1, "rgba(31, 196, 141, 0.0)");
  } else {
    eqGrad.addColorStop(0, "rgba(240, 91, 103, 0.22)");
    eqGrad.addColorStop(1, "rgba(240, 91, 103, 0.0)");
  }
  ctx.fillStyle = eqGrad;
  ctx.fill();

  // 2. Track separator line
  ctx.beginPath();
  ctx.strokeStyle = "#273034";
  ctx.moveTo(padding.left, splitY);
  ctx.lineTo(width - padding.right, splitY);
  ctx.stroke();

  // 3. Draw track 2: Underwater Drawdown %
  const drawdowns = points.map(p => p.drawdown_pct || 0);
  const maxDd = Math.max(...drawdowns, 5.0); // at least 5% range
  const getDdY = (dd) => ddTop + (dd / maxDd) * ddHeight;

  // Drawdown labels
  ctx.fillStyle = "#6e797d";
  ctx.fillText("0.0%", width - padding.right + 6, ddTop + 3);
  ctx.fillText(`-${fmt(maxDd, 1)}%`, width - padding.right + 6, ddTop + ddHeight + 3);

  ctx.beginPath();
  points.forEach((p, idx) => {
    const x = getX(idx);
    const y = getDdY(p.drawdown_pct || 0);
    if (idx === 0) ctx.moveTo(x, y);
    else ctx.lineTo(x, y);
  });
  ctx.lineWidth = 1.5;
  ctx.strokeStyle = "#f05b67";
  ctx.stroke();

  // Drawdown Gradient Fill
  ctx.lineTo(getX(points.length - 1), ddTop);
  ctx.lineTo(getX(0), ddTop);
  ctx.closePath();
  const ddGrad = ctx.createLinearGradient(0, ddTop, 0, ddTop + ddHeight);
  ddGrad.addColorStop(0, "rgba(240, 91, 103, 0.05)");
  ddGrad.addColorStop(1, "rgba(240, 91, 103, 0.35)");
  ctx.fillStyle = ddGrad;
  ctx.fill();
}

function toggleBacktestTradeDetail(idx) {
  const row = $(`btTradeRow_${idx}`);
  const detailRow = $(`btTradeDetail_${idx}`);
  if (!detailRow) return;
  const willShow = detailRow.hidden;
  detailRow.hidden = !willShow;
  if (row) row.classList.toggle("expanded", willShow);
}
window.toggleBacktestTradeDetail = toggleBacktestTradeDetail;

function renderBacktestTrades(trades = []) {
  const body = $("btTradesBody");
  if (!body) return;
  $("btTradesSection").hidden = false;
  $("btTradeCount").textContent = `${trades.length} 笔`;

  const filtered = trades.filter(t => {
    if (currentBacktestFilter === "win") return t.net_pnl > 0;
    if (currentBacktestFilter === "loss") return t.net_pnl < 0;
    return true;
  });

  if (!filtered.length) {
    body.innerHTML = `<tr class="empty-row"><td colspan="6">无符合条件的成交记录</td></tr>`;
    return;
  }

  body.innerHTML = filtered.map((t, idx) => {
    const isLong = t.direction.includes("多");
    const isWin = t.net_pnl > 0;
    const pnlClass = isWin ? "positive" : (t.net_pnl < 0 ? "negative" : "");
    const dateStr = t.entry_time_ms ? new Date(t.entry_time_ms).toLocaleString("zh-CN", { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" }) : "—";
    const exitDateStr = t.exit_time_ms ? new Date(t.exit_time_ms).toLocaleString("zh-CN", { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" }) : "—";
    const reasonLabel = ({"take_profit":"止盈","stop_loss":"止损","expired":"持仓超时","liquidated":"强平","backtest_end":"回测结束"})[t.exit_reason] || t.exit_reason;

    return `
      <tr id="btTradeRow_${idx}" class="bt-trade-row" onclick="toggleBacktestTradeDetail(${idx})" title="点击展开/收起交易明细">
        <td>
          <span class="expand-icon">▶</span>
          <strong>#${idx + 1} ${dateStr}</strong>
          <small>${escapeHtml(t.order_type)}</small>
        </td>
        <td>
          <span class="side-label ${isLong ? "long" : "short"}">${escapeHtml(t.direction)}</span>
        </td>
        <td>
          <strong>${fmtMoney(t.entry_price)}</strong>
          <small>平: ${fmtMoney(t.exit_price)}</small>
        </td>
        <td>
          <strong>${t.contracts} 张</strong>
          <small>${t.hold_bars} 根K线</small>
        </td>
        <td class="${pnlClass}">
          <strong>${t.net_pnl >= 0 ? "+" : ""}${fmtMoney(t.net_pnl)}</strong>
          <small>${t.pnl_percent >= 0 ? "+" : ""}${fmt(t.pnl_percent, 1)}%</small>
        </td>
        <td>
          <strong class="${pnlClass}">${t.pnl_r >= 0 ? "+" : ""}${fmt(t.pnl_r, 2)}R</strong>
          <small>${escapeHtml(reasonLabel)}</small>
        </td>
      </tr>
      <tr id="btTradeDetail_${idx}" class="bt-trade-detail-row" hidden>
        <td colspan="6">
          <div class="bt-trade-detail-content">
            <div class="bt-detail-grid">
              <div class="bt-detail-item">
                <span>交易编号 / 信号ID</span>
                <strong>${escapeHtml(t.trade_id)} · ${escapeHtml(t.signal_id)}</strong>
              </div>
              <div class="bt-detail-item">
                <span>策略标识</span>
                <strong>${escapeHtml(t.strategy_id)}</strong>
              </div>
              <div class="bt-detail-item">
                <span>出场原因</span>
                <strong class="${pnlClass}">${escapeHtml(reasonLabel)}</strong>
              </div>
              <div class="bt-detail-item">
                <span>开仓时间 / 平仓时间</span>
                <strong>${dateStr} → ${exitDateStr}</strong>
              </div>
              <div class="bt-detail-item">
                <span>止损价 / 止盈价</span>
                <strong>止损 ${fmtMoney(t.stop_loss)} · 止盈 ${fmtMoney(t.take_profit)}</strong>
              </div>
              <div class="bt-detail-item">
                <span>成交名义价值</span>
                <strong>${fmtMoney(t.notional_usdt)} USDT (${t.contracts} 张)</strong>
              </div>
              <div class="bt-detail-item">
                <span>毛盈亏 / 净盈亏</span>
                <strong class="${pnlClass}">${fmtMoney(t.gross_pnl)} / ${fmtMoney(t.net_pnl)} USDT (${fmt(t.pnl_r, 2)}R)</strong>
              </div>
              <div class="bt-detail-item">
                <span>手续费 / 模拟滑点</span>
                <strong>手续费 -${fmtMoney(t.fees)} USDT · 滑点 -${fmtMoney(t.slippage)} USDT</strong>
              </div>
              <div class="bt-detail-item">
                <span>最大顺行 / 逆行 (MFE / MAE)</span>
                <strong>顺行 +${fmt(t.mfe_r, 2)}R · 逆行 -${fmt(t.mae_r, 2)}R</strong>
              </div>
            </div>
            ${t.notes ? `<div class="bt-detail-notes"><strong>执行细节：</strong>${escapeHtml(t.notes)}</div>` : ""}
          </div>
        </td>
      </tr>`;
  }).join("");
}

async function runBacktest() {
  const btn = $("btRunBtn");
  const statusBox = $("btStatusBox");
  const fill = $("btProgressBarFill");
  const msg = $("btStatusMsg");
  const stateText = $("btStatusState");
  const pctText = $("btStatusPercent");

  btn.disabled = true;
  statusBox.hidden = false;
  fill.style.width = "5%";
  pctText.textContent = "5%";
  stateText.textContent = "正在提交回测任务...";
  msg.textContent = "正在初始化引擎与配置...";

  if (backtestPollingTimer) {
    clearInterval(backtestPollingTimer);
    backtestPollingTimer = null;
  }

  const startVal = $("btStartDate")?.value;
  const endVal = $("btEndDate")?.value;
  const startTimeMs = startVal ? new Date(startVal).getTime() : null;
  const endTimeMs = endVal ? new Date(endVal).getTime() : null;

  const config = {
    strategy_id: $("btStrategy").value,
    symbol: ($("btSymbol").value.trim() || "BTC-USDT-SWAP").toUpperCase(),
    timeframe: $("btTimeframe").value,
    start_time_ms: startTimeMs && !isNaN(startTimeMs) ? startTimeMs : undefined,
    end_time_ms: endTimeMs && !isNaN(endTimeMs) ? endTimeMs : undefined,
    max_bars: Math.max(100, Math.min(3000, Number($("btMaxBars").value) || 1000)),
    initial_capital: Math.max(100, Number($("btInitialCapital").value) || 10000),
    risk_percent: Math.max(0.1, Number($("btRiskPct").value) || 1.0),
    leverage: Math.max(1, Number($("btLeverage").value) || 5.0),
    max_margin_percent: Math.max(5, Number($("btMaxMarginPct").value) || 50.0),
    use_mechanical_exit: $("btUseMechanicalExit").checked,
    use_cache: $("btUseCache").checked,
    allow_llm_calls: $("btAllowLlm").checked,
    data_source: $("btDataSource") ? $("btDataSource").value : "okx_api",
  };

  try {
    const res = await api("/api/backtest/run", {
      method: "POST",
      body: JSON.stringify(config),
    });

    const jobId = res.job_id;
    if (!jobId) throw new Error(res.error || "未能获取回测任务编号");

    stateText.textContent = `任务进行中 [${jobId}]`;

    backtestPollingTimer = setInterval(async () => {
      try {
        const st = await api(`/api/backtest/status/${jobId}`);
        const pct = Math.max(5, Math.min(100, Math.round(st.progress_pct || 0)));
        fill.style.width = `${pct}%`;
        pctText.textContent = `${pct}%`;
        msg.textContent = st.message || `处理中... ${st.current_bar}/${st.total_bars}`;

        if (st.status === "completed") {
          clearInterval(backtestPollingTimer);
          backtestPollingTimer = null;
          fill.style.width = "100%";
          pctText.textContent = "100%";
          stateText.textContent = "回测完成！正在生成深度分析报告...";

          const report = await api(`/api/backtest/report/${jobId}`);
          currentBacktestReport = report;

          renderBacktestKPIs(report.metrics);
          $("btChartSection").hidden = false;
          drawBacktestDualChart(report);
          renderBacktestTrades(report.trades);

          btn.disabled = false;
          statusBox.hidden = true;
          toast(`回测完成！共执行 ${report.metrics.total_trades} 笔交易，净收益 ${fmtMoney(report.metrics.net_profit)} USDT`);
        } else if (st.status === "failed") {
          clearInterval(backtestPollingTimer);
          backtestPollingTimer = null;
          btn.disabled = false;
          stateText.textContent = "回测失败";
          msg.textContent = st.error || st.message || "未知错误";
          toast(`回测失败: ${msg.textContent}`);
        }
      } catch (pollErr) {
        // Continue polling unless stopped
      }
    }, 500);

  } catch (err) {
    btn.disabled = false;
    stateText.textContent = "提交失败";
    msg.textContent = err.message;
    toast(`提交回测失败: ${err.message}`);
  }
}

// 绑定回测交互事件
$("btRunBtn").addEventListener("click", runBacktest);

$("btPreset7d").addEventListener("click", () => {
  $("btTimeframe").value = "15m";
  $("btMaxBars").value = "672";
  if ($("btStartDate")) $("btStartDate").value = "";
  if ($("btEndDate")) $("btEndDate").value = "";
  toast("已设为 7 天预设 (672 根 15m K 线)");
});

$("btPreset30d").addEventListener("click", () => {
  $("btTimeframe").value = "15m";
  $("btMaxBars").value = "2880";
  if ($("btStartDate")) $("btStartDate").value = "";
  if ($("btEndDate")) $("btEndDate").value = "";
  toast("已设为 30 天预设 (2880 根 15m K 线)");
});

$("btPreset90d").addEventListener("click", () => {
  $("btTimeframe").value = "1h";
  $("btMaxBars").value = "2160";
  if ($("btStartDate")) $("btStartDate").value = "";
  if ($("btEndDate")) $("btEndDate").value = "";
  toast("已设为 90 天预设 (2160 根 1h K 线)");
});

document.querySelectorAll(".bt-table-filters button").forEach(btn => {
  btn.addEventListener("click", () => {
    document.querySelectorAll(".bt-table-filters button").forEach(b => b.classList.remove("active"));
    btn.classList.add("active");
    currentBacktestFilter = btn.dataset.filter;
    if (currentBacktestReport) {
      renderBacktestTrades(currentBacktestReport.trades);
    }
  });
});

window.addEventListener("resize", () => {
  if (currentBacktestReport && $("backtestTab").classList.contains("active")) {
    drawBacktestDualChart(currentBacktestReport);
  }
});

// allSettled: a single failing loader must not skip the trading-system restore
// below, and must not surface as an unhandled rejection.
Promise.allSettled([loadStatus(), loadInstruments(), loadCandles(), loadDecisionHistory(), loadTradeHistory()])
  .then(async () => {
    try {
      const savedSys = localStorage.getItem("okx_trading_system");
      if (savedSys && savedSys !== statusData?.trading_system) {
        const res = await api("/api/trading_system", {
          method: "POST",
          body: JSON.stringify({ trading_system: savedSys }),
        });
        statusData = res;
        currentTradingSystem = canonicalSystem(res.trading_system || savedSys);
        updateSystemUI();
        renderAutomationStatus(res);
        drawChart();
      }
    } catch {}

    // 若尚未配置 AI 或未检测到 .env，自动弹出向导
    if (statusData && (!statusData.is_ai_configured || !statusData.has_env_file)) {
      openConfigModal();
    }
  })
  .catch((error) => toast(error.message));

setInterval(loadCandles, 30000);
setInterval(() => loadStatus().catch((error) => toast(error.message)), 15000);
setInterval(() => { loadDecisionHistory(); loadTradeHistory(); }, 30000);
setInterval(() => {
  if ($("accountTab").classList.contains("active")) loadAccount(true);
  if ($("contractTab").classList.contains("active")) loadContractSpecs($("calcSymbolInput").value.trim());
}, 60000);


