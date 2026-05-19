// qwick-memory graph viewer. Vanilla JS, no build step, no innerHTML.

const KIND_COLOR = {
  Memory: "#2b6cb0",
  Repo: "#2f855a",
  Author: "#6b46c1",
  Tag: "#718096",
  File: "#dd6b20",
  Symbol: "#c53030",
};

const cy = cytoscape({
  container: document.getElementById("canvas"),
  elements: [],
  style: [
    {
      selector: "node",
      style: {
        "background-color": (ele) => KIND_COLOR[ele.data("kind")] || "#888",
        label: "data(label)",
        color: "#222",
        "font-size": 10,
        "text-wrap": "ellipsis",
        "text-max-width": 80,
      },
    },
    {
      selector: "edge",
      style: {
        width: 1,
        "line-color": "#bbb",
        "target-arrow-color": "#bbb",
        "target-arrow-shape": "triangle",
        "curve-style": "bezier",
        "font-size": 9,
        label: "data(kind)",
        color: "#666",
      },
    },
  ],
  wheelSensitivity: 0.2,
});

async function fetchJson(url) {
  const resp = await fetch(url, { headers: { Accept: "application/json" } });
  if (!resp.ok) {
    const err = await resp.json().catch(() => ({ error: { code: "unknown", message: resp.statusText } }));
    throw new Error(`${resp.status} ${err.error?.code || "error"}: ${err.error?.message || ""}`);
  }
  return resp.json();
}

function toElements(payload) {
  const nodes = payload.nodes.map((n) => ({
    data: { id: n.id, label: n.label, kind: n.kind, props: n.props },
  }));
  const edges = payload.edges.map((e) => ({
    data: {
      id: e.id,
      source: e.source,
      target: e.target,
      kind: e.kind,
      props: e.props,
    },
  }));
  return [...nodes, ...edges];
}

function mergeElements(payload) {
  for (const el of toElements(payload)) {
    if (!cy.getElementById(el.data.id).nonempty()) {
      cy.add(el);
    }
  }
}

function runLayout(animate = false) {
  cy.layout({
    name: "cose-bilkent",
    animate,
    nodeRepulsion: 4500,
    idealEdgeLength: 80,
    gravity: 0.25,
  }).run();
}

async function loadSeed(layer = "memory") {
  cy.elements().remove();
  const payload = await fetchJson(`/api/seed?layer=${encodeURIComponent(layer)}`);
  mergeElements(payload);
  runLayout(false);
}

function showDetailMessage(text) {
  const el = document.getElementById("detail");
  el.replaceChildren();
  const p = document.createElement("p");
  p.className = "muted";
  p.textContent = text;
  el.appendChild(p);
}

window.addEventListener("DOMContentLoaded", () => {
  loadSeed("memory").catch((e) => {
    console.error(e);
    showDetailMessage(`Failed to load graph: ${e.message}`);
  });
});

cy.on("dblclick", "node", async (evt) => {
  const node = evt.target;
  try {
    const payload = await fetchJson(
      `/api/expand?id=${encodeURIComponent(node.id())}&depth=1`,
    );
    mergeElements(payload);
    runLayout(false);
  } catch (e) {
    console.error("expand failed", e);
  }
});

const ALL_KINDS = ["Memory", "Repo", "Author", "Tag", "File", "Symbol"];

function renderKindFilters() {
  const fs = document.getElementById("kinds");
  fs.replaceChildren();
  const legend = document.createElement("legend");
  legend.textContent = "Kinds";
  fs.appendChild(legend);
  for (const k of ALL_KINDS) {
    const label = document.createElement("label");
    const input = document.createElement("input");
    input.type = "checkbox";
    input.dataset.kind = k;
    input.checked = true;
    input.addEventListener("change", applyKindFilter);
    label.appendChild(input);
    label.appendChild(document.createTextNode(` ${k}`));
    fs.appendChild(label);
  }
}

function applyKindFilter() {
  document.querySelectorAll('input[data-kind]').forEach((cb) => {
    const kind = cb.dataset.kind;
    const display = cb.checked ? "element" : "none";
    cy.elements(`node[kind = "${kind}"]`).style("display", display);
  });
}

document.querySelectorAll('input[data-layer]').forEach((cb) => {
  cb.addEventListener("change", async () => {
    const memOn = document.querySelector('input[data-layer="memory"]').checked;
    const codeOn = document.querySelector('input[data-layer="code"]').checked;
    const layer = codeOn ? "all" : "memory";
    await loadSeed(layer);
    if (!memOn) {
      ["Memory", "Repo", "Author", "Tag"].forEach((k) => {
        cy.elements(`node[kind = "${k}"]`).style("display", "none");
      });
    }
  });
});

