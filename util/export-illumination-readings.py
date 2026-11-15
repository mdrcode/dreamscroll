#!/usr/bin/env python3
"""Export illumination readings as JSONL, one line per reading.

Every arm is the same shape, keyed by what produced it, so arms can be compared
by joining on `capture_id`:

    {"capture_id": 172, "arm": "ungrounded-high", "web_search": false,
     "thinking_level": "high", "prompt_version": "v2", "model_id": "gemini-3.8-flash",
     "task_run_id": "...", "raw_id": 167, "created_at": "...", "duration_ms": 11234,
     "tokens": {"in": 3150, "in_cached": null, "out": 500, "thought": 2200},
     "summary": "...", "details": "...", "suggested_searches": [...],
     "entities": [{"name": "...", "type": "...", "description": "...",
                   "search_query": null, "platform_link": null}]}

Usage:
    ./util/export-illumination-readings.py > plan/eval/illumination-readings.jsonl
    ./util/export-illumination-readings.py --arm ungrounded-high

Reads the local dev Postgres through the `dreamscroll-db-1` container, mirroring
the other helpers in util/. It is read-only.
"""

import argparse
import json
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
CONTAINER = "dreamscroll-db-1"


def psql(sql):
    env = dict(
        line.split("=", 1)
        for line in (ROOT / ".env").read_text().splitlines()
        if "=" in line and not line.strip().startswith("#")
    )
    result = subprocess.run(
        [
            "docker", "exec", "-e", f"PGPASSWORD={env['POSTGRES_PASSWORD']}",
            CONTAINER, "psql", "-U", "dreamscroll_pg_user",
            "-d", env.get("POSTGRES_DB", "dreamscroll_dev1"), "-At", "-c", sql,
        ],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        sys.exit(f"psql failed: {result.stderr.strip()}")
    return result.stdout


def arm_name(web_search, thinking_level):
    grounding = "grounded" if web_search else "ungrounded"
    return f"{grounding}-{thinking_level or 'default'}"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--arm", help="only emit this arm, e.g. ungrounded-high")
    args = parser.parse_args()

    sql = """
    select json_agg(x) from (
      select r.id as raw_id, r.capture_id, r.task_run_id, r.created_at,
             r.content as content,
             im.model, im.provider, im.backend, im.duration_ms,
             im.tokens_in, im.tokens_in_cached, im.tokens_out, im.tokens_thought,
             im.params as params,
             t.task_payload->>'thinking_level' as thinking_level,
             coalesce(t.task_payload->>'web_search', 'true') as web_search,
             coalesce(t.task_payload->>'persist_projection', 'true') as persist_projection,
             coalesce(im.params->>'prompt_version', t.task_payload->>'prompt_version') as prompt_version
      from illumination_raws r
      join task_runs t on t.run_id = r.task_run_id
      left join inference_metas im on im.id = r.inference_meta_id
    ) x;
    """
    rows = json.loads(psql(sql) or "[]")
    emitted = 0
    for row in sorted(rows, key=lambda r: (r["capture_id"], r["created_at"])):
        web_search = row["web_search"] == "true"
        arm = arm_name(web_search, row["thinking_level"])
        if args.arm and args.arm != arm:
            continue
        content = row["content"] or {}
        record = {
            "capture_id": row["capture_id"],
            "arm": arm,
            "web_search": web_search,
            "thinking_level": row["thinking_level"],
            "prompt_version": row["prompt_version"],
            "model_id": row["model"],
            "provider": row["provider"],
            "backend": row["backend"],
            "persist_projection": row["persist_projection"] == "true",
            "task_run_id": row["task_run_id"],
            "raw_id": row["raw_id"],
            "created_at": row["created_at"],
            "duration_ms": row["duration_ms"],
            "tokens": {
                "in": row["tokens_in"],
                "in_cached": row["tokens_in_cached"],
                "out": row["tokens_out"],
                "thought": row["tokens_thought"],
            },
            "summary": content.get("summary"),
            "details": content.get("details"),
            "suggested_searches": content.get("suggested_searches") or [],
            "entities": [
                {
                    "name": entity.get("name"),
                    "type": entity.get("type"),
                    "description": entity.get("description"),
                    "search_query": entity.get("search_query"),
                    "platform_link": entity.get("platform_link"),
                }
                for entity in content.get("entities") or []
            ],
        }
        print(json.dumps(record, ensure_ascii=False))
        emitted += 1
    print(f"emitted {emitted} readings", file=sys.stderr)


if __name__ == "__main__":
    main()
