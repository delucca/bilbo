"""Build each answerable question's keep-list: the terms of its original prompt that a query variation must keep.
Usage: keep.py <corpus-dir> <questions.jsonl> <out.jsonl>
Built from names alone, before any model call; it never reads a fact. Each entry is a span of the prompt, so entries
never overlap, with a kind that says how a variation must hold it: `identifier` exactly, `acronym` in the same capitals
(a plural is fine), `words` in the same words in any case, separator or inflection. Any entry may have `equivalents`.
`framing_only` says that every topic word of the prompt is in a kept span. The constants below set what counts as a
project term; the developer's messages are the user entries that are not tool results, meta, sidechains, compaction
summaries, command or hook wrappers. It prints each question's list for review."""

import json
import re
import sys
from collections import Counter
from pathlib import Path

import Stemmer

MIN_TYPED = 2
DEV_SPREAD = 6
SPECIFIC = 20
MIN_RAW = 10
RAW_SPREAD = 3
COHESION = 0.1
CMD_MIN = 10
CMD_SPREAD = 6
TERMS_OF_ART = (
    "pr", "pull request", "commit", "merge", "branch", "worktree", "fork", "remote", "lint", "refactor", "migration",
    "enum", "deploy", "redeploy", "hotfix", "smoke", "dry run", "ci", "sdk", "adr", "cache invalidation", "crash dump",
    "regression", "spike", "subagent", "sandbox", "review", "fixture", "consumer", "sync", "stash", "e2e",
    "end-to-end", "rebase", "release", "changelog", "postmortem", "pagination", "endpoint", "schema", "webhook",
    "diff", "backup", "image",
)
EQUIVALENTS = (
    ("pr", "pull request"), ("e2e", "end-to-end"), ("ci", "continuous integration"), ("sdk", "software development kit"),
    ("adr", "architecture decision record"), ("prod", "production"), ("repo", "repository"), ("config", "configuration"),
    ("docs", "documentation"), ("env", "environment"), ("auth", "authentication"), ("deps", "dependencies"),
)
STOP = set("""a about above after again against all also am an and any are as at be because been before being below
between both but by can could did do does doing done down during each else ever few for from further get got had has
have having he her here hers him his i if in into is it its itself just let me more most my no nor not now of off on
once only or other our ours out over own same she should so some such than that the their them then there these they
this those through to too under until up us very was we were what when where which while who whom why will with would
you your yours anything something things thing one ones way please lets many much isn didn doesn don wasn aren ca wo
haven hasn wouldn couldn shouldn weren
""".split())
FRAMING = set("""know known learned learnt decide decided decision decisions settle settled chose choose chosen choice
reason reasons reasoning why how steps step status state left leave remaining remains still stopped stop pick picked
resume continue where when date day happened happen session sessions earlier last previous before ago recall tell told
say said find found look see show give list need needs needed want wanted use used using do does did make made work
worked working go going back again done next open pending exactly ever whole new old current story deal
rundown stuff gotcha gotchas pitfall pitfalls lesson lessons recipe process procedure rule rules overview explain remind
fill walk watch usual actual actually supposed figure
""".split())
SKIP = ("<command-", "<local-command", "<system-reminder", "Caveat:", "[Request interrupted", "<task-notification",
        "This session is being continued", "<bash-", "<user-prompt-submit-hook")
IDENTIFIER = re.compile(r"https?://\S+|`[^`]+`|\b[a-z0-9]+(?:-[a-z0-9]+){2,}\b|(?<![\w-])--?[a-z][\w-]*|"
                        r"[\w.-]*[/\\][\w./\\-]*|#\d+|\b[\w-]*\d[\w.-]*\b|\b\w+(?:[._]\w+)+\b|\b[a-z]+[A-Z]\w*\b")
