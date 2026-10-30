#!/usr/bin/env python3
"""Loopback-only conformance service, also the shared service process/IO module.

This is a deliberately independent example API, not an SDK wire protocol.
Run with an explicit new/marked --workspace; see scenarios.json for routes.
"""
from __future__ import annotations

import argparse
import copy
import hashlib
import io
import json
import math
import os
from pathlib import Path
import re
import signal
import socket
import struct
import sys
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlsplit
import uuid
import wave
import zlib

SCENARIOS = frozenset({"normal", "error", "rate_limit", "disconnect", "slow", "duplicate_usage", "partial_output", "acceptance_unknown"})
SERVICE = "bgx-sdk-sync-api"
MARKER = ".bgx-sdk-conformance.json"
STATE = "state.json"


class ApiError(Exception):
    def __init__(self, status: int, code: str, message: str):
        super().__init__(message)
        self.status, self.code, self.message = status, code, message


def require(condition: bool, message: str, code: str = "INVALID_REQUEST") -> None:
    if not condition:
        raise ApiError(400, code, message)


def integer(value, field: str, minimum: int = 0) -> int:
    require(type(value) is int and value >= minimum, f"{field} must be an integer >= {minimum}")
    return value


def check_scenario(value) -> str:
    require(isinstance(value, str) and value in SCENARIOS, "Unknown scenario; use the published scenario enum")
    return value


def strict_float(value):
    number = float(value)
    if not math.isfinite(number):
        raise ValueError("Nonfinite JSON number")
    return number


def reject_constant(_value):
    raise ValueError("Nonstandard JSON constant")


def safe_regular(path: Path) -> None:
    metadata = path.lstat()
    require(not path.is_symlink() and not (getattr(metadata, "st_file_attributes", 0) & 0x400), "Links and reparse points are forbidden")
    require(path.is_file(), "Expected a regular file")


def sync_directory(path: Path) -> None:
    if os.name != "nt":
        descriptor = os.open(path, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)


def atomic_json(path: Path, value: dict) -> None:
    """Durable state precedes acknowledgment, including acceptance_unknown."""
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8", newline="\n", dir=path.parent, prefix=".state-", delete=False) as output:
            temporary = Path(output.name)
            json.dump(value, output, sort_keys=True, ensure_ascii=False, separators=(",", ":"))
            output.write("\n")
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
        temporary = None
        sync_directory(path.parent)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


