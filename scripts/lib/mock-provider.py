"""Hermetic OpenAI-compatible mock provider for native-shell validation.

Serves POST /v1/chat/completions with scripted SSE streaming:
- A request whose last user message contains "slow" streams word-by-word
  with a 700 ms gap per chunk (reliable cancel window).
- A request that carries tools and has NOT seen a tool result yet returns a
  get_system_overview tool call (exercising the client's tool loop).
- A request whose messages contain a tool result returns a final answer
  quoting the computer name found inside that tool result (proving the tool
  round-trip delivered real data).
- Any other request streams a MOCK_REPLY prefix plus the prompt echo.

Also serves GET /v1/models (empty list) for endpoint discovery probes.
"""

from __future__ import annotations

import json
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = 18080
TOOL_NAME = "get_system_overview"


def sse_chunk(delta: dict, finish: str | None = None) -> bytes:
    choice = {"index": 0, "delta": delta, "finish_reason": finish}
    payload = {"id": "mock", "object": "chat.completion.chunk", "choices": [choice]}
    return f"data: {json.dumps(payload)}\n\n".encode()


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *args):  # silence request logging
        pass

    def do_GET(self):  # noqa: N802
        if self.path.startswith("/v1/models"):
            body = b'{"data":[]}'
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
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
            if role == "tool":
                tool_result_seen = True
                tool_result_text += str(message.get("content") or "")

        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Cache-Control", "no-cache")
        self.end_headers()

        def send(chunk: bytes) -> None:
            self.wfile.write(chunk)
            self.wfile.flush()

        # Tool round-trip: first turn with tools -> request the overview.
        if tools and not tool_result_seen:
            send(sse_chunk({"role": "assistant",
                            "tool_calls": [{"index": 0, "id": "call_mock_1",
                                            "type": "function",
                                            "function": {"name": TOOL_NAME,
                                                         "arguments": ""}}]}))
            send(sse_chunk({}, "tool_calls"))
            send(b"data: [DONE]\n\n")
            return

        if tool_result_seen:
            # Echo the computer name found in the tool result so the test can
            # assert the tool actually fed real data into the answer.
            import re

            match = re.search(r"Computer name: (\S+)", tool_result_text)
            name = match.group(1) if match else "UNKNOWN"
            answer = f"MOCK_TOOL_REPLY: this machine is {name}"
            for word in answer.split(" "):
                send(sse_chunk({"content": word + " "}))
            send(sse_chunk({}, "stop"))
            send(b"data: [DONE]\n\n")
            return

        if "slow" in last_content.lower():
            # Reliable cancel window: long, slow stream.
            for index in range(40):
                send(sse_chunk({"content": f"chunk{index} "}))
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
    server = ThreadingHTTPServer(("127.0.0.1", PORT), Handler)
    print(f"mock provider listening on {PORT}", flush=True)
    server.serve_forever()