ACRONYM = re.compile(r"\b[A-Z]{2,5}s?\b")
WORD = re.compile(r"[A-Za-z0-9]+(?:-[A-Za-z0-9]+)*(?:'[A-Za-z]+)?")
QUOTED = re.compile(r"'[^']*'|\"(?:\\.|[^\"\\])*\"")
SHELL_SEP = re.compile(r"&&|\|\||[|;\n()]")
WRAPPERS = {"sudo", "env", "time", "exec", "nohup", "npx", "bunx", "uv", "pnpm", "yarn", "npm", "bun", "run", "nix",
            "develop", "shell", "--command", "-c", "xargs", "timeout", "watch"}
OPERAND_PROGRAMS = {"cd", "ls", "cat", "rm", "mkdir", "rmdir", "cp", "mv", "touch", "head", "tail", "less", "more", "open",
                    "find", "grep", "rg", "echo", "printf", "wc", "sed", "awk", "sort", "uniq", "cut", "tr", "tee", "diff",
                    "chmod", "chown", "ln", "stat", "file", "du", "df", "tree", "source", "python", "python3", "node",
                    "bash", "sh", "zsh", "ruby", "perl", "jq", "curl", "wget", "sleep", "kill", "tar", "unzip", "vim",
                    "nano", "code", "which", "test", "export", "readlink", "realpath", "dirname", "basename", "xargs"}
SUBCOMMAND_WRAPPERS = {"nix", "npm", "pnpm", "yarn", "bun", "uv"}
QUOTED_WORD = re.compile(r"""(['"])([\w.:/-]+)\1""")
WRAPPER_PROGRAMS = WRAPPERS - {"env", "time", "exec", "timeout", "watch", "run", "develop", "shell", "--command", "-c"}
COMPOSE_GLOBAL_VALUE_FLAGS = {"-f", "--file", "-p", "--project-name", "--profile", "--env-file", "--project-directory",
                              "--ansi", "--parallel", "--progress"}
COMPOSE_VALUE_FLAGS = {
    "run": {"-e", "--env", "-u", "--user", "-w", "--workdir", "-v", "--volume", "--name", "--entrypoint", "-l", "--label",
            "-p", "--publish"},
    "exec": {"-e", "--env", "-u", "--user", "-w", "--workdir", "--index"},
    "up": {"--scale", "-t", "--timeout", "--exit-code-from", "--attach", "--no-attach", "--pull", "--wait-timeout"},
    "logs": {"-n", "--tail", "--since", "--until"},
    "restart": {"-t", "--timeout"}, "stop": {"-t", "--timeout"},
    "build": {"--build-arg", "-m", "--memory", "--ssh", "--progress"},
}
COMPOSE_ONE_SERVICE = {"run", "exec", "port", "attach"}
COMPOSE_SERVICES = {"up", "logs", "build", "restart", "stop", "start", "ps", "pull", "push", "rm", "kill", "top", "pause",
                    "unpause", "create", "images", "events", "wait", "watch"} | COMPOSE_ONE_SERVICE
SERVICE_MIN_SELECTED = 3
SERVICE_NAME = re.compile(r"[a-z][a-z0-9_-]*")
TOKEN = re.compile(r"[a-z0-9]+")
stem = Stemmer.Stemmer("english").stemWord


def words(text):
    """(start, end, base) of each word, a hyphenated compound as one word; a contraction's base drops its ending
    (what's -> what, isn't -> isn)."""
    for m in WORD.finditer(text):
        w = m.group().lower()
        base = w[:-3] if w.endswith("n't") else w.split("'")[0]
        yield m.start(), m.end(), base


def stems(text):
    """The stems of the words of text, a compound's parts apart, so that sub-release and sub release match."""
    return tuple(stem(p) for _, _, b in words(text) for p in b.split("-"))


def wstems(text):
    """The stems of the words of text, a compound as one word, so that subrelease and sub-release match."""
    return tuple(stem(b.replace("-", "")) for _, _, b in words(text))


