"""Loopback smart HTTP for disposable repositories; streaming, with no root tools.

Latency is per request; the bandwidth limit is shared across concurrent bodies.
The fixture server is a separate process tree from the measured application.
"""
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import subprocess
import tempfile
import threading
import time
from urllib.parse import urlsplit, unquote

import perf_platform


class GitServer(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, root, env, latency_ms=0, bandwidth_mib=0, fault="none"):
        super().__init__(("127.0.0.1", 0), GitHandler)
        self.root, self.env = Path(root).resolve(), env
        self.latency = latency_ms / 1000
        self.bandwidth = bandwidth_mib * 1024 * 1024
        self.fault = fault
        self.lock, self.stop = threading.Lock(), threading.Event()
        self.next_byte_time = 0.0
        self.records, self.children = [], set()
        self.url = f"http://127.0.0.1:{self.server_port}/remote.git"

    def pace(self, count):
        if not self.bandwidth:
            return
        with self.lock:
            now = time.monotonic()
            self.next_byte_time = max(now, self.next_byte_time) + count / self.bandwidth
            delay = self.next_byte_time - now
        if self.stop.wait(delay):
            raise ConnectionAbortedError("fixture stopped")


class GitHandler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.0"

    def log_message(self, *_):
        pass

    def request_body(self, output):
        total = 0
        if self.headers.get("Transfer-Encoding", "").lower() == "chunked":
            while True:
                count = int(self.rfile.readline(128).split(b";", 1)[0].strip(), 16)
                if count == 0:
                    while self.rfile.readline(8192) not in (b"\r\n", b"\n", b""):
                        pass
                    break
                while count:
                    data = self.rfile.read(min(count, 65536))
                    if not data:
                        raise ConnectionError("truncated chunk")
                    output.write(data)
                    self.server.pace(len(data))
                    count -= len(data)
                    total += len(data)
                if self.rfile.read(2) != b"\r\n":
                    raise ValueError("invalid chunk ending")
        else:
            left = int(self.headers.get("Content-Length", "0"))
            while left:
                data = self.rfile.read(min(left, 65536))
                if not data:
                    raise ConnectionError("truncated body")
                output.write(data)
                self.server.pace(len(data))
                left -= len(data)
                total += len(data)
        return total

    def handle_git(self):
        parsed = urlsplit(self.path)
        path = unquote(parsed.path)
        if not path.startswith("/remote.git/") or ".." in path.split("/"):
            self.send_error(404)
            return
        started, sent, received = time.monotonic(), 0, 0
        process, success = None, False
        try:
            self.connection.settimeout(300)
            if self.server.stop.wait(self.server.latency):
                return
            if self.server.fault == "stall" and self.command == "POST":
                self.server.stop.wait(300)
                return
            with tempfile.TemporaryFile(dir=self.server.root) as body, tempfile.TemporaryFile(dir=self.server.root) as errors:
                received = self.request_body(body)
                body.seek(0)
                env = dict(self.server.env, GIT_PROJECT_ROOT=str(self.server.root), GIT_HTTP_EXPORT_ALL="1",
                           REQUEST_METHOD=self.command, PATH_INFO=path, QUERY_STRING=parsed.query,
                           CONTENT_TYPE=self.headers.get("Content-Type", ""), CONTENT_LENGTH=str(received),
                           REMOTE_ADDR="127.0.0.1", REMOTE_USER="performance", SERVER_PROTOCOL="HTTP/1.0")
                if self.headers.get("Git-Protocol"):
                    env["HTTP_GIT_PROTOCOL"] = self.headers["Git-Protocol"]
                process = subprocess.Popen(["git", "http-backend"], stdin=body, stdout=subprocess.PIPE,
                                           stderr=errors, env=env, start_new_session=True)
                with self.server.lock:
                    self.server.children.add(process)
                headers, status = [], 200
                while True:
                    line = process.stdout.readline(65536)
                    if line in (b"\r\n", b"\n", b""):
                        break
                    key, value = line.decode("latin1").strip().split(":", 1)
                    if key.lower() == "status":
                        status = int(value.strip().split()[0])
                    else:
                        headers.append((key, value.strip()))
                self.send_response(status)
                for key, value in headers:
                    self.send_header(key, value)
                self.end_headers()
                while data := process.stdout.read(65536):
                    if self.server.fault == "disconnect" and self.command == "POST":
                        raise ConnectionAbortedError("injected transport disconnect")
                    self.server.pace(len(data))
                    self.wfile.write(data)
                    sent += len(data)
                success = process.wait(timeout=30) == 0 and status == 200
        except (OSError, ValueError, subprocess.SubprocessError):
            self.close_connection = True
        finally:
            if process:
                perf_platform.stop_tree(process)
                process.stdout.close()
                with self.server.lock:
                    self.server.children.discard(process)
            with self.server.lock:
                self.server.records.append({"path": path, "method": self.command, "success": success,
                                            "sent_bytes": sent, "received_bytes": received,
                                            "milliseconds": (time.monotonic() - started) * 1000})

    do_GET = handle_git
    do_POST = handle_git


@contextmanager
def serve_git(root, env, **settings):
    server = GitServer(root, env, **settings)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield server
    finally:
        server.stop.set()
        with server.lock:
            children = list(server.children)
        for process in children:
            perf_platform.stop_tree(process)
        server.shutdown()
        server.server_close()
        thread.join()
