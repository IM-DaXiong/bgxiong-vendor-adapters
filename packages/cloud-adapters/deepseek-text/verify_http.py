"""Real local HTTP through the fixed public CLI/worker; never calls the vendor."""
import argparse
import copy
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import subprocess
import threading
import uuid


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for field in ("cli", "sdk", "worker", "package", "output"):
        parser.add_argument("--" + field, required=True, type=Path)
    parser.add_argument('--recorded-response', type=Path)
    args = parser.parse_args()
    for value in vars(args).values():
        if value is not None and not value.is_absolute():
            parser.error("All paths must be absolute")
    args.output.mkdir(exist_ok=False, parents=True)
    scenario = {"name": "normal"}
    calls = []
    original = "中文原文\n\n保留空行与 emoji 🐻"
    success = {"id": "fixture-response", "model": "deepseek-flash", "choices": [{"finish_reason": "stop", "message": {"role": "assistant", "content": "本地HTTP验证", "reasoning_content": "保留思考"}}], "usage": {"prompt_tokens": 0, "completion_tokens": 5, "total_tokens": 5}, "base_resp": {"status_code": 0}}
    if args.recorded_response:
        success = json.loads(args.recorded_response.read_text(encoding='utf-8'))['response']

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            calls.append({"scenario": scenario["name"], "path": self.path, "body": body,
                          "credentialMatched": self.headers.get("Authorization") == "Bearer nonsecret-fixture-credential"})
            name = scenario["name"]
            status = 401 if name == "unauthorized" else 429 if name == "rate-limit" else 200
            value = copy.deepcopy(success)
            if name == "no-usage":
                value.pop("usage")
            if name == "wrong-model":
                value["model"] = "unselected-model"
            if name == "business-error":
                value = {"error": {"code": "fixture-business", "message": "fixture error"}}
            if name == "rate-limit":
                value = {"error": {"code": "fixture-rate-limit"}}
            wire = b"not-json" if name in {"unauthorized", "malformed"} else json.dumps(value, ensure_ascii=False).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(wire)))
            self.end_headers()
            self.wfile.write(wire)

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    origin = "http://127.0.0.1:" + str(server.server_port)
    secret = args.output / "fixture-credential.txt"
    secret.write_text("nonsecret-fixture-credential", encoding="utf-8")
    config = {"bindingId": "deepseek-http-fixture", "bindingRevision": 1, "configurationRef": "deepseek-http-fixture-v1",
              "configuration": {"endpointBaseUrl": origin, "credentialRef": "fixture-credential"},
              "trust": {"publishers": [], "revokedPlugins": [], "revokedVersions": [], "revokedKeys": [], "denylistedHosts": [], "isFresh": False, "supportedFeatures": []},
              "network": {"origins": [origin], "allowPrivate": True, "allowEmbeddedAuthorization": False, "deniedHosts": [], "deniedNetworks": []},
              "credentials": [{"reference": "fixture-credential", "secretFile": str(secret), "publicId": None, "customHeader": None, "modes": ["bearer"], "origins": [origin], "ttlMs": 60000}],
              "media": [], "mediaTtlMs": 60000, "transport": {"handshakeTimeoutMs": 10000, "writeTimeoutMs": 60000, "shutdownTimeoutMs": 10000, "cancelGraceMs": 100}, "probe": None}
    config_file = args.output / "host-config.json"
    config_file.write_text(json.dumps(config), encoding="utf-8")
    workspace = args.output / "workspace"
    records = []

    def invoke(operation, payload, label):
        request = {"protocolVersion": 2, "operation": operation, "profileId": "deepseek-http-fixture", "capabilitySlot": "generic" if operation == "capabilities" else "text", "requestId": str(uuid.uuid4()), "budgetMs": 60000, "payloadJson": json.dumps(payload, ensure_ascii=False), "bindingId": "deepseek-http-fixture", "negotiatedVersion": 2, "enabledFeatures": []}
        source = args.output / (label + ".input.json")
        target = workspace / "receipts" / (label + ".output.json")
        source.write_text(json.dumps(request, ensure_ascii=False), encoding="utf-8")
        command = [str(args.cli), operation, "--sdk", str(args.sdk), "--worker", str(args.worker), "--package", str(args.package), "--workspace", str(workspace), "--config", str(config_file), "--input", str(source), "--output", str(target)]
        if not workspace.exists():
            command.append("--create-workspace")
        run = subprocess.run(command, capture_output=True, text=True, encoding="utf-8", timeout=90)
        (args.output / (label + ".stdout.log")).write_text(run.stdout, encoding="utf-8")
        (args.output / (label + ".stderr.log")).write_text(run.stderr, encoding="utf-8")
        response = json.loads(target.read_text(encoding="utf-8")) if target.exists() else None
        return run, response, command

    try:
        run, response, command = invoke("capabilities", {}, "capabilities")
        assert run.returncode == 0 and response and "dataJson" in response, (run.returncode, run.stderr)
        catalog = json.loads(response["dataJson"])
        operation = catalog["capabilityContracts"][0]["operations"][0]
        payload = {"schemaVersion": 2, "modelId": "deepseek-flash", "capabilityKey": "text.generate", "operationId": operation["id"], "operationVersion": operation["version"], "inputSchemaDigest": operation["inputSchema"]["digest"], "input": {"messages": [{"role": "system", "content": "保持原文"}, {"role": "user", "content": original}], "parameters": {"thinking": False, "maxTokens": 32}}, "media": [], "configuration": config["configuration"], "uploads": []}
        cases = [("normal", None), ("no-usage", None), ("unauthorized", "UNAUTHORIZED"), ("rate-limit", "RATE_LIMITED"), ("business-error", "UPSTREAM_ERROR"), ("wrong-model", "UPSTREAM_ERROR"), ("malformed", "UPSTREAM_ERROR"), ("unsupported-temperature", "INVALID_INPUT")]
        if args.recorded_response:
            cases = [('recorded-vendor-response', None)]
        for name, expected_error in cases:
            scenario["name"] = name
            p = copy.deepcopy(payload)
            if name == "unsupported-temperature":
                p["input"]["parameters"]["temperature"] = 0.7
                p["input"]["parameters"]["thinking"] = True
            before = len(calls)
            run, response, command = invoke("submit", p, name)
            delta = len(calls) - before
            if expected_error:
                error = (response or {}).get("error")
                if error is None:
                    error = json.loads(run.stderr.strip())
                assert error["code"] == expected_error, (name, run.returncode, error)
                assert delta == (0 if name == "unsupported-temperature" else 1), (name, delta)
            else:
                assert run.returncode == 0 and response and "dataJson" in response, (name, run.returncode, run.stderr)
                result = json.loads(response["dataJson"])
                assert result["outputs"][0]["text"] == success['choices'][0]['message']['content']
                assert result["data"]["reasoningContent"] == success['choices'][0]['message'].get('reasoning_content')
                if name == "no-usage":
                    assert result["usage"] is None
                else:
                    assert result["usage"]["measurements"][0]["amount"] == str(success['usage']['prompt_tokens'])
                assert delta == 1
            records.append({"case": name, "exitCode": run.returncode, "expectedError": expected_error, "httpCalls": delta, "command": command})
            (args.output / "cases.json").write_text(json.dumps(records, indent=2), encoding="utf-8")
        for call in calls:
            assert call["path"] == "/chat/completions" and call["credentialMatched"]
            assert call["body"]["model"] == "deepseek-flash"
            assert call["body"]["messages"][1]["content"] == original
            assert call["body"]["max_tokens"] == 32 and call["body"]["thinking"]["type"] == "disabled"
        (args.output / "http-calls.json").write_text(json.dumps(calls, ensure_ascii=False, indent=2), encoding="utf-8")
        print(json.dumps({"status": "PASS", "cases": len(records), "httpCalls": len(calls), "scope": "local real HTTP through fixed SDK CLI/worker; no vendor request"}))
    finally:
        server.shutdown()
        server.server_close()


if __name__ == "__main__":
    main()
