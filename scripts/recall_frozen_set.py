#!/usr/bin/env python3
"""Run the frozen recall question set against a hivemind build and print counts per kind.

The set is `tests/fixtures/recall_frozen/questions.tsv`: one row per question with the decision it
should find (or `none`) and the kind of wrong answer the row watches for. It is kept the same from
build to build, so two runs of two builds on the same ledger copy say whether a build got better or
worse. Rules for the set and the two kinds are in the header of that file.

    scripts/recall_frozen_set.py --bin target/debug/hivemind --ledger-dir /path/with/ledger.sqlite
    scripts/recall_frozen_set.py ... --out new.json --compare old.json    # rows that gained or lost

`--ledger-dir` is only read: its ledger.sqlite is copied (sqlite backup) into a temporary folder and
that copy is served on a loopback port, so the one-time replay of a large ledger is paid once.
Standard library only.
"""
import argparse
import json
import os
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
from concurrent.futures import ThreadPoolExecutor

KINDS = ("NEARBY", "SYNONYM")
DEFAULT_QUESTIONS = os.path.join(
    os.path.dirname(os.path.abspath(__file__)), "..", "tests", "fixtures", "recall_frozen", "questions.tsv"
)
TOP = 3


def load_questions(path):
    """Rows of the fixture: (id, kind, expect set, question). `none` expects no decision."""
    rows = []
    for number, line in enumerate(open(path, encoding="utf-8"), 1):
        if not line.strip() or line.startswith("#"):
            continue
        cols = line.rstrip("\n").split("\t")
        if len(cols) < 4:
            sys.exit(f"{path}:{number}: expected id, kind, expect, question")
        row_id, kind, expect, question = cols[:4]
        if kind not in KINDS:
            sys.exit(f"{path}:{number}: kind {kind!r} is not one of {KINDS}")
        targets = set() if expect == "none" else set(expect.split("|"))
        rows.append((row_id, kind, targets, question))
    return rows


def short(decision_id):
    """`decision-0a1b2c3d-...` -> `0a1b2c3d`; a draft id (`capture:7:0`) stays whole."""
    return decision_id[len("decision-"):][:8] if decision_id.startswith("decision-") else decision_id


def get(port, path, params, attempts=40):
    """A GET that waits for the server (a large ledger replays before it listens)."""
    last = None
    for _ in range(attempts):
        try:
            url = f"http://127.0.0.1:{port}{path}?" + urllib.parse.urlencode(params)
            with urllib.request.urlopen(url, timeout=1800) as response:
                return json.loads(response.read())
        except (urllib.error.URLError, ConnectionError, TimeoutError, OSError) as error:
            last = error
            time.sleep(15)
    raise SystemExit(f"server on port {port} never answered: {last}")


def ask(port, question):
    why = get(port, "/v1/decisions/why", {"description": question})
    data = why.get("data", {})
    outcome = data.get("outcome")
    if outcome == "ambiguous":
        answer = {"outcome": "listed", "ids": [short(c["decision_id"]) for c in data["candidates"]]}
    elif outcome == "not_found":
        answer = {"outcome": "none", "ids": []}
    else:
        root = (data.get("root") or {}).get("id")
        answer = {"outcome": "answered", "ids": [short(root)] if root else []}
    recall = get(port, "/v1/decisions/recall", {"q": question, "limit": 20})
    ids = [short(item["decision"]["id"]) for item in recall["data"]["ranked"]["items"]]
    return {"why": answer, "recall": ids}


def judge(kind, targets, result):
    """What went wrong on this row, as a list of flags; an empty list is a pass."""
    why, recall = result["why"], result["recall"]
    flags = []
    if why["outcome"] == "answered" and not (why["ids"] and why["ids"][0] in targets):
        flags.append("wrong_answer")
    if targets and not targets & set(recall[:TOP]):
        flags.append("not_top3" if targets & set(recall) else "not_found")
    if kind == "SYNONYM":
        return [f for f in flags if f != "wrong_answer"]
    return flags


def answer_class(targets, why):
    """right / first / listed / wrong / miss for a question with targets; answered / listed / none for `none`."""
    outcome, ids = why["outcome"], why["ids"]
    if not targets:
        return outcome
    if outcome == "answered":
        return "right" if ids and ids[0] in targets else "wrong"
    if outcome == "listed":
        if ids and ids[0] in targets:
            return "first"
        return "listed" if targets & set(ids) else "miss"
    return "miss"


def serve(binary, ledger_dir, port, workdir):
    os.makedirs(workdir, exist_ok=True)
    source = sqlite3.connect(f"file:{os.path.join(ledger_dir, 'ledger.sqlite')}?mode=ro", uri=True)
    target = sqlite3.connect(os.path.join(workdir, "ledger.sqlite"))
    source.backup(target)
    events = target.execute("select count(*) from events").fetchone()[0]
    target.close()
    source.close()
    env = {k: v for k, v in os.environ.items() if not k.startswith("HIVEMIND_")}
    log = open(os.path.join(workdir, "serve.log"), "w")
    process = subprocess.Popen(
        [binary, "--hivemind-dir", workdir, "serve", "--port", str(port)],
        env=env, stdout=log, stderr=log,
    )
    return process, events