def parts(text):
    """(start, end, stem) of each part of each word, a compound's parts apart."""
    for s, e, _ in words(text):
        offset = s
        for piece in text[s:e].lower().split("-"):
            word = piece[:-3] if piece.endswith("n't") else piece.split("'")[0]
            yield offset, offset + len(word), stem(word)
            offset += len(piece) + 1


def exact(term, text):
    """Whether text holds term as a whole token: a letter, digit or underscore before or after is another token, and
    so is a dot, hyphen or slash that joins it to one, while sentence punctuation after it is not."""
    head = r"(?<![A-Za-z0-9_])(?<![A-Za-z0-9_][.\-/\\])" if re.match(r"\w", term) else ""
    return bool(re.search(head + re.escape(term) + r"(?![A-Za-z0-9_])(?![.\-/\\][A-Za-z0-9_])", text))


def has_run(haystack, needle):
    return bool(needle) and any(haystack[i:i + len(needle)] == needle for i in range(len(haystack) - len(needle) + 1))


def found(entry, text, equivalents=True):
    """Whether text holds the entry in the form its kind allows, or one of its equivalents."""
    term, kind = entry["term"], entry["kind"]
    if kind == "identifier" and exact(term, text):
        return True
    if kind == "acronym" and re.search(rf"(?<![A-Za-z0-9_]){re.escape(term.rstrip('s'))}(?:e?s)?(?![A-Za-z0-9_])", text):
        return True
    have, whole = stems(text), wstems(text)

    def holds(t):
        return has_run(have, stems(t)) or has_run(whole, wstems(t))

    return (kind == "words" and holds(term)) or (equivalents and any(holds(e) for e in entry["equivalents"]))


def topic(text):
    return [(s, e) for s, e, b in words(text) if b not in STOP and b not in FRAMING]


def developer_messages(repo_dir):
    seen, out = set(), []
    for f in sorted(repo_dir.glob("*.jsonl")):
        for line in f.read_text(encoding="utf-8").split("\n"):
            if not line.strip():
                continue
            try:
                e = json.loads(line)
            except ValueError:
                continue
            role = e.get("type") if e.get("type") in ("user", "assistant") else e.get("role")
            if role != "user" or e.get("isMeta") or e.get("isSidechain") or e.get("isCompactSummary") \
                    or e.get("toolUseResult") is not None:
                continue
            if e.get("uuid") in seen and e.get("uuid"):
                continue
            seen.add(e.get("uuid"))
            content = (e.get("message") or {}).get("content")
            if isinstance(content, str):
                blocks = [content]
            elif isinstance(content, list) and not any(isinstance(b, dict) and b.get("type") == "tool_result"
                                                        for b in content):
                blocks = [b.get("text", "") for b in content if isinstance(b, dict) and b.get("type") == "text"]
            else:
                blocks = []
            for t in blocks:
                t = t.strip()
                if t and not t.startswith(SKIP):
                    m = re.search(r"<user_query>(.*?)</user_query>", t, re.S)
                    out.append(m.group(1) if m else t)
    return out


def typed_counts(messages):
    """For each run of one to three stems, the number of messages that hold it."""
    counts = Counter()
    for m in messages:
        s = stems(m)
        counts.update({s[i:i + n] for n in (1, 2, 3) for i in range(len(s) - n + 1)})
    return counts


def compose_services(tokens):
    """The services a `docker compose <subcommand> ...` or `docker-compose ...` command line selects, or none. Global
    flags and each subcommand's own flags are skipped with their values (-f is a file before the subcommand and
    `follow` after logs); for run, exec, port and attach only the first operand is a service, cp takes the service of a
    `service:path`, and an operand that is a path, a number or an assignment is never one."""
    base = [t.split("/")[-1] for t in tokens]
    start = next((i + 1 for i, t in enumerate(base) if t in ("docker-compose", "compose")), None)
    if start is None:
        return []
    i = start
    while i < len(tokens) and tokens[i].startswith("-"):
        i += 2 if tokens[i] in COMPOSE_GLOBAL_VALUE_FLAGS else 1
    if i >= len(tokens) or tokens[i] not in COMPOSE_SERVICES | {"cp"}:
        return []
    sub, operands, i = tokens[i], [], i + 1
    while i < len(tokens):
        if tokens[i].startswith("-"):
            i += 2 if tokens[i] in COMPOSE_VALUE_FLAGS.get(sub, ()) else 1
        else:
            operands.append(tokens[i])
            i += 1
    if sub == "cp":
        return [o.split(":", 1)[0] for o in operands if ":" in o and SERVICE_NAME.fullmatch(o.split(":", 1)[0])]
    names = [t for t in operands if SERVICE_NAME.fullmatch(t)]
    return names[:1] if sub in COMPOSE_ONE_SERVICE else names


