#!/usr/bin/env python3
"""Parse per-turn usage from raw CLI output and append one accounting row."""

from __future__ import annotations

import argparse
import datetime
import gzip
import json
import re
from pathlib import Path


def open_text(path: Path):
    if path.suffix == ".gz":
        return gzip.open(path, "rt", encoding="utf-8", errors="replace")
    return path.open(encoding="utf-8", errors="replace")


def nested_usage(value):
    found = []
    stack = [value]
    while stack:
        item = stack.pop()
        if isinstance(item, dict):
            keys = set(item)
            if keys & {"input_tokens", "output_tokens", "total_tokens"}:
                found.append(item)
            stack.extend(item.values())
        elif isinstance(item, list):
            stack.extend(item)
    return found


def parse_usage(path: Path):
    last = None
    text_tail = ""
    with open_text(path) as stream:
        for line in stream:
            text_tail = (text_tail + line)[-2_000_000:]
            try:
                data = json.loads(line)
            except Exception:
                continue
            for usage in nested_usage(data):
                last = usage
    if last is not None:
        input_tokens = int(last.get("input_tokens") or 0)
        output_tokens = int(last.get("output_tokens") or 0)
        total = int(last.get("total_tokens") or input_tokens + output_tokens)
        return {
            "tokens": total,
            "input_tokens": input_tokens,
            "cached_input_tokens": int(last.get("cached_input_tokens") or 0),
            "output_tokens": output_tokens,
            "accounting_source": "structured_cli_usage",
        }
    for pattern in (
        r"tokens\s+used\s*[:\n ]\s*([0-9][0-9,]*)",
        r"total[_ ]tokens\s*[=:]\s*([0-9][0-9,]*)",
        r"total\s+tokens\s*[:=]\s*([0-9][0-9,]*)",
    ):
        matches = re.findall(pattern, text_tail, flags=re.I)
        if matches:
            return {
                "tokens": int(matches[-1].replace(",", "")),
                "accounting_source": "text_total_fallback",
            }
    return None


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--usage-file", required=True)
    parser.add_argument("--log-file", required=True)
    parser.add_argument("--raw-log")
    parser.add_argument("--prompt-file")
    parser.add_argument("--role", required=True)
    parser.add_argument("--model", required=True)
    parser.add_argument("--round", default="0")
    parser.add_argument("--task", default="")
    args = parser.parse_args()

    candidates = [Path(args.raw_log)] if args.raw_log else []
    candidates.append(Path(args.log_file))
    source = next((path for path in candidates if path.is_file() and path.stat().st_size), None)
    if source is None:
        return 0
    usage = parse_usage(source)
    if usage is None:
        return 0
    prompt_tokens = None
    if args.prompt_file and Path(args.prompt_file).is_file():
        prompt_tokens = max(1, (Path(args.prompt_file).stat().st_size + 3) // 4)
    row = {
        "timestamp": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "role": args.role,
        "model": args.model,
        "round": int(args.round or 0),
        "task": args.task,
        "prompt_file_estimate_tokens": prompt_tokens,
        "log_file": args.log_file,
        "raw_log_file": str(source),
        **usage,
    }
    out = Path(args.usage_file)
    out.parent.mkdir(parents=True, exist_ok=True)
    with out.open("a", encoding="utf-8") as stream:
        stream.write(json.dumps(row, ensure_ascii=False) + "\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
