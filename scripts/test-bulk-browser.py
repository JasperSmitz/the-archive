#!/usr/bin/env python3
"""Dependency-free local browser regression harness; never uploads to the Archive."""
import functools
import argparse
import http.server
import os
import pathlib
import shutil
import subprocess
import tempfile
import threading


class QuietHandler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_GET(self):
        if self.path not in ("/tests/bulk-upload.html", "/static/bulk-upload.js"):
            self.send_error(404)
            return
        super().do_GET()

    def do_POST(self):
        if self.path != "/test-result":
            self.send_error(404)
            return
        size = int(self.headers.get("Content-Length", "0"))
        if not 0 < size <= 16384:
            self.send_error(413)
            return
        self.server.test_result = self.rfile.read(size).decode("utf-8")
        self.send_response(204)
        self.end_headers()
        self.server.finished.set()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--firefox", action="store_true", help="Use a Firefox-compatible browser instead of Chromium")
    options = parser.parse_args()
    browser = os.environ.get("BROWSER") or next(
        (path for name in (("firefox", "zen") if options.firefox else ("chromium", "chromium-browser", "google-chrome", "brave"))
         if (path := shutil.which(name))), None
    )
    if not browser:
        raise SystemExit("Set BROWSER to a Chromium-compatible executable, or use --firefox with a Firefox-compatible executable.")
    root = pathlib.Path(__file__).resolve().parents[1]
    with tempfile.TemporaryDirectory(prefix="archive-bulk-browser-") as temporary:
        directory = pathlib.Path(temporary)
        for name in ("static/bulk-upload.js", "tests/bulk-upload.html"):
            target = directory / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(root / name, target)
        server = http.server.ThreadingHTTPServer(
            ("127.0.0.1", 0), functools.partial(QuietHandler, directory=temporary)
        )
        server.finished = threading.Event()
        server.test_result = ""
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            profile = directory / "profile"
            profile.mkdir()
            args = (["--headless", "--no-remote", "--profile", str(profile)] if options.firefox
                    else ["--headless", "--disable-gpu", "--disable-background-networking", "--no-first-run",
                          "--password-store=basic", "--no-default-browser-check", f"--user-data-dir={profile}"])
            # Wait for a test-only callback rather than a browser-specific DOM dump.
            with (directory / "browser.log").open("w+") as log:
                process = subprocess.Popen([browser, *args, f"http://127.0.0.1:{server.server_port}/tests/bulk-upload.html"], stdout=log, stderr=log)
                try:
                    if not server.finished.wait(40):
                        log.seek(0)
                        print(log.read())
                        raise SystemExit("Browser timed out before producing test results.")
                    print(server.test_result)
                    if not server.test_result.startswith("10 browser checks passed."):
                        raise SystemExit("Bulk browser checks failed.")
                finally:
                    process.terminate()
                    try:
                        process.wait(timeout=3)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()
        finally:
            server.shutdown()
            server.server_close()
            thread.join()


if __name__ == "__main__":
    main()
