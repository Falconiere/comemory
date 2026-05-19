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
