"""Hermetic OpenAI-compatible mock provider for native-shell validation.

Serves POST /v1/chat/completions with scripted SSE streaming:
- A request whose last user message contains "slow" streams word-by-word
  with a 700 ms gap per chunk (reliable cancel window).
- A request whose current user message mentions "tool", carries tools, and
  has NOT seen a tool result yet verifies the exact canonical ten-tool
  contract and returns a list_remediations tool call (exercising the client's
  closed tool loop without hijacking ordinary turns).
- A request whose messages contain a tool result returns a final answer
  quoting the known open_disk_cleanup catalog ID found inside that result
  (proving the tool round-trip delivered real catalog data).
- Any other request streams a MOCK_REPLY prefix plus the prompt echo.

Also serves GET /v1/models (empty list) for endpoint discovery probes.
"""

from __future__ import annotations

import argparse
import json
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

DEFAULT_PORT = 18080
TOOL_NAME = "list_remediations"
EXPECTED_TOOL_NAMES = (
    "run_diagnostic",
    "search_windows_knowledge",
    "get_scan_summary",
    "request_full_scan",
    "get_detected_issues",
    "compare_with_previous_scan",
    "get_live_stats",
    "list_remediations",
    "list_scan_history",
    "stage_remediation",
)


def sse_chunk(delta: dict, finish: str | None = None) -> bytes:
    choice = {"index": 0, "delta": delta, "finish_reason": finish}
    payload = {
        "id": "mock",
        "object": "chat.completion.chunk",
        "created": 0,
        "model": "mock-model",
        "choices": [choice],
    }
    return f"data: {json.dumps(payload)}\n\n".encode()


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *args):
        # Keep the hermetic evidence useful without recording request bodies,
        # prompts, or authorization headers.
        print(f"{self.command} {self.path} - {fmt % args}", flush=True)

    def do_GET(self):  # noqa: N802
        if self.path.startswith("/v1/models"):
            body = b'{"data":[]}'
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Connection", "close")
            self.close_connection = True
            self.end_headers()
            self.wfile.write(body)

    def do_POST(self):  # noqa: N802
        length = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(length).decode("utf-8", "replace")
        try:
            request = json.loads(raw)
        except json.JSONDecodeError:
            request = {}

        messages = request.get("messages") or []
        tools = request.get("tools") or []
        last_content = ""
        tool_result_seen = False
        tool_result_text = ""
        for message in messages:
            role = message.get("role")
            if role == "user":
                last_content = str(message.get("content") or "")
                # Tool results from an earlier turn must not affect the new
                # user message's scripted behavior.
                tool_result_seen = False
                tool_result_text = ""
            if role == "tool":
                tool_result_seen = True
                tool_result_text += str(message.get("content") or "")

        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Cache-Control", "no-cache")
        self.send_header("Connection", "close")
        self.close_connection = True
        self.end_headers()

        def send(chunk: bytes) -> bool:
            try:
                self.wfile.write(chunk)
                self.wfile.flush()
                return True
            except (BrokenPipeError, ConnectionAbortedError, ConnectionResetError):
                return False

        # Tool round-trip: first turn with tools -> verify the closed catalog
        # and request one deterministic, read-only catalog operation.
        if tools and not tool_result_seen and "tool" in last_content.lower():
            names = tuple(
                str(tool.get("function", {}).get("name") or "")
                for tool in tools
            )
            if names != EXPECTED_TOOL_NAMES:
                expected = ",".join(EXPECTED_TOOL_NAMES)
                actual = ",".join(names)
                answer = (
                    "MOCK_TOOL_CONTRACT_ERROR: expected exact tools "
                    f"[{expected}], received [{actual}]"
                )
                for word in answer.split(" "):
                    send(sse_chunk({"content": word + " "}))
                send(sse_chunk({}, "stop"))
                send(b"data: [DONE]\n\n")
                return
            send(sse_chunk({"role": "assistant",
                            "tool_calls": [{"index": 0, "id": "call_mock_1",
                                            "type": "function",
                                            "function": {"name": TOOL_NAME,
                                                         "arguments": "{}"}}]}))
            send(sse_chunk({}, "tool_calls"))
            send(b"data: [DONE]\n\n")
            return

        if tool_result_seen:
            # Echo a stable catalog ID from the tool result so the test can
            # assert that the tool actually fed native remediation data into
            # the second provider request.
            remediation_id = (
                "open_disk_cleanup"
                if "open_disk_cleanup" in tool_result_text
                else "MISSING_EXPECTED_REMEDIATION"
            )
            answer = f"MOCK_TOOL_REPLY: vetted remediation {remediation_id}"
            for word in answer.split(" "):
                send(sse_chunk({"content": word + " "}))
            send(sse_chunk({}, "stop"))
            send(b"data: [DONE]\n\n")
            return

        if "slow" in last_content.lower():
            # Reliable cancel window: long, slow stream.
            for index in range(40):
                if not send(sse_chunk({"content": f"chunk{index} "})):
                    return
                time.sleep(0.7)
            send(sse_chunk({}, "stop"))
            send(b"data: [DONE]\n\n")
            return

        answer = f"MOCK_REPLY: you said {last_content!r}"
        for word in answer.split(" "):
            send(sse_chunk({"content": word + " "}))
        send(sse_chunk({}, "stop"))
        send(b"data: [DONE]\n\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=DEFAULT_PORT)
    args = parser.parse_args()
    if not 1 <= args.port <= 65535:
        parser.error("--port must be between 1 and 65535")
    server = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    print(f"mock provider listening on {args.port}", flush=True)
    server.serve_forever()
