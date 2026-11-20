"""Hand-checked golden set: a small page to mark model labels and loker's
output right or wrong, blind. Run by hand; not part of CI.

  python3 review.py build          # pick jobs from the clean holdouts -> items.json (not committed)
  python3 review.py serve [port]   # serve the page; verdicts save to verdicts.json
  python3 review.py score          # how often each source was right, by your verdicts

Each job shows its distinct answers as A, B, C in a shuffled order, with no
source named: Opus's label, DeepSeek's label and loker's output (for roles,
rules alone and rules + Jev). Identical answers are one option. Which source
gave which answer stays in items.json, so the review can't be steered.
"""
import html, json, os, random, re, subprocess, sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

HERE = os.path.dirname(os.path.abspath(__file__))
EVAL = os.path.dirname(HERE)
sys.path.insert(0, os.path.join(EVAL, "pay"))
sys.path.insert(0, os.path.join(EVAL, "role"))
N_PAY, N_ROLE = 12, 12


def load(path, default):
    return json.load(open(path)) if os.path.exists(path) else default


def pay_text(p):
    if not p or not p.get("base") and not p.get("extras"):
        return "No pay stated"
    def money(m):
        if not isinstance(m, dict):  # a labeler wrote a bare figure
            return str(m)
        lo, hi = m.get("min"), m.get("max")
        f = lambda v: f"{v:,.2f}".rstrip("0").rstrip(".")
        amt = f(lo) if lo == hi else f"up to {f(hi)}" if lo is None else f"from {f(lo)}" if hi is None else f"{f(lo)}–{f(hi)}"
        per = f" a {m['period']}" if m.get("period") else ""
        label = f"{m['label']}: " if m.get("label") else ""
        return f"{label}{m['currency']} {amt}{per}"
    parts = [money(m) for m in p.get("base", [])] or ["no base pay"]
    extras = []
    for e in p.get("extras", []):
        x = e["kind"]
        if e.get("amount"):
            x += " " + money(e["amount"])
        if isinstance(e.get("percent"), (int, float)):
            x += f" {e['percent']}%"
        elif e.get("percent"):
            pc = e["percent"]
            x += f" {pc.get('min')}–{pc.get('max')}%" if pc.get("min") != pc.get("max") else f" {pc.get('min')}%"
        extras.append(x)
    return "; ".join(parts) + (" + " + ", ".join(sorted(extras)) if extras else "")


def role_text(r):
    if not r or r.get("primary") in (None, "none"):
        return "none (no role fits)"
    t = r["primary"]
    if r.get("secondary"):
        t += " (also " + ", ".join(r["secondary"]) + ")"
    if r.get("modifiers"):
        t += " · " + ", ".join(r["modifiers"])
    return t


def options(answers, rng):
    """{source: text} -> shuffled distinct options, each with the sources behind it."""
    by_text = {}
    for src, text in answers.items():
        by_text.setdefault(text, []).append(src)
    opts = [{"text": t, "sources": s} for t, s in by_text.items()]
    rng.shuffle(opts)
    for i, o in enumerate(opts):
        o["key"] = "ABCDE"[i]
    return opts


