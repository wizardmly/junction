#!/usr/bin/env python3
"""List the task board in docs/tasks: each task's status, owner and branch,
plus who holds a claim lock on the remote (a `task/<id>` branch) and,
with --prs, open pull requests whose title starts with `[<id>]`.

    python3 tools/tasks.py            # all tasks
    python3 tools/tasks.py --open     # only tasks nobody holds
    python3 tools/tasks.py --prs      # also ask GitHub for open PRs
"""

import json
import os
import re
import subprocess
import sys
import urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TASKS = os.path.join(ROOT, "docs", "tasks")
REPO = "wizardmly/junction"


def front_matter(path):
    with open(path, encoding="utf-8") as f:
        text = f.read()
    m = re.match(r"---\n(.*?)\n---", text, re.S)
    fields = {}
    for line in (m.group(1) if m else "").splitlines():
        key, _, value = line.partition(":")
        fields[key.strip()] = value.split("#")[0].strip()
    return fields


def remote_branches():
    """The `task/*` branches on origin, or None when origin can't be reached."""
    try:
        out = subprocess.run(
            ["git", "ls-remote", "--heads", "origin", "task/*"],
            cwd=ROOT, capture_output=True, text=True, timeout=30,
        )
    except (OSError, subprocess.TimeoutExpired):
        return None
    if out.returncode != 0:
        return None
    return {line.split("refs/heads/", 1)[1] for line in out.stdout.splitlines() if "refs/heads/" in line}


def open_prs():
    """Open PRs as (number, title, head branch), or None when GitHub can't be reached."""
    prs = []
    try:
        for page in range(1, 6):
            url = f"https://api.github.com/repos/{REPO}/pulls?state=open&per_page=100&page={page}"
            req = urllib.request.Request(url, headers={"Accept": "application/vnd.github+json", "User-Agent": "junction-tasks"})
            with urllib.request.urlopen(req, timeout=30) as r:
                batch = json.load(r)
            prs += [(p["number"], p["title"], p["head"]["ref"]) for p in batch]
            if len(batch) < 100:
                break
    except Exception:
        return None
    return prs


def main():
    args = sys.argv[1:]
    branches = remote_branches()
    prs = open_prs() if "--prs" in args else None
    rows = []
    for name in sorted(os.listdir(TASKS)):
        if not name.endswith(".md") or name == "README.md":
            continue
        t = front_matter(os.path.join(TASKS, name))
        tid = t.get("id") or name[:-3]
        held = []
        if branches is not None and f"task/{tid}" in branches:
            held.append(f"branch task/{tid}")
        for number, title, head in prs or []:
            if title.startswith(f"[{tid}]") or head == f"task/{tid}":
                held.append(f"PR #{number}")
        status = t.get("status", "open")
        if status == "open" and held:
            status = "claimed?"  # locked on the remote, not merged to master yet
        if "--open" in args and (status != "open" or held):
            continue
        rows.append((tid, status, t.get("owner", ""), ", ".join(held), t.get("title", "")))
    if branches is None:
        print("(could not reach origin: remote claim locks not checked)", file=sys.stderr)
    if "--prs" in args and prs is None:
        print("(could not reach GitHub: open PRs not checked)", file=sys.stderr)
    width = max((len(r[0]) for r in rows), default=2)
    for tid, status, owner, held, title in rows:
        line = f"{tid:<{width}}  {status:<9} {title}"
        if owner:
            line += f"  — {owner}"
        if held:
            line += f"  [{held}]"
        print(line)


if __name__ == "__main__":
    main()