class WorkspaceLock:
    """OS-owned lock releases on process death, so restart needs no stale-lock fix."""
    def __init__(self, path: Path):
        if os.path.lexists(path):
            safe_regular(path)
        self.file = path.open("a+b")
        self.file.seek(0, os.SEEK_END)
        if self.file.tell() == 0:
            self.file.write(b"\0")
            self.file.flush()
        self.file.seek(0)
        try:
            if os.name == "nt":
                import msvcrt
                msvcrt.locking(self.file.fileno(), msvcrt.LK_NBLCK, 1)
            else:
                import fcntl
                fcntl.flock(self.file.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BaseException:
            self.file.close()
            raise ApiError(409, "WORKSPACE_BUSY", "Another service process owns the workspace") from None

    def close(self):
        if not self.file.closed:
            self.file.close()


class Store:
    def __init__(self, path: Path, service: str):
        self.root = path.absolute()
        if not self.root.exists():
            self.root.mkdir(parents=False)
        metadata = self.root.lstat()
        require(self.root.is_dir() and not self.root.is_symlink() and not (getattr(metadata, "st_file_attributes", 0) & 0x400), "Workspace must be a real directory")
        self.root = self.root.resolve(strict=True)
        marker = self.root / MARKER
        if not marker.exists():
            require(not any(self.root.iterdir()), "Workspace must be new/empty or marked by this conformance service", "UNMARKED_WORKSPACE")
        else:
            safe_regular(marker)
        self.process_lock = WorkspaceLock(self.root / ".service.lock")
        try:
            if marker.exists():
                with marker.open(encoding="utf-8") as source:
                    identity = json.load(source)
                require(isinstance(identity, dict), "Workspace marker must be an object")
                require(identity.get("format") == 1 and identity.get("service") == service and isinstance(identity.get("workspaceId"), str), "Workspace belongs to a different service or format")
            else:
                identity = {"format": 1, "service": service, "workspaceId": str(uuid.uuid4())}
                atomic_json(marker, identity)
            self.identity = identity
            self.path = self.root / STATE
            self.lock = threading.RLock()
            self.media_dir = self.root / "media"
            if self.media_dir.exists():
                require(self.media_dir.is_dir() and not self.media_dir.is_symlink(), "Media directory cannot be a link")
                require(not (getattr(self.media_dir.lstat(), "st_file_attributes", 0) & 0x400), "Media directory cannot be a reparse point")
            else:
                self.media_dir.mkdir()
            if self.path.exists():
                safe_regular(self.path)
                with self.path.open(encoding="utf-8") as source:
                    self.data = json.load(source)
                require(isinstance(self.data, dict), "Durable service state must be an object")
                require(self.data.get("identity") == identity and self.data.get("format") == 1, "Durable service state identity mismatch")
                require(type(self.data.get("submitCount")) is int and isinstance(self.data.get("jobs"), dict) and isinstance(self.data.get("media"), dict), "Durable service state is malformed")
            else:
                self.data = {"format": 1, "identity": identity, "submitCount": 0, "jobs": {}, "media": {}}
                atomic_json(self.path, self.data)
        except BaseException:
            self.process_lock.close()
            raise

    def read(self):
        with self.lock:
            return copy.deepcopy(self.data)

    def update(self, change):
        with self.lock:
            updated = copy.deepcopy(self.data)
            result = change(updated)
            atomic_json(self.path, updated)
            self.data = updated
            return copy.deepcopy(result)

    def media(self, identifier: str):
        require(re.fullmatch(r"[a-f0-9]{32}", identifier) is not None, "Invalid media identifier")
        with self.lock:
            record = copy.deepcopy(self.data["media"].get(identifier))
        if record is None:
            raise ApiError(404, "MEDIA_NOT_FOUND", "Media does not exist in this workspace")
        path = self.media_dir / identifier
        safe_regular(path)
        return record, path

    def commit_media(self, temporary: Path, digest: str, size: int, name: str, mime: str):
        identifier = uuid.uuid4().hex
        target = self.media_dir / identifier
        os.replace(temporary, target)
        sync_directory(self.media_dir)
        record = {"id": identifier, "sha256": digest, "bytes": size, "fileName": name, "mimeType": mime}
        try:
            self.update(lambda data: data["media"].__setitem__(identifier, record))
        except BaseException:
            target.unlink(missing_ok=True)
            raise
        return record

    def add_bytes(self, data: bytes, name: str, mime: str):
        temporary = None
        try:
            with tempfile.NamedTemporaryFile(dir=self.media_dir, prefix=".upload-", delete=False) as output:
                temporary = Path(output.name)
                output.write(data)
                output.flush()
                os.fsync(output.fileno())
            record = self.commit_media(temporary, hashlib.sha256(data).hexdigest(), len(data), name, mime)
            temporary = None
            return record
        finally:
            if temporary is not None:
                temporary.unlink(missing_ok=True)

    def submit_job(self, input_data, scenario, polls, outputs, client_id=None):
        identifier = uuid.uuid4().hex
        input_digest = hashlib.sha256(json.dumps(input_data, sort_keys=True, ensure_ascii=False, separators=(",", ":")).encode()).hexdigest()
        def change(data):
            data["submitCount"] += 1
            job = {"jobId": identifier, "status": "queued", "scenario": scenario, "pollsUntilReady": polls,
                   "queryCount": 0, "cancelCount": 0, "submitOrdinal": data["submitCount"],
                   "inputSha256": input_digest, "clientRequestId": client_id, "artifacts": outputs}
            data["jobs"][identifier] = job
            return job
        return self.update(change)

    def job(self, identifier, advance=False, cancel=False):
        require(re.fullmatch(r"[a-f0-9]{32}", identifier) is not None, "Invalid job identifier")
        def change(data):
            job = data["jobs"].get(identifier)
            if job is None:
                raise ApiError(404, "JOB_NOT_FOUND", "Job does not exist in this workspace")
            terminal = job["status"] in ("succeeded", "failed", "cancelled")
            if cancel:
                job["cancelCount"] += 1
                if not terminal:
                    job["status"] = "cancelled"
            elif advance:
                job["queryCount"] += 1
                if not terminal:
                    job["status"] = "running"
                    if job["queryCount"] >= job["pollsUntilReady"]:
                        job["status"] = "failed" if job["scenario"] == "partial_output" else "succeeded"
            return job
        return self.update(change)


def png_fixture() -> bytes:
    def chunk(kind, body):
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 1, 1, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(b"\x00\x20\x80\xc0\xff")) + chunk(b"IEND", b"")


