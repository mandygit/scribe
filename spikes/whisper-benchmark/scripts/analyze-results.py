#!/usr/bin/env python3
"""Summarize whisper.cpp benchmark logs and optional transcript WER."""

from __future__ import annotations

import argparse
import csv
import re
from pathlib import Path
from typing import Optional


def parse_time_log(path: Path) -> tuple[Optional[float], Optional[int]]:
    if not path.exists():
        return None, None

    real_seconds: Optional[float] = None
    max_rss_bytes: Optional[int] = None
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        real_match = re.match(r"\s*([0-9.]+)\s+real\b", line)
        if real_match:
            real_seconds = float(real_match.group(1))
            continue

        rss_match = re.match(r"\s*(\d+)\s+maximum resident set size", line)
        if rss_match:
            max_rss_bytes = int(rss_match.group(1))

    return real_seconds, max_rss_bytes


def read_text_if_present(path_text: str) -> str:
    path = Path(path_text)
    if not path_text or not path.exists():
        return ""
    return path.read_text(encoding="utf-8", errors="replace").strip()


def compute_wer(reference: str, hypothesis: str) -> Optional[float]:
    if not reference or not hypothesis:
        return None
    try:
        from jiwer import wer
    except ImportError as exc:
        raise SystemExit("jiwer is required for WER; use .venv-benchmark/bin/python") from exc
    return float(wer(reference, hypothesis))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--reference", type=Path)
    args = parser.parse_args()

    reference_text = ""
    if args.reference:
        reference_text = args.reference.read_text(encoding="utf-8", errors="replace").strip()

    rows: list[dict[str, str]] = []
    with args.manifest.open(newline="", encoding="utf-8") as manifest_file:
        for row in csv.DictReader(manifest_file):
            real_seconds, max_rss_bytes = parse_time_log(Path(row["log_path"]))
            hypothesis = read_text_if_present(row.get("transcript_path", ""))
            wer_value = compute_wer(reference_text, hypothesis)
            rows.append(
                {
                    "model": row["model"],
                    "command": row["command"],
                    "status": row["status"],
                    "real_seconds": "" if real_seconds is None else f"{real_seconds:.3f}",
                    "max_rss_bytes": "" if max_rss_bytes is None else str(max_rss_bytes),
                    "wer": "" if wer_value is None else f"{wer_value:.6f}",
                    "transcript_path": row.get("transcript_path", ""),
                    "log_path": row["log_path"],
                }
            )

    args.output.parent.mkdir(parents=True, exist_ok=True)
    fieldnames = [
        "model",
        "command",
        "status",
        "real_seconds",
        "max_rss_bytes",
        "wer",
        "transcript_path",
        "log_path",
    ]
    with args.output.open("w", newline="", encoding="utf-8") as output_file:
        writer = csv.DictWriter(output_file, fieldnames=fieldnames)
        writer.writeheader()
        writer.writerows(rows)

    print(args.output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
