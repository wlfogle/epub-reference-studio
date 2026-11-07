const metadata = { title: "Fictional Volume <img src=x onerror='window.pwned=true'>", authors: ["Example Writer"], language: "en", identifiers: [] };
let job = { id: "a".repeat(32), book_path: "/fixture/book.epub", input_sha256: "original",
  metadata, findings: [{ kind: "Joined words", location: "chapter.xhtml", context: "The path ofVarnal.", explanation: "Inspect spacing." }],
  reference: null, search: null, comparison: null, selected: [], exports: [], updated: 1 };
window.confirm = () => true;
window.alert = () => {};
window.__TAURI__ = {
  event: { listen: async () => () => {} },
  core: { invoke: async (command, args = {}) => {
    if (command === "choose") return args.kind === "directory" ? "/fixture/exports" : args.kind === "reference" ? "/fixture/reference.epub" : "/fixture/book.epub";
    if (command === "open_source") return;
    if (command !== "operation") throw Error("Unexpected command " + command);
    const { action, args: data } = args;
    if (action === "initialize") return { library: { root: "/fixture", complete: true, books: [
      { path: job.book_path, metadata, findings: 1, error: null }
    ] }, jobs: [], workspace: "/private/fixture", ollama: { endpoint:"http://127.0.0.1:11434",model:"qwen3.5:4b" } };
    if (action === "models") return [{ name:"qwen3.5:4b",size:3324173934,digest:"fixture-model" }];
    if (action === "save_ai") return data.config;
    if (action === "analyze" || action === "job") return structuredClone(job);
    if (action === "assist") {
      const selected = [...job.selected];
      job.ai = { items:[{ id:"fixture",verdict:"uncertain",reason:"Verify the spacing against the reference." }],
        inputs:[{ id:"fixture",suspect:"ofVarnal",reference:null,context:"Fictional excerpt" }],
        model:"qwen3.5:4b",model_digest:"fixture-model",created:1,cache_hit:false,input_sha256:"fixture" };
      if (JSON.stringify(selected) !== JSON.stringify(job.selected)) throw Error("AI changed approval");
      return structuredClone(job);
    }
    if (action === "search") {
      if (Object.keys(data).some(k => /text|contents|ebook/.test(k))) throw Error("Unexpected book upload");
      job.search = { candidates: [{ id: "pg-1", provider: "Fixture provider", metadata,
        page_url: "https://www.gutenberg.org/ebooks/1", download_url: "https://www.gutenberg.org/a.epub",
        identity_match: true, format: "application/epub+zip", notice: "Edition review required." }],
        notices: ["Fixture catalogue result."] };
    } else if (action === "download" || action === "import") {
      job.reference = { path: "/private/reference.epub", sha256: "reference", metadata,
        source: "Fixture public reference", acquired: 1, plain_text: false, confirmed: false };
    } else if (action === "compare") {
      if (!data.confirmed) throw Error("Missing identity consent");
      job.comparison = { warning: "Review edition differences.", anchor_count: 20, anchored_fraction: .9,
        skipped_regions: 0, edits: [{ id: "edit1", before: "ofVarnal", after: "of Varnal", kind: "Reference-backed text",
          context: "The path ofVarnal.", location: "chapter.xhtml", start: 1, end: 9 }] };
      job.reference.confirmed = true;
    } else if (action === "select") job.selected = data.selected;
    else if (action === "export") {
      if (job.selected.length !== 1) throw Error("Export without reviewed selection");
      job.exports.push("/fixture/exports/corrected.epub");
      return "/fixture/exports/corrected.epub";
    } else throw Error("Unexpected action " + action);
    return structuredClone(job);
  } }
};
window.__RUN_UI_TEST__ = async () => {
  const wait = async predicate => {
    for (let i = 0; i < 150; i++) { if (predicate()) return; await new Promise(r => setTimeout(r, 20)); }
    throw Error("UI condition timed out");
  };
  const ready = () => !document.body.classList.contains("busy");
  const click = async text => {
    const b = [...document.querySelectorAll("button")].find(b => b.textContent.trim().includes(text));
    if (!b) throw Error("Missing UI control " + text);
    b.click(); await wait(ready);
  };
  await wait(() => ready() && document.querySelector(".book button"));
  document.querySelector(".book button").click(); await wait(ready);
  if (!document.querySelector("#ollamaStatus").textContent.includes("qwen3.5:4b")) throw Error("Local model not connected");
  if (window.pwned || document.querySelector("#detail img")) throw Error("Untrusted ebook content executed");
  if (!document.querySelector("#detail").textContent.includes("ofVarnal")) throw Error("No text finding shown");
  await click("Ask Ollama about first 8 suspects");
  if (!document.querySelector("#detail").textContent.includes("Ollama advisory assessment")) throw Error("AI advice missing");
  if (job.selected.length) throw Error("AI approved a change");
  await click("Reference sources"); await click("Search online");
  if (!document.querySelector("#detail").textContent.includes("Fixture provider")) throw Error("Search results missing");
  await click("Download reference");
  await click("Compare actual text");
  if (document.querySelector("#error").hidden) throw Error("Missing edition-consent refusal");
  document.querySelector("#editionConfirmed").checked = true;
  await click("Compare actual text");
  if (!document.querySelector(".before")?.textContent.includes("ofVarnal") ||
      !document.querySelector(".after")?.textContent.includes("of Varnal")) throw Error("Comparison not visible");
  const check = document.querySelector(".change input"); check.checked = true; check.dispatchEvent(new Event("change"));
  await wait(ready);
  await click("Export 1 reviewed changes");
  if (!document.querySelector("#detail").textContent.includes("/fixture/exports/corrected.epub")) throw Error("Export history missing");
  return { status: "passed", checks: ["library", "XSS-safe text", "findings", "Ollama connection/advice", "online search", "download", "edition gate", "diff", "review", "export"] };
};