def wav_fixture() -> bytes:
    output = io.BytesIO()
    with wave.open(output, "wb") as audio:
        audio.setnchannels(1)
        audio.setsampwidth(2)
        audio.setframerate(8000)
        audio.writeframes(b"\0\0" * 800)
    return output.getvalue()


def check_fixture_signature(data: bytes, extension: str) -> None:
    # Container signatures catch accidental wrong files. Audio/video decoding
    # and actual geometry/duration remain the harness's independent ffprobe job.
    if extension == ".mp4":
        valid = len(data) >= 12 and data[4:8] == b"ftyp" and 8 <= int.from_bytes(data[:4], "big") <= len(data)
    elif extension == ".webm":
        valid = data.startswith(b"\x1aE\xdf\xa3")
    elif extension == ".wav":
        try:
            with wave.open(io.BytesIO(data), "rb") as source:
                valid = source.getnchannels() > 0 and source.getnframes() > 0 and source.getframerate() > 0
        except (wave.Error, EOFError):
            valid = False
    elif extension == ".mp3":
        valid = data.startswith(b"ID3") or (len(data) >= 2 and data[0] == 0xff and data[1] & 0xe0 == 0xe0)
    elif extension == ".ogg":
        valid = data.startswith(b"OggS")
    else:
        valid = extension == ".flac" and data.startswith(b"fLaC")
    require(valid, "Explicit fixture container signature does not match its extension", "INVALID_FIXTURE")