def build():
    rng = random.Random("golden-v1")
    items = []
    # pay: from holdout3, the clean pay holdout
    import pay_eval as pe
    h = load(os.path.join(EVAL, "pay", "holdout3.json"), {})
    ids = rng.sample(h["mention"], 9) + rng.sample(h["rest"], 3)
    texts = pe.descriptions(set(ids))
    ours = pe.predicted(pe.BIN, ids)
    lo, ld = load(os.path.join(EVAL, "pay", "labels-opus.json"), {}), load(os.path.join(EVAL, "pay", "labels-deepseek.json"), {})
    for j in ids:
        answers = {"opus": pay_text(lo.get(f"v1|{j}")), "deepseek": pay_text(ld.get(f"v1|{j}")), "loker": pay_text(ours.get(j))}
        items.append({"id": j, "kind": "pay", "title": texts[j]["title"], "company": texts[j]["company"],
                      "description": texts[j]["description"], "options": options(answers, rng)})
    # roles: from fresh.json, jobs fetched after the rules were frozen
    os.environ["LOKER_EVAL_DB"] = ""
    sys.argv = ["role_eval.py", "--set", "fresh"]
    import role_eval as re_
    fresh = re_.set_ids()
    ids = rng.sample(fresh, N_ROLE)
    _, version = re_.vocab_tree(re_.BIN)
    key = lambda jid: f"{re_.PROMPT_VERSION}|{version}|{jid}"
    lo, ld = re_.load("labels-opus.json", {}), re_.load("labels-deepseek.json", {})
    re_.DB = os.path.join(EVAL, "role", "snapshot-fresh.db")
    rules = re_.classified(re_.BIN, ids)
    re_.DB = os.path.join(EVAL, "role", "snapshot-fresh-jev.db")
    shipped = re_.classified(re_.BIN, ids)
    texts = re_.jobs_text(ids)
    for j in ids:
        answers = {"opus": role_text(lo.get(key(j))), "deepseek": role_text(ld.get(key(j))),
                   "loker_rules": role_text(rules[j].get("role")), "loker_rules_jev": role_text(shipped[j].get("role"))}
        items.append({"id": j, "kind": "role", "title": texts[j]["title"], "company": texts[j]["company"],
                      "category": texts[j]["category"], "description": texts[j]["description"],
                      "options": options(answers, rng)})
    vocab = [r["id"] for r in re_.with_home(re_.BIN, lambda home: re_.loker(re_.BIN, home, "roles"))["roles"]]
    json.dump({"version": 1, "vocabulary": vocab, "items": items}, open(os.path.join(HERE, "items.json"), "w"),
              indent=1, ensure_ascii=False)
    print(f"{len(items)} items -> items.json")


PAGE = os.path.join(HERE, "index.html")


class Handler(BaseHTTPRequestHandler):
    def send(self, code, body, ctype):
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path in ("/", "/index.html"):
            return self.send(200, open(PAGE, "rb").read(), "text/html; charset=utf-8")
        if self.path == "/data":
            data = load(os.path.join(HERE, "items.json"), {})
            # the page never learns which source gave which answer
            for it in data.get("items", []):
                for o in it["options"]:
                    o.pop("sources", None)
            data["verdicts"] = load(os.path.join(HERE, "verdicts.json"), {})
            return self.send(200, json.dumps(data).encode(), "application/json")
        self.send(404, b"not found", "text/plain")

    def do_POST(self):
        if self.path != "/save":
            return self.send(404, b"not found", "text/plain")
        body = self.rfile.read(int(self.headers.get("Content-Length", 0)))
        verdicts = json.loads(body)
        tmp = os.path.join(HERE, "verdicts.json.tmp")
        json.dump(verdicts, open(tmp, "w"), indent=1, sort_keys=True, ensure_ascii=False)
        os.replace(tmp, os.path.join(HERE, "verdicts.json"))
        self.send(200, b"ok", "text/plain")

    def log_message(self, *a):
        pass


def serve():
    port = int(sys.argv[2]) if len(sys.argv) > 2 else 8787
    print(f"serving on http://127.0.0.1:{port}", flush=True)
    ThreadingHTTPServer(("127.0.0.1", port), Handler).serve_forever()


def score():
    data = load(os.path.join(HERE, "items.json"), {})
    verdicts = load(os.path.join(HERE, "verdicts.json"), {})
    from collections import Counter
    right, seen, open_ = Counter(), Counter(), Counter()
    for it in data["items"]:
        v = verdicts.get(it["id"], {})
        for o in it["options"]:
            mark = v.get("marks", {}).get(o["key"])
            for s in o["sources"]:
                seen[(it["kind"], s)] += 1
                right[(it["kind"], s)] += mark == "right"
                # an answer left unmarked is not counted right or wrong, and is shown
                open_[(it["kind"], s)] += mark not in ("right", "wrong")
    for kind in ("pay", "role"):
        done = sum(1 for it in data["items"] if it["kind"] == kind and verdicts.get(it["id"], {}).get("done"))
        print(f"{kind}: {done} fully marked")
        for (k, s), n in sorted(seen.items()):
            if k == kind:
                print(f"  {s:16} right {right[(k, s)]}/{n}" + (f"  ({open_[(k, s)]} unmarked)" if open_[(k, s)] else ""))


if __name__ == "__main__":
    cmds = {"build": build, "serve": serve, "score": score}
    if len(sys.argv) < 2 or sys.argv[1] not in cmds:
        sys.exit(__doc__)
    cmds[sys.argv[1]]()
