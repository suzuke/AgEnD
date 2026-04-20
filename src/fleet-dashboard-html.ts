/**
 * Activity / fleet dashboard HTML served by the daemon's health server.
 * Pure constant — extracted from fleet-manager.ts to keep that module under
 * a manageable size (P4.1).
 */
export const ACTIVITY_VIEWER_HTML = `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>AgEnD Activity Viewer</title>
<script src="https://cdn.jsdelivr.net/npm/mermaid@11/dist/mermaid.min.js"></script>
<style>
  * { margin: 0; padding: 0; box-sizing: border-box; }
  body { background: #0d1117; color: #c9d1d9; font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', monospace; }
  .header { padding: 16px 24px; border-bottom: 1px solid #21262d; display: flex; align-items: center; gap: 16px; flex-wrap: wrap; }
  .header h1 { font-size: 18px; color: #58a6ff; font-weight: 600; }
  .controls { display: flex; gap: 8px; align-items: center; }
  .controls select, .controls button { background: #21262d; color: #c9d1d9; border: 1px solid #30363d; border-radius: 6px; padding: 4px 10px; font-size: 13px; cursor: pointer; }
  .controls button.active { background: #1f6feb; border-color: #1f6feb; color: #fff; }
  .controls button:hover { border-color: #58a6ff; }
  .speed-group { display: flex; gap: 2px; }
  .speed-group button { border-radius: 0; }
  .speed-group button:first-child { border-radius: 6px 0 0 6px; }
  .speed-group button:last-child { border-radius: 0 6px 6px 0; }
  .status { font-size: 12px; color: #8b949e; margin-left: auto; }
  #diagram { padding: 24px; overflow-x: auto; }
  #diagram .mermaid { background: transparent; }
  #diagram svg { max-width: 100%; }
  .feed { padding: 12px 24px; max-height: 300px; overflow-y: auto; border-top: 1px solid #21262d; font-size: 13px; line-height: 1.8; }
  .feed-line { opacity: 0.6; }
  .feed-line.visible { opacity: 1; }
  .feed-line .time { color: #8b949e; }
  .feed-line .msg { color: #58a6ff; }
  .feed-line .tool { color: #d29922; }
  .feed-line .task { color: #3fb950; }
  /* Agent Board */
  .board { padding: 16px 24px; display: flex; gap: 12px; flex-wrap: wrap; border-bottom: 1px solid #21262d; }
  .card { background: #161b22; border: 1px solid #30363d; border-radius: 8px; padding: 12px 14px; min-width: 200px; flex: 1; max-width: 280px; transition: border-color 0.3s; }
  .card.flash { border-color: #58a6ff; }
  .card-header { display: flex; align-items: center; gap: 6px; margin-bottom: 8px; }
  .card-header .dot { width: 8px; height: 8px; border-radius: 50%; flex-shrink: 0; }
  .card-header .dot.running { background: #3fb950; }
  .card-header .dot.stopped { background: #8b949e; }
  .card-header .dot.crashed { background: #f85149; }
  .card-header .name { font-weight: 600; font-size: 14px; }
  .card-row { font-size: 12px; color: #8b949e; line-height: 1.6; }
  .card-row span { color: #c9d1d9; }
  .card-task { font-size: 12px; color: #d29922; margin-top: 6px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .board-empty { font-size: 13px; color: #8b949e; padding: 8px 0; }
  .section-label { font-size: 11px; color: #484f58; text-transform: uppercase; letter-spacing: 1px; padding: 10px 24px 0; }
  .tabs { display: flex; gap: 0; padding: 0 24px; border-bottom: 1px solid #21262d; }
  .tab { padding: 8px 16px; font-size: 13px; color: #8b949e; cursor: pointer; border: none; border-bottom: 2px solid transparent; background: none; }
  .tab.active { color: #58a6ff; border-bottom-color: #58a6ff; }
  .tab:hover { color: #c9d1d9; }
  .view { display: none; }
  .view.active { display: block; }
  #graphCanvas { width: 100%; background: #0d1117; display: block; }
</style>
</head>
<body>
<div class="header">
  <h1>AgEnD Activity</h1>
  <div class="controls">
    <select id="range">
      <option value="1h">1h</option>
      <option value="2h" selected>2h</option>
      <option value="4h">4h</option>
      <option value="8h">8h</option>
      <option value="24h">24h</option>
    </select>
    <button id="btnLoad">Load</button>
    <button id="btnPlay">▶ Play</button>
    <button id="btnPause" style="display:none">⏸ Pause</button>
    <div class="speed-group">
      <button class="speed" data-speed="1">1x</button>
      <button class="speed active" data-speed="2">2x</button>
      <button class="speed" data-speed="5">5x</button>
      <button class="speed" data-speed="10">10x</button>
    </div>
  </div>
  <div class="status" id="status">Ready</div>
</div>
<div class="section-label">Agents</div>
<div class="board" id="board"><div class="board-empty">Loading...</div></div>
<div class="tabs">
  <button class="tab active" data-view="graph">Network Graph</button>
  <button class="tab" data-view="seq">Sequence Diagram</button>
</div>
<div id="viewGraph" class="view active"><canvas id="graphCanvas" height="400"></canvas></div>
<div id="viewSeq" class="view"><div id="diagram"><div class="mermaid" id="mermaidEl"></div></div></div>
<div class="feed" id="feed"></div>

<script>
mermaid.initialize({ startOnLoad: false, theme: 'dark', sequence: { mirrorActors: false, messageAlign: 'left' } });

let rows = [];
let speed = 2;
let playing = false;
let playTimeout = null;
let visibleCount = 0;

document.querySelectorAll('.speed').forEach(btn => {
  btn.addEventListener('click', () => {
    document.querySelectorAll('.speed').forEach(b => b.classList.remove('active'));
    btn.classList.add('active');
    speed = parseInt(btn.dataset.speed);
  });
});

document.getElementById('btnLoad').addEventListener('click', load);
document.getElementById('btnPlay').addEventListener('click', startReplay);
document.getElementById('btnPause').addEventListener('click', pauseReplay);

async function load() {
  const range = document.getElementById('range').value;
  document.getElementById('status').textContent = 'Loading...';
  try {
    const resp = await fetch('/api/activity?since=' + range + '&limit=500');
    rows = await resp.json();
    document.getElementById('status').textContent = rows.length + ' events loaded';
    visibleCount = rows.length;
    renderFull();
  } catch (e) {
    document.getElementById('status').textContent = 'Error: ' + e.message;
  }
}

function buildMermaid(entries) {
  const participants = new Set();
  entries.forEach(r => { participants.add(r.sender); if (r.receiver) participants.add(r.receiver); });
  const aliases = new Map();
  let idx = 0;
  participants.forEach(p => {
    const a = p.length > 12 ? String.fromCharCode(65 + idx++) : p;
    aliases.set(p, a);
  });

  let lines = ['sequenceDiagram'];
  aliases.forEach((a, p) => lines.push('    participant ' + a + ' as ' + p));

  entries.forEach(r => {
    const s = aliases.get(r.sender) || r.sender;
    const summary = (r.summary || '').replace(/"/g, "'").slice(0, 80);
    if (r.event === 'tool_call') {
      lines.push('    Note over ' + s + ': 🔧 ' + summary);
    } else if (r.receiver) {
      const recv = aliases.get(r.receiver) || r.receiver;
      lines.push('    ' + s + '->>' + recv + ': ' + summary);
    } else {
      lines.push('    Note over ' + s + ': ' + summary);
    }
  });
  return lines.join('\\n');
}

async function renderDiagram(entries) {
  const code = buildMermaid(entries);
  const el = document.getElementById('mermaidEl');
  el.removeAttribute('data-processed');
  el.innerHTML = code;
  try { await mermaid.run({ nodes: [el] }); } catch {}
}

function renderFeed(count) {
  const feed = document.getElementById('feed');
  feed.innerHTML = '';
  rows.forEach((r, i) => {
    const vis = i < count;
    const time = (r.timestamp || '').replace('T', ' ').slice(11, 19);
    const icon = r.event === 'message' ? '💬' : r.event === 'tool_call' ? '🔧' : '📋';
    const cls = r.event === 'tool_call' ? 'tool' : r.event === 'task_update' ? 'task' : 'msg';
    const arrow = r.receiver ? r.sender + ' → ' + r.receiver : r.sender;
    const line = document.createElement('div');
    line.className = 'feed-line' + (vis ? ' visible' : '');
    line.innerHTML = '<span class="time">' + time + '</span> ' + icon + ' <span class="' + cls + '">' + arrow + ': ' + (r.summary || '') + '</span>';
    feed.appendChild(line);
  });
  if (count > 0) feed.lastElementChild?.scrollIntoView({ behavior: 'smooth' });
}

function renderFull() {
  visibleCount = rows.length;
  renderDiagram(rows);
  renderFeed(rows.length);
}

function startReplay() {
  playing = true;
  visibleCount = 0;
  document.getElementById('btnPlay').style.display = 'none';
  document.getElementById('btnPause').style.display = '';
  stepReplay();
}

function pauseReplay() {
  playing = false;
  if (playTimeout) clearTimeout(playTimeout);
  document.getElementById('btnPlay').style.display = '';
  document.getElementById('btnPause').style.display = 'none';
}

function stepReplay() {
  if (!playing || visibleCount >= rows.length) {
    pauseReplay();
    document.getElementById('status').textContent = 'Replay complete';
    return;
  }
  visibleCount++;
  const visible = rows.slice(0, visibleCount);
  renderDiagram(visible);
  renderFeed(visibleCount);
  document.getElementById('status').textContent = visibleCount + '/' + rows.length;

  // Calculate delay from real timestamps
  let delayMs = 500;
  if (visibleCount < rows.length) {
    const curr = new Date(rows[visibleCount - 1].timestamp).getTime();
    const next = new Date(rows[visibleCount].timestamp).getTime();
    delayMs = Math.max(100, Math.min(3000, (next - curr) / speed));
  }
  playTimeout = setTimeout(stepReplay, delayMs);
}

// ── Tab switching ────────────────────────────────
document.querySelectorAll('.tab').forEach(tab => {
  tab.addEventListener('click', () => {
    document.querySelectorAll('.tab').forEach(t => t.classList.remove('active'));
    document.querySelectorAll('.view').forEach(v => v.classList.remove('active'));
    tab.classList.add('active');
    document.getElementById('view' + (tab.dataset.view === 'graph' ? 'Graph' : 'Seq')).classList.add('active');
    if (tab.dataset.view === 'graph') resizeCanvas();
  });
});

// ── Network Graph ────────────────────────────────
const canvas = document.getElementById('graphCanvas');
const ctx2d = canvas.getContext('2d');
let graphNodes = [];     // {name, x, y, color, isGeneral}
let graphEdges = new Map(); // "a->b" → {from, to}
let pulses = [];         // {fromX, fromY, toX, toY, progress, color}

function resizeCanvas() {
  canvas.width = canvas.parentElement.offsetWidth;
  canvas.height = 400;
  layoutNodes();
}

function layoutNodes() {
  if (graphNodes.length === 0) return;
  const cx = canvas.width / 2;
  const cy = canvas.height / 2;
  const radius = Math.min(cx, cy) - 60;
  // Find general (center)
  const general = graphNodes.find(n => n.isGeneral);
  const others = graphNodes.filter(n => !n.isGeneral);
  if (general) { general.x = cx; general.y = cy; }
  others.forEach((n, i) => {
    const angle = (2 * Math.PI * i / others.length) - Math.PI / 2;
    n.x = cx + radius * Math.cos(angle);
    n.y = cy + radius * Math.sin(angle);
  });
}

function updateGraphFromFleet(data) {
  const names = new Set();
  data.instances.forEach(inst => names.add(inst.name));
  // Add user node if activity mentions it
  rows.forEach(r => { names.add(r.sender); if (r.receiver) names.add(r.receiver); });
  // Rebuild nodes (preserve positions if same set)
  const oldMap = new Map(graphNodes.map(n => [n.name, n]));
  graphNodes = [...names].map(name => {
    const old = oldMap.get(name);
    const inst = data.instances.find(i => i.name === name);
    const color = !inst ? '#8b949e' : inst.status === 'running' ? '#3fb950' : inst.status === 'crashed' ? '#f85149' : '#484f58';
    return { name, x: old?.x ?? 0, y: old?.y ?? 0, color, isGeneral: inst?.general_topic ?? false };
  });
  layoutNodes();
  // Build edges from activity
  graphEdges.clear();
  rows.forEach(r => {
    if (r.receiver && r.event === 'message') {
      const key = r.sender + '->' + r.receiver;
      graphEdges.set(key, { from: r.sender, to: r.receiver });
    }
  });
}

function spawnPulse(sender, receiver, event) {
  const from = graphNodes.find(n => n.name === sender);
  const to = graphNodes.find(n => n.name === (receiver || sender));
  if (!from || !to) return;
  const colors = { message: '#58a6ff', tool_call: '#d29922', task_update: '#3fb950' };
  pulses.push({ fromX: from.x, fromY: from.y, toX: to.x, toY: to.y, progress: 0, color: colors[event] || '#58a6ff' });
}

function drawGraph() {
  if (!ctx2d) return;
  ctx2d.clearRect(0, 0, canvas.width, canvas.height);
  // Draw edges
  ctx2d.strokeStyle = '#21262d';
  ctx2d.lineWidth = 1;
  graphEdges.forEach(e => {
    const from = graphNodes.find(n => n.name === e.from);
    const to = graphNodes.find(n => n.name === e.to);
    if (from && to) {
      ctx2d.beginPath();
      ctx2d.moveTo(from.x, from.y);
      ctx2d.lineTo(to.x, to.y);
      ctx2d.stroke();
    }
  });
  // Draw pulses
  pulses = pulses.filter(p => p.progress <= 1);
  pulses.forEach(p => {
    p.progress += 0.02;
    const x = p.fromX + (p.toX - p.fromX) * p.progress;
    const y = p.fromY + (p.toY - p.fromY) * p.progress;
    ctx2d.beginPath();
    ctx2d.arc(x, y, 5, 0, Math.PI * 2);
    ctx2d.fillStyle = p.color;
    ctx2d.shadowColor = p.color;
    ctx2d.shadowBlur = 12;
    ctx2d.fill();
    ctx2d.shadowBlur = 0;
  });
  // Draw nodes
  graphNodes.forEach(n => {
    // Glow
    ctx2d.beginPath();
    ctx2d.arc(n.x, n.y, n.isGeneral ? 28 : 22, 0, Math.PI * 2);
    ctx2d.fillStyle = n.color + '22';
    ctx2d.fill();
    // Circle
    ctx2d.beginPath();
    ctx2d.arc(n.x, n.y, n.isGeneral ? 24 : 18, 0, Math.PI * 2);
    ctx2d.fillStyle = '#161b22';
    ctx2d.strokeStyle = n.color;
    ctx2d.lineWidth = 2;
    ctx2d.fill();
    ctx2d.stroke();
    // Label
    ctx2d.fillStyle = '#c9d1d9';
    ctx2d.font = (n.isGeneral ? '12' : '11') + 'px -apple-system, monospace';
    ctx2d.textAlign = 'center';
    ctx2d.fillText(n.name.length > 14 ? n.name.slice(0, 12) + '..' : n.name, n.x, n.y + (n.isGeneral ? 38 : 32));
  });
  requestAnimationFrame(drawGraph);
}

// Hook into replay: spawn pulses when stepping
const origStep = stepReplay;
stepReplay = function() {
  const prevCount = visibleCount;
  origStep();
  if (visibleCount > prevCount && visibleCount <= rows.length) {
    const r = rows[visibleCount - 1];
    spawnPulse(r.sender, r.receiver, r.event);
  }
};

// Hook into full load: spawn pulses for all visible events on load
const origRenderFull = renderFull;
renderFull = function() {
  origRenderFull();
  // Update graph nodes from fleet data (if available)
  fetch('/api/fleet').then(r => r.json()).then(data => {
    updateGraphFromFleet(data);
  }).catch(() => {
    // Fallback: build nodes from activity only
    const names = new Set();
    rows.forEach(r => { names.add(r.sender); if (r.receiver) names.add(r.receiver); });
    graphNodes = [...names].map(n => ({ name: n, x: 0, y: 0, color: '#8b949e', isGeneral: n === 'general' }));
    layoutNodes();
  });
};

resizeCanvas();
window.addEventListener('resize', resizeCanvas);
requestAnimationFrame(drawGraph);

// ── Agent Board ──────────────────────────────────

let prevBoard = '';

async function loadBoard() {
  try {
    const resp = await fetch('/api/fleet');
    const data = await resp.json();
    renderBoard(data);
  } catch {}
}

function renderBoard(data) {
  const board = document.getElementById('board');
  const cards = data.instances.map(inst => {
    const statusDot = inst.status === 'running' ? 'running' : inst.status === 'crashed' ? 'crashed' : 'stopped';
    const icon = inst.status === 'running' ? '🟢' : inst.status === 'crashed' ? '🔴' : '⚪';
    const role = inst.general_topic ? 'coordinator' : inst.description || 'worker';
    const costStr = '$' + (inst.costCents / 100).toFixed(2);
    const lastMs = inst.lastActivity;
    let lastStr = '—';
    if (lastMs) {
      const ago = Math.floor((Date.now() - lastMs) / 1000);
      lastStr = ago < 60 ? ago + 's ago' : ago < 3600 ? Math.floor(ago/60) + 'm ago' : Math.floor(ago/3600) + 'h ago';
    }
    const ipc = inst.ipc ? '✓' : '✗';
    const rl = inst.rateLimits ? ' · 5h:' + inst.rateLimits.five_hour_pct + '%' : '';
    const taskLine = inst.currentTask
      ? '<div class="card-task">📌 ' + inst.currentTask + '</div>'
      : '<div class="card-task" style="color:#484f58">(idle)</div>';
    return '<div class="card" data-name="' + inst.name + '">' +
      '<div class="card-header"><div class="dot ' + statusDot + '"></div><div class="name">' + inst.name + '</div></div>' +
      '<div class="card-row">' + role.slice(0, 30) + '</div>' +
      '<div class="card-row">Backend: <span>' + inst.backend + '</span> · Tools: <span>' + inst.tool_set + '</span></div>' +
      '<div class="card-row">IPC: <span>' + ipc + '</span> · Cost: <span>' + costStr + '</span>' + rl + '</div>' +
      '<div class="card-row">Last: <span>' + lastStr + '</span></div>' +
      taskLine +
      '</div>';
  });

  const newHtml = cards.join('');
  if (newHtml !== prevBoard) {
    board.innerHTML = newHtml;
    // Flash changed cards
    board.querySelectorAll('.card').forEach(c => {
      c.classList.add('flash');
      setTimeout(() => c.classList.remove('flash'), 1000);
    });
    prevBoard = newHtml;
  }
}

// Auto-refresh board every 10s
setInterval(loadBoard, 10000);

// Auto-load on page open
loadBoard();
load();
</script>
</body>
</html>`;
