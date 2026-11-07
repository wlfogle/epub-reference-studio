/* Imported ebook text is always assigned to textContent, never innerHTML. */
const native = window.__TAURI__;
const invoke = (command, args = {}) => {
  if (!native) return Promise.reject(new Error("Run the native EPUB Reference Studio application."));
  return native.core.invoke(command, args);
};
const op = (action, args = {}) => invoke("operation", { action, args });
const $ = id => document.getElementById(id);
const state = { library: { books: [] }, jobs: [], job: null, tab: "library", view: "findings", busy: false, checked: new Set(), limit: 100, stopBatch: false, sigilHandoff: null, revealSigil: false };
async function openSigil(target) {
  const id = state.job.id;
  if (target.kind === "export" && !window.confirm("Open this corrected export for editing? Save/close an existing Sigil window for this file first to avoid parallel edits.")) return;
  const handoff = await op("open_sigil", { id, target });
  state.job = await op("job", { id });
  state.sigilHandoff = { ...handoff, jobId: id };
  state.revealSigil = true;
}
async function listModels() {
  const config = { endpoint: $("ollamaEndpoint").value.trim(), model: $("ollamaModel").value };
  const models = await op("models", { config });
  const select = $("ollamaModel"); select.replaceChildren();
  for (const model of models) {
    const option = el("option", model.name + ` · ${(model.size / 1e9).toFixed(1)} GB`);
    option.value = model.name; select.append(option);
  }
  if (models.some(m => m.name === config.model)) select.value = config.model;
  else if (config.model) {
    const missing = el("option", "Unavailable: " + config.model);
    missing.value = config.model; missing.disabled = true; select.prepend(missing); select.value = config.model;
    throw new Error("The saved model is unavailable. Choose an installed model and explicitly Save model; nothing was switched automatically.");
  }
  $("ollamaStatus").textContent = `Connected · ${models.length} installed text-capable candidates. Save your selected model; its capabilities are checked before use.`;
  if (!models.length) throw new Error("No installed local generation models found. Nothing was downloaded.");
}
async function saveModel() {
  const config = { endpoint: $("ollamaEndpoint").value.trim(), model: $("ollamaModel").value };
  await op("save_ai", { config });
  $("ollamaStatus").textContent = "Connected · " + config.model + " · advisory only, no automatic changes";
}
async function askOllama(mode) {
  if (!window.confirm("Send up to 8 displayed suspect passages or selected reference differences (at most 10 KB) to your local Ollama model? Suggestions cannot approve or apply corrections.")) return;
  const view = state.view;
  state.job = await op("assist", { id: state.job.id, mode });
  state.view = view;
}
function rememberChecks() { localStorage.setItem("research-queue", JSON.stringify([...state.checked])); }
function el(tag, text, cls) { const e = document.createElement(tag); if (text !== undefined) e.textContent = text; if (cls) e.className = cls; return e; }
function button(text, action, primary = false) { const b = el("button", text, primary ? "primary" : ""); b.disabled = state.busy; b.addEventListener("click", () => run(text, action)); return b; }
function info(text, cls = "muted") { return el("p", text, cls); }
function status(text) { $("status").textContent = text; }
async function run(label, action) {
  if (state.busy) return;
  state.busy = true; $("error").hidden = true; status(label + "…");
  document.body.classList.add("busy");
  for (const b of document.querySelectorAll("button:not(#cancel)")) b.disabled = true;
  try { await action(); status("Ready · Originals unchanged"); }
  catch (e) { $("error").hidden = false; $("error").textContent = String(e.message || e); status("Operation stopped · Originals unchanged"); }
  finally {
    state.busy = false; document.body.classList.remove("busy");
    for (const b of document.querySelectorAll("button")) b.disabled = false;
    $("cancel").hidden = true; renderList(); if (state.job) renderJob();
    if (state.revealSigil) { $("detail").scrollTop = 0; state.revealSigil = false; }
  }
}
function setJob(job) {
  if (state.job?.id !== job.id) state.sigilHandoff = null;
  state.job = job;
  state.jobs = [job, ...state.jobs.filter(j => j.id !== job.id)];
  state.view = job.comparison ? "changes" : "findings";
}
function renderList() {
  $("libraryTab").classList.toggle("active", state.tab === "library");
  $("jobsTab").classList.toggle("active", state.tab === "jobs");
  const filter = $("filter").value.toLowerCase();
  const rows = state.tab === "jobs" ? state.jobs.map(j => ({
    path: j.book_path, metadata: j.metadata, findings: j.findings.length, job: j
  })) : state.library.books;
  const visible = rows.filter(r => (!filter || (r.metadata.title + r.metadata.authors.join(" ") + r.path).toLowerCase().includes(filter))
    && (!$("flaggedOnly").checked || r.findings || r.error));
  $("librarySummary").textContent = `${visible.length} matching / ${rows.length} ${state.tab === "jobs" ? "saved reviews" : "books"}${state.library.root && state.library.complete === false && state.tab === "library" ? " · partial scan; resume to finish" : ""}`;
  const list = $("books"); list.replaceChildren();
  for (const row of visible.slice(0, state.limit)) {
    const card = el("div", undefined, "book" + (state.job?.book_path === row.path ? " selected" : ""));
    const check = el("input"); check.type = "checkbox"; check.checked = state.checked.has(row.path); check.disabled = state.busy || !!row.error;
    check.setAttribute("aria-label", "Select " + (row.metadata.title || row.path));
    check.onchange = () => { if (check.checked) state.checked.add(row.path); else state.checked.delete(row.path); rememberChecks(); };
    const b = button("", async () => {
      setJob(row.job ? await op("job", { id: row.job.id }) : await op("analyze", { path: row.path }));
    });
    b.append(el("strong", row.metadata.title || row.path.split("/").pop()),
      el("span", row.metadata.authors.join(", ") || "No author metadata", "meta"),
      el("span", row.error ? "Unreadable · " + row.error : row.findings + " suspect passages", row.error ? "meta unreadable" : "count"));
    card.append(check, b); list.append(card);
  }
  if (!visible.length) list.append(info("No books in this view. Scan a library, open an EPUB or clear the filter.", "empty"));
  $("moreBooks").hidden = visible.length <= state.limit;
}
function renderJob() {
  const j = state.job;
  if (!j) return;
  const detail = $("detail"); detail.replaceChildren();
  detail.append(el("span", "CONTENT REVIEW", "eyebrow"), el("h2", j.metadata.title || "Untitled EPUB"),
    info(j.metadata.authors.join(" · ") || "Author metadata missing"),
    info(j.book_path, "path"), info("Original SHA-256: " + j.input_sha256, "hash"));
  const bar = el("div", undefined, "count-bar");
  bar.append(el("span", j.findings.length + " heuristic findings"), el("span", (j.comparison?.edits.length || 0) + " reference differences"),
    el("span", j.exports.length + " corrected exports"));
  detail.append(bar);
  const editorActions = el("div", undefined, "row");
  editorActions.append(button("Open editable copy in Sigil", async () => openSigil({ kind: "book" })));
  detail.append(editorActions);
  if (j.sigil || state.sigilHandoff?.jobId === j.id) renderSigil(detail, j);
  const tabs = el("div", undefined, "tabs");
  for (const [key, title] of [["findings", "Suspect text"], ["reference", "Reference sources"], ["changes", "Review & export"]]) {
    const b = button(title, async () => { state.view = key; });
    b.classList.toggle("active", state.view === key); tabs.append(b);
  }
  detail.append(tabs);
  if (state.view === "findings") renderFindings(detail, j);
  if (state.view === "reference") renderReferences(detail, j);
  if (state.view === "changes") renderChanges(detail, j);
  if (j.ai) {
    const card = el("section", undefined, "card");
    card.append(el("h3", "Ollama advisory assessment"), info(`${j.ai.model} · ${j.ai.cache_hit ? "cached" : "local inference"} · ${new Date(j.ai.created * 1000).toLocaleString()}`, "context"),
      info("Model suggestions are not source evidence and do not change your selection or the book. Verify every assessment against the reference.", "notice"));
    for (const advice of j.ai.items) {
      const item = el("div", undefined, "finding");
      const source = (j.ai.inputs || []).find(i => i.id === advice.id);
      item.append(el("span", advice.verdict.replaceAll("_", " "), "kind"));
      if (source) item.append(info(source.suspect, "context"));
      item.append(info(advice.reason)); card.append(item);
    }
    detail.append(card);
  }
}
function renderSigil(detail, j) {
  const handoff = state.sigilHandoff?.jobId === j.id ? state.sigilHandoff : null;
  const card = el("section", undefined, "card");
  card.append(el("h3", "Sigil editing handoff"), info(handoff?.path || j.sigil.path, "path"),
    info("Save edits in this working copy. The Calibre original is not opened for editing.", "notice"));
  if (handoff) {
    if (handoff.member) card.append(info(`Book Browser → ${handoff.member}${handoff.line ? " · source line hint " + handoff.line : ""}`));
    card.append(info(handoff.notice, "context"));
    if (handoff.context) card.append(info(handoff.context, "context"));
    if (handoff.search_text) {
      const label = el("label", "Normal-mode search text (raw XHTML excerpt)");
      const search = el("input"); search.readOnly = true; search.value = handoff.search_text;
      search.setAttribute("aria-label", "Sigil search text");
      search.onclick = () => search.select();
      card.append(label, search, button("Copy search text", async () => {
        await op("copy_sigil_search", { text: handoff.search_text });
      }));
    }
  } else card.append(info("Use a finding's Open in Sigil button for current chapter/line/search details. Sigil has no direct CLI passage-jump option.", "context"));
  if (j.sigil) card.append(info("Tracked working copy for re-analysis: " + j.sigil.path, "path"), button("Re-analyze saved Sigil copy", async () => {
    if (!window.confirm("Save your edits in Sigil first (Ctrl+S). Analyze that saved copy without modifying the original?")) return;
    setJob(await op("analyze_sigil", { id: j.id })); state.view = "findings";
  }, true));
  detail.append(card);
}
function renderFindings(detail, j) {
  detail.append(info("These are suspects, not automatic corrections. A clean reference can reveal additional errors such as ordinary words split by OCR.", "notice"));
  const card = el("section", undefined, "card");
  card.append(el("h3", "Inside the EPUB"));
  for (const [index, f] of j.findings.slice(0, 500).entries()) {
    const item = el("div", undefined, "finding");
    item.append(el("span", f.kind, "kind"), info(f.location, "path"), info(f.context, "context"), info(f.explanation, "context"));
    item.append(button("Open in Sigil", async () => openSigil({ kind: "finding", index })));
    card.append(item);
  }
  if (!j.findings.length) card.append(info("No heuristic suspects. This does not prove the text is error-free; compare a reference to find ordinary-word errors."));
  if (j.findings.length > 500) card.append(info("Showing the first 500 findings; the full set remains in the saved review."));
  const actions = el("div", undefined, "row");
  actions.append(button("Find reference text", async () => { state.view = "reference"; }, true),
    button("Ask Ollama about first 8 suspects", async () => askOllama("findings")));
  card.append(actions); detail.append(card);
}
function renderReferences(detail, j) {
  const searchCard = el("section", undefined, "card");
  searchCard.append(el("h3", "Find a matching online edition"), info("Gutenberg supplies available full texts. Open Library supplies catalogue leads, not unrestricted downloads. Only the title/author query leaves this computer."));
  const label = el("label", "Search title"); label.htmlFor = "query";
  const query = el("input"); query.id = "query"; query.value = j.metadata.title;
  const row = el("div", undefined, "row");
  row.append(button("Search online", async () => {
    const q = query.value;
    if (!window.confirm("Send this title and author metadata to Gutenberg, optional Gutendex fallback, and Open Library? No book text will be uploaded.")) return;
    setJob(await op("search", { id: j.id, query: q, refresh: false })); state.view = "reference";
  }, true), button("Refresh sources", async () => {
    if (!window.confirm("Request fresh online title/author results?")) return;
    setJob(await op("search", { id: j.id, query: query.value, refresh: true })); state.view = "reference";
  }), button("Import reference EPUB / text…", async () => {
    const path = await invoke("choose", { kind: "reference" }); if (!path) return;
    setJob(await op("import", { id: j.id, path })); state.view = "reference";
  }));
  searchCard.append(label, query, row); detail.append(searchCard);
  if (j.reference) {
    const r = j.reference, card = el("section", undefined, "card");
    card.append(el("h3", "Acquired reference"), info(r.metadata.title || "Plain text — no embedded work/edition metadata"),
      info(r.metadata.authors.join(", ")), info(r.source, "path"), info("SHA-256: " + r.sha256, "hash"),
      info("Downloaded/imported " + new Date(r.acquired * 1000).toLocaleString(), "context"));
    const consent = el("input"); consent.type = "checkbox"; consent.id = "editionConfirmed";
    const check = el("label", undefined, "check");
    check.append(consent, document.createTextNode("I verified the same work, language/translation, edition/abridgment and permission to use this reference."));
    card.append(check, button("Compare actual text", async () => {
      if (!consent.checked) throw new Error("Confirm the reference identity and edition before comparing.");
      setJob(await op("compare", { id: j.id, confirmed: true })); state.view = "changes";
    }, true));
    detail.append(card);
  }
  for (const text of j.search?.notices || []) detail.append(info(text, "notice"));
  for (const c of j.search?.candidates || []) {
    const card = el("section", undefined, "card candidate");
    card.append(el("span", c.provider, "kind"), el("h4", c.metadata.title), info(c.metadata.authors.join(", ")),
      el("span", c.identity_match ? "Title / author compatible — edition still needs confirmation" : "Identity mismatch — download blocked", "badge"),
      info(c.format, "context"), info(c.notice, "notice"), info(c.page_url, "path"));
    const row = el("div", undefined, "row");
    row.append(button("Open source page", async () => invoke("open_source", { url: c.page_url })));
    if (c.download_url && c.identity_match) row.append(button("Download reference", async () => {
      setJob(await op("download", { id: j.id, candidate: c.id })); state.view = "reference";
    }, true));
    card.append(row); detail.append(card);
  }
}
function renderChanges(detail, j) {
  const c = j.comparison;
  if (!c) { detail.append(info("Acquire a full-text reference and compare it before reviewing corrections.", "notice"),
    button("Choose reference", async () => { state.view = "reference"; }, true)); return; }
  detail.append(info(c.warning, "notice"), info(`${c.anchor_count} unique five-token anchors · ${(c.anchored_fraction * 100).toFixed(1)}% anchored · ${c.skipped_regions} large/unmatched regions not repaired`, "context"));
  const row = el("div", undefined, "row");
  row.append(button("Select spacing proposals", async () => {
    state.job = await op("select", { id: j.id, selected: c.edits.filter(e => e.kind === "Reference-backed spacing").map(e => e.id) });
    state.view = "changes";
  }), button("Clear selection", async () => {
    state.job = await op("select", { id: j.id, selected: [] }); state.view = "changes";
  }), button("Ask Ollama about up to 8 differences", async () => askOllama("differences")),
  button(`Export ${j.selected.length} reviewed changes…`, async () => {
    if (!j.selected.length) throw new Error("Select the corrections you have reviewed.");
    if (!window.confirm(`Export ${j.selected.length} selected corrections as a NEW EPUB? Your original remains unchanged.`)) return;
    const path = await invoke("choose", { kind: "directory" }); if (!path) return;
    const output = await op("export", { id: j.id, path });
    state.job = await op("job", { id: j.id }); state.view = "changes";
    window.alert("Verified corrected EPUB:\n" + output + "\nA changes.json report is saved beside it.");
  }, true)); detail.append(row);
  if (!c.edits.length) detail.append(info("No small text differences found in aligned regions. Unmatched/large differences are not automatically rewritten."));
  const holder = el("section", undefined, "card");
  for (const edit of c.edits) {
    const item = el("div", undefined, "change");
    const head = el("div", undefined, "change-head"), check = el("input");
    check.type = "checkbox"; check.checked = j.selected.includes(edit.id); check.disabled = state.busy;
    check.setAttribute("aria-label", "Approve correction in " + edit.location);
    check.addEventListener("change", () => run("Saving review", async () => {
      const selected = new Set(state.job.selected); if (check.checked) selected.add(edit.id); else selected.delete(edit.id);
      state.job = await op("select", { id: j.id, selected: [...selected] }); state.view = "changes";
    }));
    head.append(check, el("span", edit.kind, "kind"), info(edit.location, "path"));
    head.append(button("Open in Sigil", async () => openSigil({ kind: "change", id: edit.id })));
    const grid = el("div", undefined, "compare-grid");
    grid.append(el("div", edit.before || "∅ (insertion)", "before"), el("div", edit.after || "∅ (remove)", "after"));
    item.append(head, grid, info(edit.context, "context")); holder.append(item);
  }
  detail.append(holder);
  if (j.exports.length) {
    detail.append(el("h3", "Previous corrected copies"));
    for (const path of j.exports) {
      const row = el("div", undefined, "row");
      row.append(info(path, "path"), button("Open export in Sigil", async () => openSigil({ kind: "export", path })));
      detail.append(row);
    }
  }
}
$("libraryTab").onclick = () => { state.tab = "library"; renderList(); };
$("connectOllama").onclick = () => run("Connecting to local Ollama", listModels);
$("saveOllama").onclick = () => run("Validating and saving local model", saveModel);
$("jobsTab").onclick = () => { state.tab = "jobs"; renderList(); };
$("filter").oninput = () => { state.limit = 100; renderList(); };
$("flaggedOnly").onchange = () => renderList();
$("moreBooks").onclick = () => { state.limit += 100; renderList(); };
$("clearChecks").onclick = () => { state.checked.clear(); rememberChecks(); renderList(); };
$("browseLibrary").onclick = () => run("Choosing library", async () => {
  const path = await invoke("choose", { kind: "directory" }); if (path) $("libraryPath").value = path;
});
$("scan").onclick = () => run("Scanning EPUB contents", async () => {
  $("cancel").hidden = false;
  const library = await op("scan", { path: $("libraryPath").value });
  if (state.library.root && library.root !== state.library.root) { state.checked.clear(); rememberChecks(); }
  state.library = library;
  state.tab = "library";
});
$("cancel").onclick = () => { state.stopBatch = true; invoke("cancel_scan").catch(console.error); status("Stopping after the current book…"); };
$("openBook").onclick = () => run("Analyzing EPUB", async () => {
  const path = await invoke("choose", { kind: "book" }); if (!path) return;
  setJob(await op("analyze", { path }));
});
$("researchBatch").onclick = () => run("Researching selected books", async () => {
  if (!state.checked.size) throw new Error("Check books in the library list first.");
  if (!window.confirm(`Research ${state.checked.size} books online? Only titles/authors are sent; no reference is approved or applied automatically.`)) return;
  state.stopBatch = false; $("cancel").hidden = false;
  const queue = [...state.checked];
  let n = 0;
  for (const path of queue) {
    if (state.stopBatch) break;
    status(`Researching ${++n} / ${queue.length} · completed results are saved`);
    const j = await op("analyze", { path });
    setJob(await op("search", { id: j.id, query: j.metadata.title, refresh: false }));
    state.checked.delete(path); rememberChecks();
  }
  state.tab = "jobs"; state.view = "reference";
});
if (native) native.event.listen("scan-progress", event => status(`Scanning EPUB contents · ${event.payload} books checked`));
run("Opening private workspace", async () => {
  const initial = await op("initialize");
  state.library = initial.library; state.jobs = initial.jobs;
  if (initial.ollama) {
    $("ollamaEndpoint").value = initial.ollama.endpoint;
    $("ollamaModel").replaceChildren(Object.assign(el("option", initial.ollama.model), { value: initial.ollama.model }));
    try { await listModels(); await saveModel(); }
    catch (e) { $("ollamaStatus").textContent = "Ollama unavailable: " + String(e.message || e); }
  }
  try {
    const queue = JSON.parse(localStorage.getItem("research-queue") || "[]");
    if (Array.isArray(queue) && queue.length <= 50000) state.checked = new Set(queue.filter(p => typeof p === "string" && p.length <= 4096));
  } catch { localStorage.removeItem("research-queue"); }
  $("workspace").textContent = "Private workspace: " + initial.workspace;
  if (initial.library.root) $("libraryPath").value = initial.library.root;
  if (initial.jobs.length) setJob(initial.jobs[0]);
});