def version(binary):
    return subprocess.run([binary, "--version"], capture_output=True, text=True).stdout.strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--bin", required=True, help="the hivemind binary to measure")
    parser.add_argument("--ledger-dir", required=True, help="folder holding ledger.sqlite (only read)")
    parser.add_argument("--questions", default=DEFAULT_QUESTIONS, help="the question set (default: the frozen one)")
    parser.add_argument("--port", type=int, default=19311, help="loopback port for the temporary server")
    parser.add_argument("--out", help="write every row's answer as JSON")
    parser.add_argument("--compare", help="a JSON from an earlier run: list the rows that gained or lost")
    parser.add_argument("--workers", type=int, default=4)
    args = parser.parse_args()

    rows = load_questions(args.questions)
    workdir = tempfile.mkdtemp(prefix="recall-frozen-")
    process, events = serve(args.bin, args.ledger_dir, args.port, workdir)
    try:
        get(args.port, "/v1/decisions/recall", {"q": "warm up the server", "limit": 1})
        graph = get(args.port, "/v1/graph", {})
        held = {short(d["id"]) for d in graph.get("data", graph)["decisions"]}
        with ThreadPoolExecutor(args.workers) as pool:
            answers = list(pool.map(lambda row: ask(args.port, row[3]), rows))
    finally:
        process.terminate()
        process.wait()
        shutil.rmtree(workdir, ignore_errors=True)

    results = []
    for (row_id, kind, targets, question), answer in zip(rows, answers):
        # A row whose decision this ledger copy does not hold cannot be answered here: it is left out.
        applies = not targets or any(t in held or t.startswith("capture:") for t in targets)
        results.append({
            "id": row_id, "kind": kind, "expect": sorted(targets), "question": question, "applies": applies,
            "flags": judge(kind, targets, answer) if applies else [],
            "class": answer_class(targets, answer["why"]) if applies else "n/a", **answer,
        })
    report(results, version(args.bin), events, args.ledger_dir)
    if args.out:
        json.dump({"build": version(args.bin), "events": events, "rows": results}, open(args.out, "w"), indent=1)
    if args.compare:
        compare(results, json.load(open(args.compare))["rows"])


def report(results, build, events, ledger_dir):
    skipped = sum(not r["applies"] for r in results)
    results = [r for r in results if r["applies"]]
    print(f"build {build} | ledger {ledger_dir} ({events} events) | {len(results)} questions"
          + (f", {skipped} left out (their decision is not in this ledger)" if skipped else ""))
    print(f"{'kind':<8} {'rows':>4} {'fail':>4}  wrong one-call answers / expected missing from top {TOP} / expected not found at all")
    for kind in KINDS:
        of_kind = [r for r in results if r["kind"] == kind]
        count = lambda flag: sum(flag in r["flags"] for r in of_kind)  # noqa: E731
        failed = sum(bool(r["flags"]) for r in of_kind)
        print(f"{kind:<8} {len(of_kind):>4} {failed:>4}  {count('wrong_answer')} / {count('not_top3') + count('not_found')} / {count('not_found')}")
    with_target = [r for r in results if r["expect"]]
    top3 = sum(bool(set(r["expect"]) & set(r["recall"][:TOP])) for r in with_target)
    classes = {}
    for r in with_target:
        classes[r["class"]] = classes.get(r["class"], 0) + 1
    controls = {}
    for r in results:
        if not r["expect"]:
            controls[r["class"]] = controls.get(r["class"], 0) + 1
    print(f"all rows: recall top {TOP} {top3} of {len(with_target)}; why " + ", ".join(
        f"{name} {classes.get(name, 0)}" for name in ("right", "first", "listed", "wrong", "miss")))
    print("controls (expect none): " + ", ".join(f"{name} {controls.get(name, 0)}" for name in ("answered", "listed", "none")))
    for r in results:
        if r["flags"]:
            print(f"  FAIL {r['kind']:<8} {r['id']:<14} {','.join(r['flags']):<22} expect={'|'.join(r['expect']) or 'none':<10} "
                  f"why={r['class']:<8} {r['question'][:72]}")


def compare(results, before):
    old = {r["id"]: r for r in before}
    gained = [r for r in results if not r["flags"] and old.get(r["id"], {}).get("flags")]
    lost = [r for r in results if r["flags"] and old.get(r["id"]) is not None and not old[r["id"]]["flags"]]
    changed = [r for r in results if r["id"] in old and r["flags"] and old[r["id"]]["flags"] != r["flags"]]
    print(f"vs earlier run: {len(gained)} rows pass now, {len(lost)} rows pass no longer, {len(changed)} fail differently")
    for label, rows in (("GAIN", gained), ("LOSS", lost), ("CHANGED", changed)):
        for r in rows:
            was = ",".join(old[r["id"]]["flags"]) or "pass"
            print(f"  {label:<7} {r['kind']:<8} {r['id']:<14} {was} -> {','.join(r['flags']) or 'pass'}  {r['question'][:64]}")


if __name__ == "__main__":
    main()
