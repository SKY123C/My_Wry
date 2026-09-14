import { invokePython } from "./python_bridge.js";

const $ = (selector) => document.querySelector(selector);
const state = { folder: "/Game", folders: [], assets: [], tree: [], expanded: new Set(["/Game"]), selected: null, thumbnails: new Map(), request: 0 };
const grid = $("#grid");
const thumbnailQueue = [];
let thumbnailActive = 0;
let thumbnailErrorShown = false;

function status(message) { $("#status-text").textContent = message.toUpperCase(); }
function toast(message) {
  const node = $("#toast");
  node.textContent = message;
  node.hidden = false;
  clearTimeout(toast.timer);
  toast.timer = setTimeout(() => { node.hidden = true; }, 3600);
}
function report(error) { const message = error instanceof Error ? error.message : String(error); status(message); toast(message); }
function objectPath(path) { return path.includes(".") ? path : `${path}.${path.split("/").pop()}`; }

async function load(folder = state.folder) {
  const request = ++state.request;
  status(`Loading ${folder}`);
  try {
    const result = await invokePython("browser.list", { folder });
    if (request !== state.request) return;
    state.folder = result.folder;
    state.folders = result.folders;
    state.assets = result.assets;
    state.selected = null;
    for (const pending of thumbnailQueue) state.thumbnails.delete(pending.path);
    thumbnailQueue.length = 0;
    for (const ancestor of ancestors(state.folder)) state.expanded.add(ancestor);
    render();
    status(result.truncated ? "Showing first 500 items" : `${result.assets.length} assets loaded`);
  } catch (error) { report(error); }
}

function ancestors(path) {
  const parts = path.split("/").filter(Boolean);
  return parts.map((_, index) => "/" + parts.slice(0, index + 1).join("/"));
}

async function loadTree() {
  try {
    const result = await invokePython("browser.tree", {}, { timeoutMs: 20000 });
    state.tree = result.paths;
    renderTree();
  } catch (error) { report(error); }
}

function renderTree() {
  const host = $("#folder-tree");
  host.replaceChildren();
  const paths = new Set(state.tree);
  for (const path of state.tree) {
    const parent = path.slice(0, path.lastIndexOf("/")) || "/Game";
    if (parent !== "/Game" && !paths.has(parent)) continue;
    if (!ancestors(parent).every((ancestor) => state.expanded.has(ancestor))) continue;
    const row = document.createElement("div"); row.className = "tree-row";
    const depth = path.split("/").length - 3;
    row.style.paddingLeft = `${Math.max(0, depth) * 13}px`;
    const hasChildren = state.tree.some((other) => other.startsWith(path + "/"));
    const toggle = document.createElement("button"); toggle.className = "tree-toggle";
    toggle.textContent = hasChildren ? (state.expanded.has(path) ? "▾" : "▸") : "·";
    toggle.onclick = () => { if (state.expanded.has(path)) state.expanded.delete(path); else state.expanded.add(path); renderTree(); };
    const name = document.createElement("button"); name.className = `tree-name${path === state.folder ? " active" : ""}`;
    name.title = path; name.textContent = `▱  ${path.split("/").pop()}`;
    name.onclick = () => { state.expanded.add(path); load(path); };
    row.append(toggle, name); host.append(row);
  }
}

function render() {
  const parts = state.folder.split("/").filter(Boolean);
  const crumb = $("#breadcrumb");
  crumb.replaceChildren();
  parts.forEach((part, index) => {
    if (index) { const slash = document.createElement("b"); slash.textContent = "/"; crumb.append(slash); }
    const button = document.createElement("button");
    button.textContent = part;
    button.onclick = () => load("/" + parts.slice(0, index + 1).join("/"));
    crumb.append(button);
  });
  $("#section-heading").textContent = state.folder === "/Game" ? "All Assets" : parts.at(-1);
  renderTree();
  renderGrid();
  renderInspector();
}

function renderGrid() {
  const query = $("#search").value.trim().toLowerCase();
  const items = [
    ...state.folders.map((folder) => ({ ...folder, folder: true, class: "FOLDER" })),
    ...state.assets.map((asset) => ({ ...asset, folder: false })),
  ].filter((item) => item.name.toLowerCase().includes(query));
  $("#asset-count").textContent = `${items.length} items`;
  $("#empty").hidden = items.length !== 0;
  grid.replaceChildren(...items.map(makeCard));
  for (const asset of items.filter((item) => !item.folder).slice(0, 80)) loadThumbnail(asset);
}

function makeCard(item) {
  const card = document.createElement("article");
  card.className = `card${state.selected?.path === item.path ? " selected" : ""}`;
  const preview = document.createElement("div"); preview.className = `preview${item.folder ? " folder" : ""}`;
  const glyph = document.createElement("span"); glyph.className = "glyph"; glyph.textContent = item.folder ? "▰" : symbol(item.class);
  preview.append(glyph);
  const tag = document.createElement("span"); tag.className = "type-tag"; tag.textContent = item.class.toUpperCase(); preview.append(tag);
  if (!item.folder && state.thumbnails.has(item.path)) setImage(preview, state.thumbnails.get(item.path));
  const info = document.createElement("div"); info.className = "card-info";
  const name = document.createElement("div"); name.className = "card-name"; name.textContent = item.name;
  const meta = document.createElement("div"); meta.className = "card-meta"; meta.textContent = item.folder ? "Directory" : item.class;
  info.append(name, meta); card.append(preview, info);
  card.onclick = () => { state.selected = item; renderGrid(); renderInspector(); };
  card.ondblclick = () => item.folder ? load(item.path) : syncAsset(item);
  card.dataset.path = item.path;
  return card;
}

