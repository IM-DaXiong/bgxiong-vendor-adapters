#!/usr/bin/env python3
"""Graph-shaped conformance API. All node names/mapping belong to this example.

The public SDK has no knowledge of these graph nodes or history field names.
"""
from __future__ import annotations

import sys
sys.dont_write_bytecode = True

from sync_api import ApiError, Handler, check_scenario, integer, require, run

SERVICE = "bgx-sdk-workflow-api"
NODE_OUTPUTS = {"ImageNode": "image", "AudioNode": "audio", "VideoNode": "video", "SpeechNode": "video", "EditNode": "video"}
PHASES = {"queued": "waiting", "running": "executing", "succeeded": "finished", "failed": "errored", "cancelled": "interrupted"}


class WorkflowHandler(Handler):
    def graph_outputs(self, nodes):
        require(isinstance(nodes, dict) and bool(nodes), "nodes must be a nonempty node map")
        outputs = []
        for identifier, node in nodes.items():
            require(isinstance(identifier, str) and bool(identifier) and isinstance(node, dict), "Node IDs must be nonempty strings and nodes must be objects")
            class_name = node.get("class_type")
            require(isinstance(class_name, str) and class_name in {"TextNode", "UploadedInput", *NODE_OUTPUTS}, "Unknown class_type; use the documented workflow fixture nodes")
            inputs = node.get("inputs")
            require(isinstance(inputs, dict), "Each node requires an inputs object")
            if class_name == "TextNode":
                require(isinstance(inputs.get("text"), str), "TextNode.inputs.text must be a string")
            elif class_name == "UploadedInput":
                require(isinstance(inputs.get("upload_id"), str), "UploadedInput.inputs.upload_id is required")
                self.server.store.media(inputs["upload_id"])
            else:
                reference = inputs.get("prompt")
                require(isinstance(reference, list) and len(reference) == 2 and isinstance(reference[0], str) and type(reference[1]) is int and reference[1] == 0, "Output node inputs.prompt must be [TextNode ID, 0]")
                text_node = nodes.get(reference[0])
                require(isinstance(text_node, dict) and text_node.get("class_type") == "TextNode", "Output prompt must reference a TextNode in this graph")
                output = {key: value for key, value in self.server.output(NODE_OUTPUTS[class_name]).items() if key != "url"}
                outputs.append({**output, "nodeId": identifier})
        return outputs

    def route(self, method, path, query):
        if self.common(method, path, query):
            return
        if method == "POST" and path == "/graph/run":
            body = self.read_json()
            scenario = check_scenario(body.get("scenario", "normal"))
            if not self.inject_before(scenario, body):
                return
            nodes = body.get("nodes")
            outputs = self.graph_outputs(nodes)
            polls = integer(body.get("pollsUntilReady", 1), "pollsUntilReady")
            client_id = body.get("clientRequestId")
            require(client_id is None or isinstance(client_id, str), "clientRequestId must be null or a string")
            job = self.server.store.submit_job({"nodes": nodes}, scenario, polls, outputs, client_id)
            if scenario == "acceptance_unknown":
                self.disconnect()
            else:
                self.reply(202, {"run_token": job["jobId"], "phase": PHASES[job["status"]], "poll_path": "/graph/history/" + job["jobId"], "accepted_number": job["submitOrdinal"]})
            return
        if method == "POST" and path.startswith("/graph/interrupt/"):
            self.read_json()
            job = self.server.store.job(path.removeprefix("/graph/interrupt/"), cancel=True)
            self.reply(200, {"run_token": job["jobId"], "phase": PHASES[job["status"]], "interrupted": job["status"] == "cancelled"})
            return
        if method == "GET" and path.startswith("/graph/history/"):
            scenario = check_scenario(query.get("scenario", ["normal"])[0])
            require(scenario in ("normal", "error", "rate_limit", "disconnect", "slow"), "Query-only scenario must be normal/error/rate_limit/disconnect/slow; output scenarios are selected at submit")
            delay = query.get("delayMs", ["250"])[0]
            require(delay.isascii() and delay.isdecimal(), "delayMs must be a nonnegative decimal integer")
            if not self.inject_before(scenario, {"delayMs": integer(int(delay), "delayMs")}):
                return
            job = self.server.store.job(path.removeprefix("/graph/history/"), advance=True)
            if scenario == "disconnect" or (job["scenario"] == "disconnect" and job["queryCount"] == 1):
                self.disconnect()
                return
            artifacts = [{"node_id": output["nodeId"], "category": output["kind"], "download_url": output["url"], "digest": output["sha256"], "length": output["bytes"], "content_type": output["mimeType"]} for output in self.server.job_outputs(job)]
            metering = {"units": "0.50", "snapshot": "wf-final"} if job["status"] in ("succeeded", "failed") else None
            history = {"artifacts": artifacts, "metering": metering, "analysis": {"input_digest": job["inputSha256"], "poll_count": job["queryCount"]}}
            if job["scenario"] == "duplicate_usage" and metering is not None:
                history["metering_updates"] = [metering, metering]
            fault = {"tag": "WF_PARTIAL", "description": "Graph produced partial output and did not complete"} if job["status"] == "failed" else None
            self.reply(200, {"run_token": job["jobId"], "phase": PHASES[job["status"]], "history": history, "fault": fault})
            return
        raise ApiError(404, "ROUTE_NOT_FOUND", "Route is not part of this workflow fixture API")


if __name__ == "__main__":
    raise SystemExit(run(SERVICE, WorkflowHandler))
