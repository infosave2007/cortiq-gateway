#!/usr/bin/env python3
"""Task-type classification accuracy: the allaigate router vs a naive keyword
heuristic (what you get without a semantic classifier, e.g. with LiteLLM/Portkey).

    CORTIQ_ROUTER_KEY=cortiq_... python3 bench/accuracy.py

Env: CORTIQ_ROUTER_KEY (required), ROUTER_URL (default https://138.226.222.209),
     TAXONOMY (default data-assistant).
"""
import json, os, ssl, sys, urllib.request

ROUTER_URL = os.environ.get("ROUTER_URL", "https://138.226.222.209").rstrip("/")
TAXONOMY = os.environ.get("TAXONOMY", "data-assistant")
KEY = os.environ.get("CORTIQ_ROUTER_KEY", "")
CTX = ssl._create_unverified_context()

HERE = os.path.dirname(os.path.abspath(__file__))
DATA = os.path.join(HERE, "tasks.jsonl")


def router_predict(text):
    body = json.dumps({"input": {"text": text}, "taxonomy_id": TAXONOMY}).encode()
    req = urllib.request.Request(
        ROUTER_URL + "/v1/route",
        data=body,
        headers={"content-type": "application/json", "Authorization": "Bearer " + KEY},
    )
    with urllib.request.urlopen(req, context=CTX, timeout=30) as r:
        d = json.load(r)
    return d.get("decision", {}).get("task_label", "?")


def keyword_predict(text):
    """Naive DIY routing without a semantic classifier."""
    t = text.lower()
    def has(*words):
        return any(w in t for w in words)
    if has("translate", "say ", "in spanish", "in japanese", "in german", "in italian",
           "to english", "to german", "from french", "render the phrase"):
        return "translation"
    if has("summarize", "summary", "tl;dr", "condense", "shorten", "bullet point"):
        return "summarization"
    if has("extract", "pull out", "list all", "find every", "find all"):
        return "extraction"
    if has("function", "python", "javascript", "rust", "sql", "query", "implement",
           "bug", "linked list", "code"):
        return "code"
    if has("solve", "derivative", "integral", "compute", "sum of", "area of",
           "limit of", "evaluate", "equation"):
        return "math"
    if t.strip().endswith("?") or t.split()[:1] and t.split()[0] in (
        "who", "what", "when", "where", "which"):
        return "qa"
    return "chitchat"


def main():
    if not KEY:
        print("set CORTIQ_ROUTER_KEY", file=sys.stderr)
        sys.exit(2)
    rows = [json.loads(l) for l in open(DATA) if l.strip()]
    r_ok = k_ok = 0
    misses = []
    for row in rows:
        gt = row["label"]
        try:
            rp = router_predict(row["text"])
        except Exception as e:
            rp = "ERR:" + str(e)[:30]
        kp = keyword_predict(row["text"])
        r_ok += rp == gt
        k_ok += kp == gt
        if rp != gt:
            misses.append((gt, rp, row["text"][:45]))
    n = len(rows)
    print(f"dataset: {n} prompts, {len(set(r['label'] for r in rows))} task types")
    print(f"allaigate router : {r_ok}/{n} = {100*r_ok/n:.1f}%")
    print(f"keyword heuristic: {k_ok}/{n} = {100*k_ok/n:.1f}%")
    if misses:
        print("\nrouter misses (truth -> predicted | prompt):")
        for gt, rp, txt in misses:
            print(f"  {gt:>13} -> {rp:<13} | {txt}")


if __name__ == "__main__":
    main()
