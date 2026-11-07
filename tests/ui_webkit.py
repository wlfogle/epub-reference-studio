"""Test the real frontend in WebKit; fictional IPC only, no library access."""
import functools
import http.server
import json
import pathlib
import threading
import time

import gi

gi.require_version("Gtk", "3.0")
try:
    gi.require_version("WebKit2", "4.1")
except ValueError:
    gi.require_version("WebKit2", "4.0")
from gi.repository import GLib, Gtk, WebKit2  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parent.parent


def main():
    class Quiet(http.server.SimpleHTTPRequestHandler):
        def log_message(self, *_args):
            pass

    server = http.server.ThreadingHTTPServer(
        ("127.0.0.1", 0),
        functools.partial(Quiet, directory=str(ROOT / "ui")),
    )
    threading.Thread(target=server.serve_forever, daemon=True).start()
    manager = WebKit2.UserContentManager()
    manager.add_script(WebKit2.UserScript.new(
        (ROOT / "tests/ui_workflow.js").read_text(),
        WebKit2.UserContentInjectedFrames.TOP_FRAME,
        WebKit2.UserScriptInjectionTime.START, None, None,
    ))
    view = WebKit2.WebView.new_with_user_content_manager(manager)
    window = Gtk.Window()
    window.set_default_size(1380, 900)
    window.add(view)
    window.show_all()
    result = {}
    started = time.monotonic()
    launched = False

    def poll():
        nonlocal launched
        if time.monotonic() - started > 25:
            result["error"] = "WebKit test timed out"
            Gtk.main_quit()
            return False
        if not launched and not view.is_loading():
            launched = True
            view.evaluate_javascript(
                "window.__RUN_UI_TEST__().then(x=>window.__UI_RESULT__=x)"
                ".catch(e=>window.__UI_RESULT__={error:String(e)})",
                -1, None, None, None, None, None,
            )
        if launched:
            def received(webview, response, _data):
                try:
                    value = webview.evaluate_javascript_finish(response)
                    raw = value.to_string()
                    if raw not in ("undefined", "null"):
                        result.update(json.loads(raw))
                        Gtk.main_quit()
                except GLib.Error as error:
                    result["error"] = str(error)
                    Gtk.main_quit()
            view.evaluate_javascript(
                "JSON.stringify(window.__UI_RESULT__)",
                -1, None, None, None, received, None,
            )
        return True

    view.load_uri(f"http://127.0.0.1:{server.server_port}/")
    GLib.timeout_add(100, poll)
    try:
        Gtk.main()
    finally:
        window.destroy()
        server.shutdown()
    print(json.dumps(result))
    if result.get("status") != "passed":
        raise SystemExit(1)


if __name__ == "__main__":
    main()