function symbol(type) {
  const value = String(type).toLowerCase();
  if (value.includes("material")) return "◈";
  if (value.includes("texture")) return "▧";
  if (value.includes("blueprint")) return "⌘";
  if (value.includes("staticmesh")) return "⬡";
  return "✦";
}

function setImage(container, imageUrl) {
  if (!imageUrl) return;
  const image = document.createElement("img"); image.src = imageUrl; image.alt = "";
  container.querySelector(".glyph")?.remove();
  container.prepend(image);
}

function loadThumbnail(asset) {
  if (state.thumbnails.has(asset.path)) return;
  state.thumbnails.set(asset.path, null);
  thumbnailQueue.push(asset);
  pumpThumbnails();
}

function pumpThumbnails() {
  while (thumbnailActive < 4 && thumbnailQueue.length) {
    const asset = thumbnailQueue.shift();
    thumbnailActive++;
    fetchThumbnail(asset).finally(() => { thumbnailActive--; pumpThumbnails(); });
  }
}

async function fetchThumbnail(asset) {
  try {
    const response = await fetch("http://127.0.0.1:30010/remote/object/thumbnail", {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ objectPath: objectPath(asset.path) }),
    });
    if (!response.ok) throw new Error(`Remote Control HTTP ${response.status}`);
    const blob = await response.blob();
    if (!blob.type.startsWith("image/")) throw new Error(`缩略图响应类型错误：${blob.type || "unknown"}`);
    const imageUrl = URL.createObjectURL(blob);
    state.thumbnails.set(asset.path, imageUrl);
    for (const card of grid.querySelectorAll(".card")) {
      if (card.dataset.path === asset.path) setImage(card.querySelector(".preview"), imageUrl);
    }
    if (state.selected?.path === asset.path) renderInspector();
  } catch (error) {
    state.thumbnails.delete(asset.path);
    if (!thumbnailErrorShown) {
      thumbnailErrorShown = true;
      report(new Error(`缩略图获取失败：${error instanceof Error ? error.message : String(error)}`));
    }
  }
}

function renderInspector() {
  const host = $("#inspector-content");
  const item = state.selected;
  if (!item) { host.innerHTML = '<div class="inspector-placeholder"><div>◇</div><p>SELECT AN ASSET</p><small>Asset details will appear here.</small></div>'; return; }
  host.replaceChildren();
  const preview = document.createElement("div"); preview.className = "inspector-preview";
  const glyph = document.createElement("span"); glyph.className = "glyph"; glyph.textContent = item.folder ? "▰" : symbol(item.class); preview.append(glyph);
  setImage(preview, state.thumbnails.get(item.path));
  const name = document.createElement("div"); name.className = "inspector-name"; name.textContent = item.name;
  const type = document.createElement("div"); type.className = "inspector-type"; type.textContent = item.folder ? "DIRECTORY" : String(item.class).toUpperCase();
  const label = document.createElement("div"); label.className = "detail-label"; label.textContent = "OBJECT PATH";
  const value = document.createElement("div"); value.className = "detail-value"; value.textContent = item.path;
  const action = document.createElement("button"); action.className = "create-button inspect-action";
  action.textContent = item.folder ? "Open Folder →" : "Select in Unreal →";
  action.onclick = () => item.folder ? load(item.path) : syncAsset(item);
  host.append(preview, name, type, label, value, action);
}

async function syncAsset(item) {
  try { await invokePython("browser.select", { path: objectPath(item.path) }); toast(`Selected ${item.name} in Unreal`); status("Selection synced"); }
  catch (error) { report(error); }
}

$("#search").oninput = renderGrid;
function refresh() { thumbnailErrorShown = false; loadTree(); load(); }
$("#refresh").onclick = refresh;
$("#rail-refresh").onclick = refresh;
$("#root-folder").onclick = () => load("/Game");
function openModal(kind = "material") { $("#create-kind").value = kind; $("#create-name").value = ""; $("#create-location").textContent = `LOCATION  ${state.folder}`; $("#modal").hidden = false; $("#create-name").focus(); }
function closeModal() { $("#modal").hidden = true; }
$("#create").onclick = () => openModal();
$("#folder-add").onclick = () => openModal("folder");
$("#modal-close").onclick = closeModal;
$("#modal").onclick = (event) => { if (event.target === $("#modal")) closeModal(); };
document.onkeydown = (event) => { if (event.key === "Escape") closeModal(); };
$("#create-form").onsubmit = async (event) => {
  event.preventDefault();
  const kind = $("#create-kind").value;
  const name = $("#create-name").value.trim();
  const button = $(".modal-submit"); button.disabled = true;
  try {
    const action = kind === "folder" ? "browser.create_folder" : "browser.create";
    await invokePython(action, { folder: state.folder, name, kind }, { timeoutMs: 20000 });
    closeModal(); toast(`${name} created in Unreal`); await load();
    if (kind === "folder") await loadTree();
  } catch (error) { report(error); }
  finally { button.disabled = false; }
};

invokePython("browser.environment").then((result) => { $("#project-name").textContent = result.project; }).catch(report);
loadTree();
load("/Game");