def declared_services(text):
    """The service names of the Compose `services:` blocks in text: the keys at the first indentation level under
    `services:`, whatever follows the colon; nested keys such as `volumes:` never count."""
    out = []
    for m in re.finditer(r"^services:[ \t]*\n((?:[ \t]+\S.*\n?|[ \t]*\n)+)", text.expandtabs(2), re.M):
        lines = [line for line in m.group(1).split("\n") if line.strip() and not line.lstrip().startswith("#")]
        if lines:
            base = len(lines[0]) - len(lines[0].lstrip())
            out += [k.group(1) for line in lines if len(line) - len(line.lstrip()) == base
                    for k in [re.match(r" *([a-z][a-z0-9_-]*):(?:[ \t]|$)", line)] if k]
    return out


def strings(value):
    if isinstance(value, str):
        yield value
    elif isinstance(value, list):
        for v in value:
            yield from strings(v)
    elif isinstance(value, dict):
        for v in value.values():
            yield from strings(v)


def command_words(repo_dir):
    """(counts, services). counts: (word, position) over the command and subcommand of every shell command run in the
    repo's sessions, with quoted text and heredocs left out and a wrapper such as nix or npx counted as the program it is
    before it is unwrapped (its own subcommands never count). The second word counts only for a program that takes
    subcommands, not one whose arguments are operands (OPERAND_PROGRAMS), and never when it is a path. services: name ->
    where the project names it as one, a Compose `services:` key in the sessions or at least SERVICE_MIN_SELECTED commands
    that select it."""
    counts, selected, services = Counter(), Counter(), {}
    for f in sorted(repo_dir.glob("*.jsonl")):
        for line in f.read_text(encoding="utf-8").split("\n"):
            declared = "services:" in line
            if '"tool_use"' not in line and not declared:
                continue
            try:
                e = json.loads(line)
            except ValueError:
                continue
            if declared:
                for text in strings(e):
                    for name in declared_services(text):
                        services.setdefault(name, "compose services: key")
            content = (e.get("message") or {}).get("content")
            for b in content if isinstance(content, list) else []:
                cmd = (b.get("input") or {}).get("command") if isinstance(b, dict) and b.get("type") == "tool_use" \
                    and isinstance(b.get("input"), dict) else None
                if not isinstance(cmd, str):
                    continue
                for seg in SHELL_SEP.split(QUOTED.sub(" ", QUOTED_WORD.sub(r"\2", cmd.split("<<")[0]))):
                    for name in compose_services(seg.split()):
                        selected[name] += 1
                    toks = [t for t in seg.split() if "=" not in t]
                    while toks and toks[0].split("/")[-1] in WRAPPERS:
                        w, toks = toks[0].split("/")[-1], toks[1:]
                        if w in WRAPPER_PROGRAMS:
                            counts[(w, 0)] += 1
                        if w in SUBCOMMAND_WRAPPERS and toks and toks[0].split("/")[-1] not in WRAPPERS:
                            toks = []
                    for i, t in enumerate(toks[:2]):
                        if t.startswith("-") or (i == 1 and (toks[0].split("/")[-1] in OPERAND_PROGRAMS or "/" in t)):
                            break
                        w = t.split("/")[-1].lower()
                        if re.fullmatch(r"[a-z][a-z0-9-]*", w):
                            counts[(w, i)] += 1
    for name, n in selected.items():
        if n >= SERVICE_MIN_SELECTED:
            services.setdefault(name, f"compose command {n}x")
    return counts, services