class ApiServer(ThreadingHTTPServer):
    daemon_threads = False
    block_on_close = True

    def __init__(self, address, handler, store, args):
        self.store, self.options = store, args
        self.stopping = threading.Event()
        self.connections = set()
        self.connection_lock = threading.Lock()
        self.fixtures = {}
        super().__init__(address, handler)
        bound_host, bound_port = self.server_address[:2]
        self.origin = f"http://[{bound_host}]:{bound_port}" if ":" in bound_host else f"http://{bound_host}:{bound_port}"

    def close_connections(self):
        self.stopping.set()
        with self.connection_lock:
            connections = list(self.connections)
        for connection in connections:
            try:
                connection.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            try:
                connection.close()
            except OSError:
                pass

    def output(self, kind):
        if kind not in self.fixtures:
            raise ApiError(422, "FIXTURE_NOT_CONFIGURED", f"No real {kind} fixture is configured; supply its explicit fixture option")
        record = self.fixtures[kind]
        return {"kind": kind, "url": self.origin + "/media/" + record["id"], **record}

    def job_outputs(self, job):
        if job["status"] not in ("succeeded", "failed"):
            return []
        selected = job["artifacts"][:1] if job["status"] == "failed" else job["artifacts"]
        return [{**record, "url": self.origin + "/media/" + record["id"]} for record in selected]


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "BGXConformance/1"
    sys_version = ""

    def setup(self):
        super().setup()
        self.connection.settimeout(10)
        with self.server.connection_lock:
            self.server.connections.add(self.connection)

    def finish(self):
        try:
            super().finish()
        finally:
            with self.server.connection_lock:
                self.server.connections.discard(self.connection)

    def log_message(self, *_args):
        # BaseHTTPRequestHandler logs request lines/headers on parse failures.
        # Never log bearer tokens, credentials, raw headers, or request bodies.
        pass

    def send_error(self, code, message=None, explain=None):
        self.reply(code, {"error": {"code": "HTTP_REQUEST_ERROR", "message": "HTTP request could not be parsed"}})

    def reply(self, status, body, headers=None):
        encoded = json.dumps(body, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(encoded)))
        self.send_header("Connection", "close")
        for name, value in (headers or {}).items():
            self.send_header(name, value)
        self.end_headers()
        self.close_connection = True
        self.wfile.write(encoded)

    def body_size(self, limit):
        require(not self.headers.get("Transfer-Encoding"), "Use an explicit Content-Length; request transfer encodings are unsupported")
        values = self.headers.get_all("Content-Length", [])
        require(len(values) == 1 and re.fullmatch(r"[0-9]+", values[0]) is not None, "Exactly one valid Content-Length is required")
        size = int(values[0])
        if size > limit:
            raise ApiError(413, "BODY_TOO_LARGE", "Request exceeds the explicitly configured service byte budget")
        return size

    def read_json(self):
        size = self.body_size(self.server.options.max_body_bytes)
        source = self.rfile.read(size)
        require(len(source) == size, "Request body ended before Content-Length")
        try:
            value = json.loads(source, parse_float=strict_float, parse_constant=reject_constant)
        except (ValueError, UnicodeError):
            raise ApiError(400, "INVALID_JSON", "Request must contain valid UTF-8 JSON") from None
        require(isinstance(value, dict), "JSON request must be an object")
        return value

    def disconnect(self, body=b'{"accepted":'):
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body) + 128))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(body)
        self.wfile.flush()
        self.close_connection = True
        try:
            self.connection.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass

    def inject_before(self, scenario, payload):
        if scenario == "error":
            raise ApiError(422, "FIXTURE_REJECTED", "The requested scenario rejects the operation")
        if scenario == "rate_limit":
            self.reply(429, {"error": {"code": "FIXTURE_RATE_LIMIT", "message": "Scenario rate limit"}}, {"Retry-After": "1"})
            return False
        if scenario == "slow":
            delay = integer(payload.get("delayMs", 250), "delayMs")
            if self.server.stopping.wait(delay / 1000):
                raise ConnectionAbortedError("service stopping")
        return True

    def upload(self, query):
        scenario = check_scenario(query.get("scenario", ["normal"])[0])
        require(scenario not in ("duplicate_usage", "partial_output"), "Upload does not produce usage snapshots or generated partial outputs")
        delay = query.get("delayMs", ["250"])[0]
        require(delay.isascii() and delay.isdecimal(), "delayMs must be a nonnegative decimal integer")
        if not self.inject_before(scenario, {"delayMs": integer(int(delay), "delayMs")}):
            return
        size = self.body_size(self.server.options.max_upload_bytes)
        name = self.headers.get("X-File-Name", "upload.bin")
        require(name not in ("", ".", "..") and not any(c in name for c in ("/", "\\", "\r", "\n", "\0")), "X-File-Name must be a plain file name")
        mime = self.headers.get("Content-Type", "application/octet-stream")
        require(re.fullmatch(r"[A-Za-z0-9.+-]+/[A-Za-z0-9.+-]+", mime) is not None, "Content-Type must be a plain MIME type")
        temporary = None
        try:
            with tempfile.NamedTemporaryFile(dir=self.server.store.media_dir, prefix=".upload-", delete=False) as output:
                temporary = Path(output.name)
                digest = hashlib.sha256()
                remaining = size
                while remaining:
                    chunk = self.rfile.read(min(65536, remaining))
                    require(bool(chunk), "Upload ended before Content-Length")
                    output.write(chunk)
                    digest.update(chunk)
                    remaining -= len(chunk)
                output.flush()
                os.fsync(output.fileno())
            record = self.server.store.commit_media(temporary, digest.hexdigest(), size, name, mime)
            temporary = None
        finally:
            if temporary is not None:
                temporary.unlink(missing_ok=True)
        if scenario in ("disconnect", "acceptance_unknown"):
            self.disconnect()
        else:
            self.reply(201, {"uploadId": record["id"], "mediaUrl": self.server.origin + "/media/" + record["id"], **record})

    def common(self, method, path, query):
        if method == "GET" and path == "/health":
            self.reply(200, {"ok": True, "service": self.server.store.identity["service"], "workspaceId": self.server.store.identity["workspaceId"]})
        elif method == "GET" and path == "/state":
            self.reply(200, self.server.store.read())
        elif method == "POST" and path == "/uploads":
            self.upload(query)
        elif method == "GET" and path.startswith("/media/"):
            record, source = self.server.store.media(path.removeprefix("/media/"))
            self.send_response(200)
            self.send_header("Content-Type", record["mimeType"])
            self.send_header("Content-Length", str(record["bytes"]))
            self.send_header("Connection", "close")
            self.end_headers()
            self.close_connection = True
            with source.open("rb") as media:
                while not self.server.stopping.is_set():
                    chunk = media.read(65536)
                    if not chunk:
                        break
                    self.wfile.write(chunk)
        else:
            return False
        return True

    def route(self, method, path, query):
        if self.common(method, path, query):
            return
        if method != "POST" or path not in ("/generate", "/stream"):
            raise ApiError(404, "ROUTE_NOT_FOUND", "Route is not part of this fixture API")
        body = self.read_json()
        scenario = check_scenario(body.get("scenario", "normal"))
        require(scenario != "acceptance_unknown", "acceptance_unknown is available only for durable job submission and uploads")
        if not self.inject_before(scenario, body):
            return
        input_data = body.get("input", {})
        require(isinstance(input_data, dict), "input must be an object")
        prompt = input_data.get("prompt", "")
        require(isinstance(prompt, str), "input.prompt must be a string")
        kinds = input_data.get("outputKinds", [])
        require(isinstance(kinds, list) and all(kind in ("image", "audio", "video") for kind in kinds), "input.outputKinds must be an array of image/audio/video")
        outputs = [self.server.output(kind) for kind in kinds]
        usage = {"input_tokens": 3, "output_tokens": 2, "total_tokens": 5, "credits": "0.25", "snapshot_id": "final"}
        if path == "/generate":
            if scenario == "disconnect":
                self.disconnect()
                return
            result = {"text": "echo: " + prompt, "data": {"echo": input_data, "items": [1, 2, 3]}, "outputs": outputs, "usage": usage, "partial": scenario == "partial_output"}
            if scenario == "duplicate_usage":
                result["usageSnapshots"] = [usage, usage]
            if scenario == "partial_output":
                result["outputs"] = outputs[:1]
                result["error"] = {"code": "FIXTURE_PARTIAL", "message": "Partial output scenario did not complete"}
            self.reply(200, result)
        else:
            self.stream(query.get("format", ["sse"])[0], scenario, body, prompt, usage, outputs)

    def stream(self, format_name, scenario, body, prompt, usage, outputs):
        require(format_name in ("sse", "ndjson", "bytes"), "format must be sse, ndjson, or bytes")
        events = [{"type": "text", "delta": "echo: "}, {"type": "text", "delta": prompt}, {"type": "usage", "snapshot": usage}]
        if scenario == "duplicate_usage":
            events.append({"type": "usage", "snapshot": usage})
        events.extend({"type": "output", "media": output} for output in (outputs[:1] if scenario == "partial_output" else outputs))
        if scenario == "partial_output":
            events.append({"type": "failure", "error": {"code": "FIXTURE_PARTIAL", "message": "Stream ended with partial output"}})
        else:
            events.append({"type": "done", "text": "echo: " + prompt})
        interval = integer(body.get("chunkDelayMs", 20), "chunkDelayMs") / 1000
        self.send_response(200)
        self.send_header("Content-Type", {"sse": "text/event-stream", "ndjson": "application/x-ndjson", "bytes": "application/octet-stream"}[format_name])
        self.send_header("Transfer-Encoding", "chunked")
        self.send_header("Connection", "close")
        self.end_headers()
        self.close_connection = True
        for index, event in enumerate(events, 1):
            if self.server.stopping.is_set():
                return
            item = {"sequence": index, **event}
            encoded = json.dumps(item, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
            if format_name == "sse":
                encoded = b"id: " + str(index).encode() + b"\nevent: " + event["type"].encode() + b"\ndata: " + encoded + b"\n\n"
            elif format_name == "ndjson":
                encoded += b"\n"
            else:
                encoded = b"\x00\xff" + encoded + b"\x80"
            self.wfile.write(f"{len(encoded):X}\r\n".encode() + encoded + b"\r\n")
            self.wfile.flush()
            if scenario == "disconnect":
                # Deliberately omit the final chunk; clients must see truncation.
                return
            if self.server.stopping.wait(interval):
                return
        self.wfile.write(b"0\r\n\r\n")
        self.wfile.flush()

    def dispatch(self, method):
        try:
            target = urlsplit(self.path)
            require(not target.scheme and not target.netloc, "Use origin-form request targets")
            self.route(method, target.path, parse_qs(target.query, keep_blank_values=True))
        except ApiError as error:
            self.reply(error.status, {"error": {"code": error.code, "message": error.message}})
        except (BrokenPipeError, ConnectionError, TimeoutError, socket.timeout):
            self.close_connection = True
        except Exception as error:
            # No exception text: a parser may include a supplied credential/body.
            print(json.dumps({"event": "service_error", "type": type(error).__name__}), file=sys.stderr, flush=True)
            self.reply(500, {"error": {"code": "SERVICE_IO_ERROR", "message": "Conformance service could not complete the operation"}})

    def do_GET(self):
        self.dispatch("GET")

    def do_POST(self):
        self.dispatch("POST")


def self_test(service, handler):
    """Exercise the actual handler over a random loopback port, no subprocess."""
    import http.client
    with tempfile.TemporaryDirectory(prefix="bgx-sdk-service-self-test-") as temporary:
        store = Store(Path(temporary) / "workspace", service)
        args = argparse.Namespace(max_body_bytes=1048576, max_upload_bytes=67108864)
        server = ApiServer(("127.0.0.1", 0), handler, store, args)
        server.fixtures["image"] = store.add_bytes(png_fixture(), "fixture.png", "image/png")
        server.fixtures["audio"] = store.add_bytes(wav_fixture(), "fixture.wav", "audio/wav")
        thread = threading.Thread(target=lambda: server.serve_forever(poll_interval=0.01), daemon=True)
        thread.start()
        def request(method, path, body=None):
            connection = http.client.HTTPConnection("127.0.0.1", server.server_address[1], timeout=3)
            try:
                connection.request(method, path, None if body is None else json.dumps(body).encode(), {"Content-Type":"application/json"})
                response = connection.getresponse()
                return response.status, json.loads(response.read())
            finally:
                connection.close()
        try:
            status, health = request("GET", "/health")
            assert status == 200 and health["service"] == service
            if service == "bgx-sdk-sync-api":
                status, result = request("POST", "/generate", {"input":{"prompt":"self-test","outputKinds":["image"]}})
                assert status == 200 and result["text"] == "echo: self-test" and len(result["outputs"]) == 1
            elif service == "bgx-sdk-async-api":
                status, result = request("POST", "/jobs", {"input":{"outputKinds":["image"]},"pollsUntilReady":0})
                assert status == 202
                status, queried = request("GET", "/jobs/" + result["jobId"])
                assert status == 200 and queried["status"] == "succeeded" and len(queried["outputs"]) == 1
            else:
                status, result = request("POST", "/graph/run", {"nodes":{"17":{"class_type":"TextNode","inputs":{"text":"self-test"}},"42":{"class_type":"ImageNode","inputs":{"prompt":["17",0]}}},"pollsUntilReady":0})
                assert status == 202
                status, queried = request("GET", "/graph/history/" + result["run_token"])
                assert status == 200 and len(queried["history"]["artifacts"]) == 1
            assert request("GET", "/unknown-self-test-route")[0] == 404
        finally:
            server.close_connections(); server.shutdown(); thread.join(timeout=3)
            server.server_close(); store.process_lock.close()
        assert not thread.is_alive()
    assert not Path(temporary).exists()
    print(json.dumps({"scope":"ACTUAL_LOOPBACK_SERVICE_SELF_TEST","service":service,"passed":True,"threadJoined":True,"workspaceRemovedOnExit":True}), flush=True)
    return 0


def run(service=SERVICE, handler=Handler):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workspace", type=Path)
    parser.add_argument("--self-test", action="store_true", help="Run an isolated actual loopback handler self-test")
    parser.add_argument("--host", default="127.0.0.1", choices=("127.0.0.1", "::1", "localhost"))
    parser.add_argument("--port", default=0, type=int)
    parser.add_argument("--max-body-bytes", default=1048576, type=int)
    parser.add_argument("--max-upload-bytes", default=67108864, type=int)
    parser.add_argument("--video-fixture", type=Path)
    parser.add_argument("--audio-fixture", type=Path)
    args = parser.parse_args()
    if args.self_test:
        if len(sys.argv) != 2:
            parser.error("--self-test is exclusive")
        return self_test(service, handler)
    if args.workspace is None:
        parser.error("--workspace is required for service mode")
    if not (0 <= args.port <= 65535) or args.max_body_bytes <= 0 or args.max_upload_bytes <= 0:
        parser.error("Port must be 0..65535 and byte budgets must be positive")
    # localhost is deliberately bound to literal IPv4 loopback, never DNS.
    host = "127.0.0.1" if args.host == "localhost" else args.host
    store = server = None
    try:
        store = Store(args.workspace, service)
        if host == "::1":
            class IPv6Server(ApiServer):
                address_family = socket.AF_INET6
            server = IPv6Server((host, args.port), handler, store, args)
        else:
            server = ApiServer((host, args.port), handler, store, args)
        server.fixtures["image"] = store.add_bytes(png_fixture(), "fixture.png", "image/png")
        server.fixtures["audio"] = store.add_bytes(wav_fixture(), "fixture.wav", "audio/wav")
        for kind, source in (("video", args.video_fixture), ("audio", args.audio_fixture)):
            if source is not None:
                safe_regular(source)
                require(source.stat().st_size <= args.max_upload_bytes, "Explicit fixture exceeds configured upload budget")
                extension = source.suffix.lower()
                mime = {".mp4": "video/mp4", ".webm": "video/webm", ".wav": "audio/wav", ".mp3": "audio/mpeg", ".ogg": "audio/ogg", ".flac": "audio/flac"}.get(extension)
                require(mime is not None and mime.startswith(kind + "/"), "Explicit fixture needs a recognized extension for its media kind")
                fixture_bytes = source.read_bytes()
                check_fixture_signature(fixture_bytes, extension)
                server.fixtures[kind] = store.add_bytes(fixture_bytes, "fixture" + extension, mime)
        def stop(_signum, _frame):
            if not server.stopping.is_set():
                server.close_connections()
                threading.Thread(target=server.shutdown, daemon=True).start()
        signal.signal(signal.SIGINT, stop)
        if hasattr(signal, "SIGTERM"):
            signal.signal(signal.SIGTERM, stop)
        print(json.dumps({"event": "ready", "service": service, "workspaceId": store.identity["workspaceId"], "runId": str(uuid.uuid4()), "host": host, "port": server.server_address[1], "origin": server.origin, "workspace": str(store.root)}, separators=(",", ":")), flush=True)
        server.serve_forever(poll_interval=0.1)
        return 0
    except KeyboardInterrupt:
        return 0
    except (ApiError, OSError, ValueError) as error:
        code = error.code if isinstance(error, ApiError) else "SERVICE_START_FAILED"
        # Startup errors describe test paths/format; never print file contents.
        print(json.dumps({"event": "startup_error", "code": code, "message": error.message if isinstance(error, ApiError) else "Cannot open explicit service workspace, fixture, or loopback listener"}), file=sys.stderr, flush=True)
        return 1
    finally:
        if server is not None:
            server.close_connections()
            server.server_close()
        if store is not None:
            store.process_lock.close()


if __name__ == "__main__":
    raise SystemExit(run())
