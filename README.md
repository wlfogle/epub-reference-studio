# EPUB Reference Studio
A Rust/Tauri 2 desktop application for **errors inside EPUB text**, not Calibre metadata. Scan a library, find an available full-text reference, compare aligned text, review corrections and export a separate corrected EPUB plus a JSON change report.

## Build and install
On Linux, install Rust/Cargo, Python 3, a C toolchain, pkg-config and the GTK3/WebKitGTK 4.1 development packages. The tested environment already provides these; no Docker or server is required. Rust 1.99 is tested. The UI is embedded vanilla JavaScript/CSS, so no Node package installation is needed.

Run `python3 scripts/build.py`, then `python3 scripts/install_launcher.py`. Open **EPUB Reference Studio** from the application menu. Use `--offline` on the build script only when all locked Cargo dependencies are cached.

The installed executable is `~/.local/lib/epub-reference-studio/epub-reference-studio`. Source builds run from `target/release/epub-reference-studio`. Its optional `--open /absolute/path/book.epub` argument analyzes a selected book read-only and opens the saved review. `--inspect /absolute/path/book.epub` emits a read-only JSON analysis for diagnostics; that output contains private text excerpts and should not be published.

## Workflow
1. **Scan text:** select a directory or open a single EPUB. Scanning reads the OPF and spine XHTML, reconstructs words across inline formatting, and flags likely joined words, missing punctuation spacing, digit/letter OCR artifacts, broken hyphenation and split words corroborated elsewhere in the book. Findings are candidates, not certified errors. English-oriented heuristics are intentionally conservative; an ordinary-word OCR substitution can require a reference.
2. **Reference sources:** search online, optionally for several checked books. Only selected title/author metadata is sent; EPUB contents are not uploaded. Completed searches/reviews and remaining checked research paths survive restarts; processed paths leave the queue. Unchanged books reuse their SHA-256 scan results. Stop cancels after the current book; rescanning resumes through the saved cache.
3. **Acquire actual text:** download a compatible candidate or import an independently acquired matching EPUB/UTF-8 `.txt` file. A catalogue record is never treated as full text. An EPUB's title, author and language are checked again after download/import. Plain text has no embedded edition identity: the operator must verify it.
4. **Confirm work and edition:** check the edition/translation/abridgment confirmation and compare. Unique five-token anchors plus bounded local alignment identify small differences. Inadequate textual agreement is refused; large/unmatched regions remain unrepaired. This is an alignment measurement, not a probability that every reference sentence is correct.
5. **Review & export:** approve individual differences or inspect/select the spacing proposals. Nothing is selected automatically. Export to a directory **outside Calibre**. The original is never overwritten; the app does not edit `metadata.db`. Inspect the corrected copy in your reader before importing it into Calibre through Calibre's normal UI.

## Edit flagged passages in Sigil
**Open in Sigil** appears beside each suspect passage and proposed reference correction. **Open editable copy in Sigil** is also available at the top of a book review, and corrected exports have their own **Open export in Sigil** action.

The app discovers native `sigil` or the installed `com.sigil_ebook.Sigil` Flatpak. Flatpak launches use documented file forwarding, without shell interpolation, installing plugins or changing sandbox permissions. A missing editor produces a clear error before an editing copy is prepared.

For findings/corrections, the first click creates a byte-verified, owner-private working EPUB under the app workspace's `sigil/` directory. Later passages reuse that same file. Existing manual edits are never replaced with a fresh source copy. Source hashes and target IDs/indices are checked; stale reviews, symlinked copies and unregistered export paths are refused. Legacy saved findings without offsets are re-located from the unchanged original.

Sigil 2.8.5 accepts an EPUB filename but **does not provide a CLI passage-jump option**. The Studio handoff panel therefore shows the chapter/member, source-line hint, passage context and a raw-XHTML search excerpt. In Sigil's Book Browser open that member, then use the hint or paste the excerpt into Find & Replace in **Normal** mode. Inline formatting/entities can make a visible word differ from its raw XHTML; the chapter/line hint remains useful. These are hints for the saved file, not guaranteed positions after Sigil prettification or unsaved changes. No IDs, reading order, TOC or book content are rewritten merely to navigate.