def raw_counts(repo_dir, wanted):
    """How often each wanted token run (with a plural ending on its last token) occurs anywhere in the repo's
    sessions, tool output and code included, and how many tokens they hold, under the key ()."""
    forms = {}
    for t in wanted:
        for end in ("", "s", "es"):
            forms.setdefault(t[:-1] + (t[-1] + end,), []).append(t)
    longest = max(map(len, forms), default=1)
    counts = Counter()
    for f in sorted(repo_dir.glob("*.jsonl")):
        window = []
        for m in TOKEN.finditer(f.read_text(encoding="utf-8").lower()):
            counts[()] += 1
            window.append(m.group())
            if len(window) > longest:
                window.pop(0)
            for n in range(1, len(window) + 1):
                for hit in forms.get(tuple(window[-n:]), ()):
                    counts[hit] += 1
    return counts


def runs(text, taken):
    """The runs of topic words outside the taken spans, as lists of (start, end)."""
    out, run = [], []
    for s, e, b in list(words(text)) + [(len(text), len(text), ".")]:
        if b == "." or b in STOP or b in FRAMING or any(s < te and ts < e for ts, te in taken):
            if run:
                out.append(run)
            run = []
        else:
            run.append((s, e))
    return out


def build(corpus, questions):
    rows = [q for q in questions if not q["abstain"]]
    repos = sorted(p.name for p in corpus.iterdir() if p.is_dir())
    typed = {r: typed_counts(developer_messages(corpus / r)) for r in repos}
    both = {r: command_words(corpus / r) for r in repos}
    commands, services = {r: b[0] for r, b in both.items()}, {r: b[1] for r, b in both.items()}
    prompt = {q["id"]: q["wording"]["original"]["prompt"] for q in rows}
    wanted = {tuple(TOKEN.findall(prompt[q["id"]][s:e].lower()))
              for q in rows for run in runs(prompt[q["id"]], []) for n in (1, 2, 3)
              for i in range(len(run) - n + 1) for s, e in [(run[i][0], run[i + n - 1][1])]} - {()}
    raw = {r: raw_counts(corpus / r, wanted) for r in repos}

    def spread(table, key, least=1):
        return sum(1 for r in repos if table[r][key] >= least)

    def cmd_total(r, w):
        return sum(commands[r][(w, i)] for i in range(2))

    def specific(r, toks):
        """A word's share of the repo's tokens over its mean share in the other repos."""
        others = sum(raw[o][toks] / raw[o][()] for o in repos if o != r) / (len(repos) - 1)
        return raw[r][toks] / raw[r][()] / others if others else float("inf")

    out = []
    for q in rows:
        text, repo = prompt[q["id"]], q["repo"]
        kept = []

        def keep(s, e, kind, why):
            if not any(s < ke and ks < e for ks, ke, _, _ in kept):
                kept.append((s, e, kind, why))

        for m in IDENTIFIER.finditer(text):
            term = m.group().rstrip(".,?:;")
            keep(m.start(), m.start() + len(term), "identifier", "identifier")
        for m in ACRONYM.finditer(text):
            keep(m.start(), m.end(), "acronym", "acronym")
        for m in re.finditer(r"(?<=[a-z,] )[A-Z][a-z]+\w*", text):
            keep(m.start(), m.end(), "words", "name")
        positions, inside = list(parts(text)), []
        starts, ends = {s for s, _, _ in words(text)}, {e for _, e, _ in words(text)}
        for t in TERMS_OF_ART:
            want = tuple(p for _, _, p in parts(t))
            for i in range(len(positions) - len(want) + 1):
                if tuple(p for _, _, p in positions[i:i + len(want)]) == want:
                    span = (positions[i][0], positions[i + len(want) - 1][1])
                    if span[0] in starts and span[1] in ends:
                        keep(*span, "words", "term of art")
                    else:
                        inside.append(span)
        for run in runs(text, [(s, e) for s, e, _, _ in kept]):
            for n in (3, 2, 1):
                for i in range(len(run) - n + 1):
                    s, e = run[i][0], run[i + n - 1][1]
                    if any(s < ke and ks < e for ks, ke, _, _ in kept):
                        continue
                    phrase = text[s:e]
                    st, toks = stems(phrase), tuple(TOKEN.findall(phrase.lower()))
                    if n > 1 and commands[repo][(toks[0], 0)] >= CMD_MIN and raw[repo][toks] >= MIN_RAW:
                        keep(s, e, "words", f"program phrase {raw[repo][toks]}x")
                    elif n == 1 and commands[repo][(toks[0], 0)] >= CMD_MIN:
                        keep(s, e, "words", f"program {commands[repo][(toks[0], 0)]}x")
                    elif n == 1 and cmd_total(repo, toks[0]) >= CMD_MIN and \
                            sum(1 for r in repos if cmd_total(r, toks[0]) >= 3) <= CMD_SPREAD:
                        keep(s, e, "words", f"command word {cmd_total(repo, toks[0])}x")
                    elif n == 1 and toks[0] in services[repo]:
                        keep(s, e, "words", f"service ({services[repo][toks[0]]})")
                    elif typed[repo][st] >= MIN_TYPED and spread(typed, st) <= DEV_SPREAD and \
                            (n > 1 or specific(repo, toks) >= SPECIFIC):
                        keep(s, e, "words", f"typed {typed[repo][st]}x")
                    elif raw[repo][toks] >= MIN_RAW and (
                            n == 1 and (spread(raw, toks, 3) <= RAW_SPREAD or specific(repo, toks) >= SPECIFIC) or
                            n > 1 and spread(raw, toks, 3) <= RAW_SPREAD and
                            raw[repo][toks] >= COHESION * min(raw[repo][(w,)] for w in toks)):
                        keep(s, e, "words", f"project word {raw[repo][toks]}x")
        for span in inside:
            keep(*span, "words", "term of art")
        entries, listed = [], set()
        for s, e, kind, why in sorted(kept):
            term = text[s:e]
            if (term.lower(), kind) in listed:
                continue
            listed.add((term.lower(), kind))
            alts = [b if stems(a) == stems(term) else a for a, b in EQUIVALENTS if stems(term) in (stems(a), stems(b))]
            entries.append({"term": term, "kind": kind, "why": why,
                            "equivalents": [a for a in alts if typed[repo][stems(a)] > 0]})
        framing_only = all(any(ks <= s and e <= ke for ks, ke, _, _ in kept) for s, e in topic(text))
        out.append({"id": q["id"], "repo": repo, "keep": entries, "framing_only": framing_only})
    return out


if __name__ == "__main__":
    corpus, qfile, dst = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
    qs = [json.loads(line) for line in qfile.read_text(encoding="utf-8").split("\n") if line.strip()]
    rows = build(corpus, qs)
    dst.write_text("".join(json.dumps(r, ensure_ascii=False) + "\n" for r in rows), encoding="utf-8")
    prompts = {q["id"]: q["wording"]["original"]["prompt"] for q in qs}
    for r in rows:
        terms = "; ".join(f"{t['term']} ({t['kind']}, {t['why']}" + "".join(f", or {e}" for e in t["equivalents"])
                          + ")" for t in r["keep"])
        print(f"{r['id']}: {prompts[r['id']]}\n    keep: {terms or '-'}"
              + (" [framing only]" if r["framing_only"] else ""))
    print(f"keep: {len(rows)} questions, {sum(len(r['keep']) for r in rows)} kept terms, "
          f"{sum(r['framing_only'] for r in rows)} framing-only", file=sys.stderr)