renderKindFilters();

const qInput = document.getElementById("q");
const resultsEl = document.getElementById("results");
let searchTimer = null;

function renderResults(items) {
  resultsEl.replaceChildren();
  for (const r of items) {
    const li = document.createElement("li");
    li.textContent = `${r.kind}: ${r.label}`;
    li.dataset.id = r.id;
    li.addEventListener("click", async () => {
      resultsEl.hidden = true;
      try {
        const payload = await fetchJson(
          `/api/expand?id=${encodeURIComponent(r.id)}&depth=1`,
        );
        mergeElements(payload);
        runLayout(false);
        const ele = cy.getElementById(r.id);
        if (ele.nonempty()) {
          cy.center(ele);
        }
      } catch (e) {
        console.error(e);
      }
    });
    resultsEl.appendChild(li);
  }
  resultsEl.hidden = items.length === 0;
}

qInput.addEventListener("input", () => {
  if (searchTimer) clearTimeout(searchTimer);
  const q = qInput.value.trim();
  if (!q) {
    resultsEl.hidden = true;
    resultsEl.replaceChildren();
    return;
  }
  searchTimer = setTimeout(async () => {
    try {
      const data = await fetchJson(
        `/api/search?q=${encodeURIComponent(q)}&limit=20`,
      );
      renderResults(data.results || []);
    } catch (e) {
      console.error(e);
    }
  }, 200);
});

document.addEventListener("click", (e) => {
  if (!resultsEl.contains(e.target) && e.target !== qInput) {
    resultsEl.hidden = true;
  }
});

function buildBlock(title) {
  const block = document.createElement("div");
  block.className = "detail-block";
  const h = document.createElement("h3");
  h.textContent = title;
  block.appendChild(h);
  return block;
}

function buildBodyBlock(markdown) {
  const block = buildBlock("Body");
  const html = marked.parse(markdown);
  const fragment = DOMPurify.sanitize(html, { RETURN_DOM_FRAGMENT: true });
  block.appendChild(fragment);
  return block;
}

function buildEdgeBlock(title, edges, fieldKey) {
  const block = buildBlock(title);
  const ul = document.createElement("ul");
  ul.className = "edge-list";
  for (const e of edges) {
    const li = document.createElement("li");
    const other = e[fieldKey] || "";
    li.textContent = fieldKey === "target"
      ? `${e.edge_kind} → ${other}`
      : `${other} → ${e.edge_kind}`;
    ul.appendChild(li);
  }
  block.appendChild(ul);
  return block;
}

function renderDetail(detail) {
  const el = document.getElementById("detail");
  el.replaceChildren();

  const head = buildBlock(detail.node.kind);
  const idRow = document.createElement("p");
  const idLabel = document.createElement("strong");
  idLabel.textContent = "id: ";
  idRow.appendChild(idLabel);
  const idCode = document.createElement("code");
  idCode.textContent = detail.node.id;
  idRow.appendChild(idCode);
  head.appendChild(idRow);

  const labelRow = document.createElement("p");
  const labelLabel = document.createElement("strong");
  labelLabel.textContent = "label: ";
  labelRow.appendChild(labelLabel);
  labelRow.appendChild(document.createTextNode(detail.node.label));
  head.appendChild(labelRow);

  el.appendChild(head);

  if (detail.memory_body) {
    el.appendChild(buildBodyBlock(detail.memory_body));
  }
  if (detail.outbound && detail.outbound.length) {
    el.appendChild(buildEdgeBlock("Outbound", detail.outbound, "target"));
  }
  if (detail.inbound && detail.inbound.length) {
    el.appendChild(buildEdgeBlock("Inbound", detail.inbound, "source"));
  }
}

cy.on("tap", "node", async (evt) => {
  const node = evt.target;
  try {
    const detail = await fetchJson(
      `/api/node/${encodeURIComponent(node.id())}`,
    );
    renderDetail(detail);
  } catch (e) {
    console.error("detail failed", e);
  }
});

async function resetView() {
  document.querySelectorAll('input[data-layer]').forEach((cb) => {
    cb.checked = cb.dataset.layer === "memory";
  });
  document.querySelectorAll('input[data-kind]').forEach((cb) => {
    cb.checked = true;
  });
  applyKindFilter();
  await loadSeed("memory");
  showDetailMessage("Click a node to see details.");
}

document.getElementById("reset").addEventListener("click", resetView);
document.addEventListener("keydown", (e) => {
  if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) {
    return;
  }
  if (e.key === "r" || e.key === "R") {
    resetView();
  }
});
