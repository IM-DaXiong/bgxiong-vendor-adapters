#!/usr/bin/env python3
"""Durable asynchronous media API fixture; run with an explicit --workspace.

All submit/query/cancel changes are fsynced before their response. No request is
deduplicated: submitCount exposes an adapter that accidentally resubmits.
"""
from __future__ import annotations

import sys
sys.dont_write_bytecode = True

from sync_api import ApiError, Handler, check_scenario, integer, require, run

SERVICE = "bgx-sdk-async-api"


class AsyncHandler(Handler):
    def route(self, method, path, query):
        if self.common(method, path, query):
            return
        if method == "POST" and path == "/jobs":
            body = self.read_json()
            scenario = check_scenario(body.get("scenario", "normal"))
            if not self.inject_before(scenario, body):
                return
            input_data = body.get("input", {})
            require(isinstance(input_data, dict), "input must be an object")
            kinds = input_data.get("outputKinds", ["image"])
            require(isinstance(kinds, list) and all(kind in ("image", "audio", "video") for kind in kinds), "input.outputKinds must contain only image/audio/video")
            polls = integer(body.get("pollsUntilReady", 1), "pollsUntilReady")
            client_id = body.get("clientRequestId")
            require(client_id is None or isinstance(client_id, str), "clientRequestId must be null or a string")
            # Validate fixture availability before acceptance. Store only stable
            # media IDs, never a port-specific URL that becomes stale on restart.
            outputs = [{key: value for key, value in self.server.output(kind).items() if key != "url"} for kind in kinds]
            job = self.server.store.submit_job(input_data, scenario, polls, outputs, client_id)
            if scenario == "acceptance_unknown":
                # submitCount + job are durable before a deliberately lost reply.
                self.disconnect()
            else:
                self.reply(202, {"jobId": job["jobId"], "status": job["status"], "queryUrl": self.server.origin + "/jobs/" + job["jobId"], "submitOrdinal": job["submitOrdinal"], "inputSha256": job["inputSha256"]})
            return
        if path.startswith("/jobs/"):
            suffix = path.removeprefix("/jobs/")
            if method == "POST" and suffix.endswith("/cancel"):
                # A JSON object is mandatory even for an otherwise empty cancel.
                self.read_json()
                job = self.server.store.job(suffix.removesuffix("/cancel"), cancel=True)
                self.reply(200, {"jobId": job["jobId"], "status": job["status"], "cancelled": job["status"] == "cancelled"})
                return
            if method == "GET" and "/" not in suffix:
                scenario = check_scenario(query.get("scenario", ["normal"])[0])
                require(scenario in ("normal", "error", "rate_limit", "disconnect", "slow"), "Query-only scenario must be normal/error/rate_limit/disconnect/slow; output scenarios are selected at submit")
                delay = query.get("delayMs", ["250"])[0]
                require(delay.isascii() and delay.isdecimal(), "delayMs must be a nonnegative decimal integer")
                options = {"delayMs": integer(int(delay), "delayMs")}
                if not self.inject_before(scenario, options):
                    return
                job = self.server.store.job(suffix, advance=True)
                if scenario == "disconnect" or (job["scenario"] == "disconnect" and job["queryCount"] == 1):
                    self.disconnect()
                    return
                usage = {"billed_seconds": "0.1", "credits": "0.50", "snapshot_id": "job-final"}
                result = {"jobId": job["jobId"], "status": job["status"], "queryCount": job["queryCount"],
                          "submitOrdinal": job["submitOrdinal"], "inputSha256": job["inputSha256"],
                          "outputs": self.server.job_outputs(job), "usage": usage if job["status"] in ("succeeded", "failed") else None}
                if job["scenario"] == "duplicate_usage" and result["usage"] is not None:
                    result["usageSnapshots"] = [usage, usage]
                if job["status"] == "failed":
                    result["error"] = {"code": "FIXTURE_PARTIAL", "message": "Job produced partial output and did not complete"}
                self.reply(200, result)
                return
        raise ApiError(404, "ROUTE_NOT_FOUND", "Route is not part of this asynchronous fixture API")


if __name__ == "__main__":
    raise SystemExit(run(SERVICE, AsyncHandler))
