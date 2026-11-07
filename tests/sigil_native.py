"""Native Sigil handoff test using only a private synthetic EPUB."""
import csv
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
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


def click_button(window, image):
    call("import", "-window", window, str(image))
    raw = call("tesseract", str(image), "stdout", "tsv").stdout
    lines = {}
    for word in csv.DictReader(io.StringIO(raw), delimiter="\t"):
        if word["text"].strip():
            key = tuple(word[k] for k in (
                "block_num", "par_num", "line_num",
            ))
            lines.setdefault(key, []).append(word)
    for words in lines.values():
        text = " ".join(w["text"] for w in words)
        if "Open editable copy" not in text:
            continue
        left = min(int(w["left"]) for w in words)
        right = max(int(w["left"]) + int(w["width"]) for w in words)
        top = min(int(w["top"]) for w in words)
        bottom = max(int(w["top"]) + int(w["height"]) for w in words)
        call("xdotool", "windowactivate", "--sync", window,
             "mousemove", "--window", window,
             str((left + right) // 2), str((top + bottom) // 2),
             "click", "1")
        return True
    return False


def main():
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
                    print(json.dumps({
                        "status": "passed", "editor_window": sigil_window,
                        "original_unchanged": True, "editing_copy": str(copy),
                        "evidence": str(directory),
                    }))
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