**Copy search text** changes the clipboard only when explicitly requested; the app never reads the prior clipboard. It uses existing `xsel` on X11 or `wl-copy` on Wayland. If unavailable, select the displayed read-only text and press Ctrl+C. Opening a passage does not silently replace your clipboard.

A tracked live Sigil process for the same working copy is reused rather than launching a duplicate. PID start-time checks prevent PID-reuse mistakes. If you opened a copy independently, save/close that separate editor before reopening it through Studio; arbitrary external or delegated editor windows cannot be detected reliably. The app never forces a Sigil save or closes your editor.

After editing, save in Sigil (Ctrl+S), then choose **Re-analyze saved Sigil copy**. Studio creates/reuses a review of that saved working file, retains its existing editing-copy binding and does not alter the original review or Calibre book. If earlier text was edited away or became ambiguous, stale passage hints are not guessed: re-analysis supplies fresh findings. A corrected export is already a separate editable file; to analyze an edited export, select it with **Open an EPUB**. Import final copies into Calibre through Calibre itself.

Private launch diagnostics are appended to `sigil-launch.log` in the workspace. Sigil is a GUI application: do not invoke `sigil --help` as a capability probe—it starts the GUI rather than printing command help. Navigation limits were checked against [Sigil 2.8.5's launcher](https://github.com/Sigil-Ebook/Sigil/blob/2.8.5/src/main.cpp) and the [Book Browser guide](https://sigil-ebook.com/sigil-user-guide/epub_content/guide/OEBPS/Text/book_browser.xhtml).

## Online reference sources
- **Project Gutenberg's official OPDS feed** is the primary machine-readable discovery source. The app uses its documented `/ebooks/search.opds/` API, not a scraper of human search pages.
- **Gutendex** is an alternative Gutenberg metadata service, used if official discovery provides no compatible downloadable candidate. It can time out; that is reported rather than disguised as a successful lookup.
- **Open Library Search API** provides additional work/edition catalogue leads. It is **not** treated as a source of unrestricted full-text downloads, even when a result has an availability flag.

References retain the source URL, acquisition timestamp, metadata and SHA-256. Requests are bounded, rate-limited between catalogue requests, HTTPS-only, proxy-free, restricted to provider hosts and pinned to public DNS addresses; redirects are rechecked. A successful download does not prove the edition is identical or the text flawless. Gutenberg itself notes that source editions and typography may differ.

Modern copyrighted novels often have **no freely downloadable matching edition** on these sources. The app reports that honestly. Import a matching copy you can access independently in that case. It does not obtain account cookies, remove DRM, bypass lending/access restrictions, invent missing reference text or use an AI rewrite as evidence. Nothing guarantees automatic coverage of every book.

Source documentation: [Gutenberg machine-readable catalogues and OPDS](https://www.gutenberg.org/ebooks/offline_catalogs.html), [Gutendex API](https://gutendex.com/), [Open Library Search API](https://openlibrary.org/dev/docs/api/search).

## Local Ollama assistant
The app connects to the existing Ollama service at `http://127.0.0.1:11434`; the initially selected installed model is `qwen3.5:4b`. Expand **Local Ollama assistant** to refresh installed models, choose a different model and **Save model**. Connection checks use `/api/tags` and `/api/show`; no weights are downloaded or services restarted. Embedding-only/cloud models are excluded or rejected. Only numeric loopback origins are accepted; there is no inherited proxy, LAN or hosted fallback.

**Ask Ollama about first 8 suspects** and **Ask Ollama about up to 8 differences** make explicit local inference requests. Difference assessment uses up to eight reviewed selections, or the first eight differences when none are selected. Each request contains at most 10 KB of bounded excerpts, not the entire book/library. A reference excerpt is included only when there is an actual comparison.

The local model returns `likely_ocr`, `edition_difference` or `uncertain` with a reason. IDs, count, verdicts, explanation bounds and complete structured output are validated; malformed/truncated answers are rejected. These assessments are **advisory, not textual sources**. They never create correction patches, change approval selections or invoke export. Model errors remain possible.

Private caches are keyed by the exact input, prompt version, endpoint, model name and installed model digest. Repeating an unchanged request reuses its validated assessment; model updates invalidate reuse. The workspace retains only the bounded submitted excerpts and advisory result. Actual local `qwen3.5:4b` inference/cache tests use fictional text and verify unchanged source hashes/review selections.

## Preservation and safety
ZIP path traversal, duplicate entries, symlinks, oversized archives/XML and custom XML entities are rejected. Unsupported/encrypted text and malformed/non-UTF-8 XHTML are reported per book; they are not silently regenerated. Font obfuscation records can be preserved when the encrypted resources are fonts.

Corrections are mapped back to the original XHTML text nodes. Existing elements, links, spine, navigation, covers and other resources remain. If a correction crosses inline formatting, replacement text is placed in the first affected text node; surrounding markup is retained, but formatting of that corrected phrase can consequently differ. The review is plain text, not a promise of identical typography.

Export checks input and reference hashes, selection validity and overlap; rebuilds and reparses the EPUB; verifies the resulting visible text matches exactly the reviewed changes (apart from whitespace normalization); and publishes with no-overwrite semantics. The EPUB `mimetype` is first and uncompressed. A source mutation blocks publishing. A disk/process failure during the subsequent report/history write may leave a completed standalone EPUB; originals remain unchanged. Check the output directory and saved history before retrying.

The private workspace defaults to `~/.local/share/org.nexus.epub-reference-studio`; `EPUB_STUDIO_WORKSPACE` overrides it for isolated testing. It must be outside Calibre libraries. It contains scan caches, review JSON, reference copies and provenance, with owner-only directory access. Each mutating operation uses a process-safe filesystem lock and atomic review-file writes. Keep this workspace to retain approvals/history. Do not commit reference copies, private books or review caches.

Limits: 128 MiB compressed EPUB, 256 MiB total declared expansion, 32 MiB individual entry, 16 MiB analyzed/reference text, 300,000 comparison tokens, 50,000 EPUBs per library scan, depth 32 and up to 5,000 small differences per comparison. At most 500 heuristic findings are displayed per book, with the full bounded result saved. These limits fail explicitly rather than promising complete repair of arbitrarily large books.

## Tests
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --features custom-protocol -- -D warnings`
- `cargo test -p epub-reference-core`
- `node --check ui/app.js` (optional syntax check when Node is installed)
- `/usr/bin/python3 tests/ui_webkit.py` (installed GTK/WebKit introspection and a graphical display required)
- `python3 tests/sigil_native.py` (opt-in real X11 desktop test; requires built release, installed Sigil, xdotool, ImageMagick and tesseract; keep keyboard/mouse idle)
- `cargo test -p epub-reference-core live_gutenberg_reference_download -- --ignored --nocapture` (explicit public-domain network check)
- `cargo test -p epub-reference-core live_local_ollama_assessment_preserves_review_and_uses_cache -- --ignored --nocapture` (explicit installed local model check; fictional excerpts)

Offline tests create synthetic EPUBs only. They exercise actual parsing, cross-span corrections, entities/Unicode, alignment, false-identity rejection, unchanged-reference behavior, resource preservation, stale hashes, no-overwrite and Calibre-directory refusal, saved reviews and cancellation. The WebKit test drives the real UI with fictional IPC fixtures and checks safe text rendering, online-source UI, confirmation, diff selection and export. The opt-in live test discovers and downloads a real matching public-domain book through the production network code.

### Verified build
On the development Pop!_OS workstation: 20 offline Rust tests, strict Clippy, Python/JavaScript checks, the WebKit UI workflow and the real native-to-Flatpak Sigil handoff passed. The Sigil test uses only a private synthetic EPUB, verifies a real editor window for the working copy and an unchanged source SHA-256, and closes only its own fixture windows. Prior live Gutenberg discovery/download and actual local Ollama assessment/cache tests also passed.

The optimized native application was built, installed and opened on the selected Calibre EPUB. Its read-only scan reported 196 heuristic findings with an unchanged original SHA-256. Those findings are not 196 certified errors, and no production book was automatically corrected or replaced.
