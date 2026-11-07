"""Native Sigil handoff/edit test using only a private synthetic EPUB."""
import argparse
import csv
import hashlib
import io
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time
import xml.etree.ElementTree as ET
import zipfile

ROOT = Path(__file__).resolve().parent.parent


def call(*args, check=True):
    return subprocess.run(
        args, check=check, capture_output=True, text=True, timeout=15,
    )


def fixture(path):
    with zipfile.ZipFile(path, "w") as archive:
        archive.writestr("mimetype", "application/epub+zip")
        archive.writestr("META-INF/container.xml", (
            '<container xmlns="urn:oasis:names:tc:opendocument:'
            'xmlns:container" version="1.0"><rootfiles><rootfile '
            'full-path="OEBPS/content.opf" media-type="application/'
            'oebps-package+xml"/></rootfiles></container>'
        ))
        archive.writestr("OEBPS/content.opf", (
            '<package xmlns="http://www.idpf.org/2007/opf" version="2.0" '
            'unique-identifier="bookid"><metadata '
            'xmlns:dc="http://purl.org/dc/elements/1.1/">'
            '<dc:title>Sigil handoff fixture</dc:title>'
            '<dc:creator>Example Writer</dc:creator>'
            '<dc:language>en</dc:language>'
            '<dc:identifier id="bookid">urn:uuid:sigil-fixture</dc:identifier>'
            '</metadata><manifest><item id="chapter" href="chapter.xhtml" '
            'media-type="application/xhtml+xml"/><item id="ncx" '
            'href="toc.ncx" media-type="application/x-dtbncx+xml"/></manifest>'
            '<spine toc="ncx"><itemref idref="chapter"/></spine></package>'
        ))
        archive.writestr("OEBPS/chapter.xhtml", (
            '<?xml version="1.0" encoding="utf-8"?>\n'
            '<!DOCTYPE html PUBLIC "-//W3C//DTD XHTML 1.1//EN" '
            '"http://www.w3.org/TR/xhtml11/DTD/xhtml11.dtd">\n'
            '<html xmlns="http://www.w3.org/1999/xhtml"><head>'
            '<title>Fixture</title></head><body>\n'
            '<p>The river ofVarnal.The fictional traveler paused.</p>\n'
            '</body></html>'
        ))
        archive.writestr("OEBPS/toc.ncx", (
            '<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" '
            'version="2005-1">'
            '<head><meta name="dtb:uid" content="urn:uuid:sigil-fixture"/>'
            '</head><docTitle><text>Fixture</text></docTitle><navMap>'
            '<navPoint id="nav1" playOrder="1"><navLabel><text>Chapter</text>'
            '</navLabel><content src="chapter.xhtml"/></navPoint>'
            '</navMap></ncx>'
        ))


def visible_windows(*args):
    return call("xdotool", "search", "--onlyvisible", *args,
                check=False).stdout.split()


def ocr(window, image):
    call("import", "-window", window, str(image))
    raw = call("tesseract", str(image), "stdout", "tsv").stdout
    lines = {}
    for word in csv.DictReader(
        io.StringIO(raw), delimiter="\t", quoting=csv.QUOTE_NONE,
    ):
        if word["text"].strip():
            key = tuple(word[k] for k in (
                "block_num", "par_num", "line_num",
            ))
            lines.setdefault(key, []).append(word)
    return list(lines.values())


def click_at(window, x, y):
    call("xdotool", "windowactivate", "--sync", window,
         "mousemove", "--window", window, str(x), str(y), "click", "1")


