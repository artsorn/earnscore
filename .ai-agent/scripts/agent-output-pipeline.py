#!/usr/bin/env python3
"""Keep an exact compressed CLI stream while emitting a bounded operational log."""

from __future__ import annotations

import argparse
import collections
import gzip
import hashlib
import json
import re
import sys
from pathlib import Path


IMPORTANT = re.compile(
    r"(error|fatal|fail(?:ed|ure)?|blocked|denied|permission|timed?\s*out|traceback|panic|"
    r"warning|warn:|tokens?\s+(?:used|remaining)|total[_ ]tokens|session id|conversation id|"
    r"validation|test result|not run|capacity|rate.?limit)",
    re.I,
)
DIFF_LINE = re.compile(r"^(?:diff --git |index [0-9a-f]|--- (?:a/|/dev/null)|\+\+\+ (?:b/|/dev/null)|@@ |[+-])")


def parser() -> argparse.ArgumentParser:
    value = argparse.ArgumentParser()
    value.add_argument("--raw-log", required=True)
    value.add_argument("--visible-log", required=True)
    value.add_argument("--max-lines", type=int, default=1200)
    value.add_argument("--max-bytes", type=int, default=160_000)
    value.add_argument("--head-lines", type=int, default=100)
    value.add_argument("--tail-lines", type=int, default=120)
    value.add_argument("--diff-lines", type=int, default=160)
    return value


def raw_writer(path: Path):
    path.parent.mkdir(parents=True, exist_ok=True)
    mode = "ab" if path.exists() else "wb"
    if path.suffix == ".gz":
        return gzip.open(path, mode, compresslevel=6)
    return path.open(mode)


def compact_json_line(text: str) -> tuple[bool, list[str]]:
    try:
        data = json.loads(text)
    except Exception:
        return False, []
    if not isinstance(data, dict):
        return False, []
    event_type = str(data.get("type") or "")
    emitted: list[str] = []
    if event_type in {"thread.started", "session", "session.started"}:
        identifier = data.get("thread_id") or data.get("session_id") or data.get("id")
        if identifier:
            emitted.append(f"{event_type}: {identifier}")
    if event_type in {"error", "turn.failed"}:
        emitted.append(f"{event_type}: {data.get('message') or data.get('error') or text}")
    item = data.get("item")
    if isinstance(item, dict):
        item_type = str(item.get("type") or "")
        if item_type in {"agent_message", "assistant_message"}:
            message = item.get("text") or item.get("content")
            if isinstance(message, str) and message.strip():
                emitted.append(message.strip())
        elif item_type in {"error", "command_execution"}:
            detail = item.get("message") or item.get("status") or item.get("exit_code")
            if item_type == "error" or detail not in {None, 0, "0", "completed"}:
                emitted.append(f"{item_type}: {detail}")
    usage = data.get("usage")
    if isinstance(usage, dict):
        fields = []
        for name in ("input_tokens", "cached_input_tokens", "output_tokens", "total_tokens"):
            if usage.get(name) is not None:
                fields.append(f"{name}={usage[name]}")
        if fields:
            emitted.append("token usage: " + " ".join(fields))
    return True, emitted


def main() -> int:
    args = parser().parse_args()
    raw_path = Path(args.raw_log)
    visible_path = Path(args.visible_log)
    visible_path.parent.mkdir(parents=True, exist_ok=True)

    try:
        selected: list[str] = visible_path.read_text(encoding="utf-8", errors="replace").splitlines()
    except OSError:
        selected = []
    tail: collections.deque[str] = collections.deque(maxlen=max(0, args.tail_lines))
    fingerprints: set[str] = set()
    input_lines = input_bytes = diff_seen = repeated = omitted = 0
    selected_bytes = sum(len(line.encode("utf-8", "replace")) + 1 for line in selected)
    json_stream = False

    def add(value: str, force: bool = False) -> None:
        nonlocal selected_bytes, repeated, omitted
        value = value.rstrip("\r\n")
        if not value:
            return
        digest = hashlib.sha256(value.encode("utf-8", "replace")).hexdigest()
        if digest in fingerprints:
            repeated += 1
            return
        encoded = len(value.encode("utf-8", "replace")) + 1
        if not force and (len(selected) >= args.max_lines or selected_bytes + encoded > args.max_bytes):
            omitted += 1
            return
        fingerprints.add(digest)
        selected.append(value)
        selected_bytes += encoded

    fingerprints.update(hashlib.sha256(line.encode("utf-8", "replace")).hexdigest() for line in selected)
    with raw_writer(raw_path) as raw:
        for payload in sys.stdin.buffer:
            raw.write(payload)
            input_lines += 1
            input_bytes += len(payload)
            text = payload.decode("utf-8", "replace").rstrip("\r\n")
            is_json, json_lines = compact_json_line(text)
            if is_json:
                json_stream = True
                for line in json_lines:
                    add(line, force=IMPORTANT.search(line) is not None)
                continue
            if json_stream:
                if IMPORTANT.search(text):
                    add(text, force=True)
                continue
            tail.append(text)
            is_diff = DIFF_LINE.search(text) is not None
            if is_diff:
                diff_seen += 1
                if diff_seen > args.diff_lines:
                    omitted += 1
                    continue
            if input_lines <= args.head_lines or IMPORTANT.search(text):
                add(text, force=IMPORTANT.search(text) is not None)

    tail_added = 0
    for line in tail:
        before = len(selected)
        add(line)
        tail_added += len(selected) - before

    summary = (
        f"[output-pipeline] raw={raw_path} input_lines={input_lines} input_bytes={input_bytes} "
        f"visible_lines={len(selected)} visible_bytes={selected_bytes} omitted={omitted} "
        f"repeated={repeated} diff_lines_seen={diff_seen} tail_added={tail_added}"
    )
    selected.append(summary)
    rendered = "\n".join(selected).rstrip() + "\n"
    visible_path.write_text(rendered, encoding="utf-8")
    sys.stdout.write(rendered)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