def click_button(window, image, needle="Open editable copy"):
    for words in ocr(window, image):
        text = " ".join(w["text"] for w in words)
        if needle not in text:
            continue
        left = min(int(w["left"]) for w in words)
        right = max(int(w["left"]) + int(w["width"]) for w in words)
        top = min(int(w["top"]) for w in words)
        bottom = max(int(w["top"]) + int(w["height"]) for w in words)
        click_at(window, (left + right) // 2, (top + bottom) // 2)
        return True
    return False


def chapter_text(path):
    with zipfile.ZipFile(path) as archive:
        if archive.testzip() is not None:
            raise RuntimeError("Saved EPUB failed CRC validation")
        raw = archive.read("OEBPS/chapter.xhtml")
    document = ET.fromstring(raw)
    text = " ".join("".join(document.itertext()).split())
    return raw.decode("utf-8"), text


def edit_and_save(window, path, directory):
    raw, before = chapter_text(path)
    if "ofVarnal.The" not in before:
        raise RuntimeError("Wrong fixture or edit already applied")
    paragraph = next(
        line for line in raw.splitlines() if "ofVarnal.The" in line
    )
    if not paragraph.startswith("<p>") or not paragraph.isascii():
        raise RuntimeError("Unexpected fixture layout; refusing guessed edit")
    lines = ocr(window, directory / "sigil-before-edit.png")
    width = int(call(
        "identify", "-format", "%w", str(directory / "sigil-before-edit.png"),
    ).stdout)
    candidates = [
        w for line in lines for w in line
        if "ofvarnal" in w["text"].lower()
        and 150 < int(w["left"]) < width * .75
    ]
    if len(candidates) != 1:
        raise RuntimeError("Cannot uniquely identify fixture in Code View")
    word = candidates[0]
    click_at(window, int(word["left"]) + int(word["width"]) // 2,
             int(word["top"]) + int(word["height"]) // 2)
    # This controlled fixture line has no indentation or wrapped prefix.
    offset = paragraph.index("ofVarnal") + len("of")
    call("xdotool", "key", "--clearmodifiers", "Home",
         *(["Right"] * offset))
    call("xdotool", "type", "--clearmodifiers", "--delay", "25", " ")
    call("import", "-window", window, str(directory / "sigil-edited.png"))
    call("xdotool", "key", "--clearmodifiers", "ctrl+s")
    for _ in range(100):
        try:
            _, after = chapter_text(path)
            if "of Varnal.The" in after and "ofVarnal" not in after:
                call("import", "-window", window,
                     str(directory / "sigil-saved.png"))
                return
        except (OSError, zipfile.BadZipFile, ET.ParseError):
            pass  # Sigil may be atomically replacing the ZIP while saving.
        time.sleep(.1)
    raise RuntimeError("Sigil did not save the expected single-space edit")


def confirm_fixture_dialog(process, window, directory, workspace):
    for _ in range(30):
        if len(list((workspace / "jobs").glob("*.json"))) > 1:
            return
        for candidate in visible_windows("--pid", str(process.pid)):
            if candidate == window:
                continue
            transient = call(
                "xprop", "-id", candidate, "WM_TRANSIENT_FOR", check=False,
            ).stdout
            if hex(int(window)) not in transient:
                continue
            words = [w for line in ocr(
                candidate, directory / "reanalyze-confirm.png",
            ) for w in line]
            for word in words:
                if word["text"].strip() in ("OK", "Yes"):
                    click_at(candidate,
                             int(word["left"]) + int(word["width"]) // 2,
                             int(word["top"]) + int(word["height"]) // 2)
                    return
        time.sleep(.1)
    raise RuntimeError("Saved-copy confirmation was not available")


def reanalyze_saved_copy(window, process, copy, directory):
    workspace = directory / "workspace"
    call("xdotool", "windowactivate", "--sync", window)
    for _ in range(12):
        if click_button(window, directory / "studio-reanalyze.png",
                        "Re-analyze saved"):
            break
        call("xdotool", "mousemove", "--window", window, "950", "650",
             "click", "--repeat", "3", "5")
        time.sleep(.15)
    else:
        raise RuntimeError("Saved-copy re-analysis control was not found")
    confirm_fixture_dialog(process, window, directory, workspace)
    for _ in range(100):
        jobs = [
            json.loads(p.read_text())
            for p in (workspace / "jobs").glob("*.json")
        ]
        review = next((j for j in jobs if j["book_path"] == str(copy)), None)
        if review:
            kinds = [f["kind"] for f in review["findings"]]
            if kinds != ["Missing sentence space"]:
                raise RuntimeError(f"Unexpected saved-copy findings: {kinds}")
            if review["sigil"]["path"] != str(copy):
                raise RuntimeError("Re-analysis lost the editing-copy binding")
            if len(list((workspace / "sigil").glob("*.epub"))) != 1:
                raise RuntimeError("Re-analysis created a duplicate copy")
            saved_hash = hashlib.sha256(copy.read_bytes()).hexdigest()
            if review["input_sha256"] != saved_hash:
                raise RuntimeError("Review does not match the saved EPUB")
            # The atomic review write can precede the frontend's repaint.
            call("xdotool", "mousemove", "--window", window, "950", "650",
                 "click", "--repeat", "10", "4")
            for _ in range(10):
                time.sleep(.15)
                text = " ".join(
                    word["text"] for line in ocr(
                        window, directory / "studio-reanalyzed.png",
                    ) for word in line
                )
                if re.search(r"\b1\s+heuristic\s+findings\b", text):
                    return len(review["findings"])
            raise RuntimeError("Saved review exists but UI did not show it")
        time.sleep(.1)
    raise RuntimeError("Native app did not re-analyze the saved working copy")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--edit", action="store_true",
        help="Insert one space in Sigil, save and verify native re-analysis",
    )
    args = parser.parse_args()
    # Retain private evidence for inspection if the test fails.
    directory = Path(tempfile.mkdtemp(prefix="epub-sigil-native-"))
    directory.chmod(0o700)
    print(f"Private native-test evidence: {directory}", flush=True)
    source = directory / "original.epub"
    fixture(source)
    original_hash = hashlib.sha256(source.read_bytes()).hexdigest()
    env = os.environ.copy()
    env.pop("LD_PRELOAD", None)
    env["EPUB_STUDIO_WORKSPACE"] = str(directory / "workspace")
    with (directory / "app.log").open("w") as log:
        process = subprocess.Popen([
            str(ROOT / "target/release/epub-reference-studio"),
            "--open", str(source),
        ], env=env, stdout=log, stderr=log)
    window = None
    sigil_window = None
    try:
        deadline = time.monotonic() + 30
        for _ in range(100):
            if time.monotonic() > deadline:
                raise RuntimeError("Native UI startup timed out")
            windows = visible_windows("--pid", str(process.pid))
            if windows:
                window = windows[0]
                if click_button(window, directory / "studio.png"):
                    break
            if process.poll() is not None:
                raise RuntimeError("Native app exited; inspect app.log")
            time.sleep(.2)
        else:
            raise RuntimeError("Native Open in Sigil button not available")
        for _ in range(150):
            jobs = list((directory / "workspace/jobs").glob("*.json"))
            job = json.loads(jobs[0].read_text()) if jobs else {}
            editing = job.get("sigil")
            if editing:
                copy = Path(editing["path"])
                windows = visible_windows("--name", copy.stem)
                if windows:
                    sigil_window = windows[0]
                    call("import", "-window", sigil_window,
                         str(directory / "sigil.png"))
                    current_hash = hashlib.sha256(
                        source.read_bytes(),
                    ).hexdigest()
                    if current_hash != original_hash:
                        raise RuntimeError("Original book changed")
                    if not copy.is_relative_to(directory / "workspace/sigil"):
                        raise RuntimeError("Wrong editing target")
                    result = {
                        "status": "passed", "editor_window": sigil_window,
                        "original_unchanged": True, "editing_copy": str(copy),
                        "evidence": str(directory),
                    }
                    if args.edit:
                        edit_and_save(sigil_window, copy, directory)
                        count = reanalyze_saved_copy(
                            window, process, copy, directory,
                        )
                        final_hash = hashlib.sha256(
                            source.read_bytes(),
                        ).hexdigest()
                        if final_hash != original_hash:
                            raise RuntimeError("Original changed during edit")
                        result.update({
                            "edit": "ofVarnal → of Varnal",
                            "saved_in_sigil": True,
                            "findings_before": len(job["findings"]),
                            "findings_after": count,
                            "reanalysis_via_native_ui": True,
                            "original_sha256": original_hash,
                            "saved_copy_sha256": hashlib.sha256(
                                copy.read_bytes(),
                            ).hexdigest(),
                        })
                    (directory / "verification.json").write_text(
                        json.dumps(result, indent=2) + "\n",
                    )
                    print(json.dumps(result))
                    return
            time.sleep(.2)
        raise RuntimeError("Sigil did not open the copy; inspect launch log")
    finally:
        if sigil_window:
            call("xdotool", "windowactivate", "--sync", sigil_window,
                 "key", "alt+F4", check=False)
        if window:
            call("xdotool", "windowactivate", "--sync", window,
                 "key", "alt+F4", check=False)
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.terminate()


if __name__ == "__main__":
    main()
