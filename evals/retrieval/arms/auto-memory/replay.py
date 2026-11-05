"""Replay a recorded Bash command against the memory store without running it: a small shell that knows the commands
memory writes use, and a Python evaluator that knows only string operations and file reads and writes. Anything else
that may write a memory file is reported, never guessed."""

import ast
import posixpath
import re

HOME = "/home/~"


class Unparsed(Exception):
    """A memory write this replay cannot follow; the message says why."""


class Unknown(Exception):
    """A step that reads a memory file the store does not hold."""


# --- the shell's words and statements


def heredocs(command):
    """The command with every heredoc body cut out, and the bodies: (text, [(body, literal)]). The operator stays in
    the text as `<<\0<n>` so the tokenizer can tie the body to its command."""
    lines, out, bodies, i = command.split("\n"), [], [], 0
    while i < len(lines):
        line, pending = lines[i], []

        def cut(m):
            pending.append((m.group(3), m.group(1) == "-", bool(m.group(2))))
            return f"<<\0{len(bodies) + len(pending) - 1}"

        line = re.sub(r"<<(-?)\s*(['\"]?)([A-Za-z_][A-Za-z0-9_]*)\2", cut, line) if "<<<" not in line else line
        out.append(line)
        i += 1
        for delim, strip, quoted in pending:
            body = []
            while i < len(lines) and (lines[i].lstrip("\t") if strip else lines[i]) != delim:
                body.append(lines[i].lstrip("\t") if strip else lines[i])
                i += 1
            if i == len(lines):
                raise Unparsed("heredoc without its end")
            i += 1
            bodies.append(("\n".join(body) + "\n", quoted))
    return "\n".join(out), bodies


OPERATORS = ["&&", "||", ";;", ">>", "&>", ">|", ";", "|", "&", "(", ")", "\n", ">", "<"]


def tokens(text):
    """Shell tokens: words as lists of parts (str, or ("var", name), or ("dynamic",) for a substitution or a glob),
    operators as ("op", text), redirection fds folded into the operator ("2>")."""
    out, word, i, started = [], [], 0, False

    def flush():
        nonlocal word, started
        if started:
            out.append(("word", word))
        word, started = [], False

    def lit(s):
        nonlocal started
        started = True
        if word and isinstance(word[-1], str):
            word[-1] += s
        else:
            word.append(s)

    def var(j, quoted):
        nonlocal started
        started = True
        m = re.match(r"\$(?:\{([A-Za-z_][A-Za-z0-9_]*)\}|([A-Za-z_][A-Za-z0-9_]*))", text[j:])
        if m:
            word.append(("var", m.group(1) or m.group(2)))
            return j + m.end()
        if text.startswith("$(", j) or text.startswith("${", j):
            word.append(("dynamic",))
            depth, k = 0, j + 1
            while k < len(text):
                depth += {"(": 1, "{": 1, ")": -1, "}": -1}.get(text[k], 0)
                k += 1
                if depth == 0:
                    break
            return k
        word.append(("dynamic",)) if j + 1 < len(text) and text[j + 1] in "?#@*!0123456789-" else lit("$")
        return j + (2 if j + 1 < len(text) and text[j + 1] in "?#@*!0123456789-" else 1)

    while i < len(text):
        c = text[i]
        if c == "\0":
            m = re.match(r"\0(\d+)", text[i:])
            flush()
            out.append(("heredoc", int(m.group(1))))
            i += m.end()
        elif c in " \t":
            flush()
            i += 1
        elif c == "\\" and i + 1 < len(text):
            if text[i + 1] != "\n":
                lit(text[i + 1])
            i += 2
        elif c == "'":
            j = text.index("'", i + 1) if "'" in text[i + 1:] else -1
            if j < 0:
                raise Unparsed("unclosed quote")
            lit(text[i + 1:j])
            i = j + 1
        elif c == '"':
            i += 1
            lit("")
            while i < len(text) and text[i] != '"':
                if text[i] == "\\" and i + 1 < len(text) and text[i + 1] in '$`"\\\n':
                    if text[i + 1] != "\n":
                        lit(text[i + 1])
                    i += 2
                elif text[i] == "$":
                    i = var(i, True)
                elif text[i] == "`":
                    word.append(("dynamic",))
                    i = text.index("`", i + 1) + 1 if "`" in text[i + 1:] else len(text)
                else:
                    lit(text[i])
                    i += 1
            if i >= len(text):
                raise Unparsed("unclosed quote")
            i += 1
        elif c == "$":
            i = var(i, False)
        elif c == "`":
            word.append(("dynamic",))
            started = True
            i = text.index("`", i + 1) + 1 if "`" in text[i + 1:] else len(text)
        elif c == "#" and not started:
            while i < len(text) and text[i] != "\n":
                i += 1
        elif c in "*?[" and c != "[" or (c == "[" and started):
            word.append(("dynamic",))
            started = True
            i += 1
        elif c in ";&|()<>\n":
            fd = ""
            if word and len(word) == 1 and isinstance(word[0], str) and word[0].isdigit() and c in "<>" and text[i - 1].isdigit():
                fd, word, started = word[0], [], False
            flush()
            op = next(o for o in OPERATORS if text.startswith(o, i))
            i += len(op)
            if op in (">", ">>", ">|", "&>", "<") and text.startswith("&", i):
                m = re.match(r"&(\d+|-)", text[i:])
                i += m.end() if m else 0
                out.append(("op", fd + op + "&"))
                continue
            out.append(("op", fd + op))
        else:
            lit(c)
            i += 1
    flush()
    return out


def statements(toks):
    """Simple commands joined by how they run: [(connector, [pipeline commands])], each command a list of tokens.
    The connector is ";" (always runs), "&&" or "||"."""
    out, pipeline, cmd, conn = [], [], [], ";"
    for kind, val in toks + [("op", "\n")]:
        if kind == "op" and val in (";", "\n", "&&", "||", "&"):
            if cmd:
                pipeline.append(cmd)
            if pipeline:
                out.append((conn, pipeline))
            pipeline, cmd = [], []
            conn = val if val in ("&&", "||") else ";"
        elif kind == "op" and val == "|":
            pipeline.append(cmd)
            cmd = []
        else:
            cmd.append((kind, val))
    return out


CONTROL = {"for", "do", "done", "if", "then", "else", "elif", "fi", "while", "until", "case", "esac", "{", "}", "(", ")"}
QUIET = {"cd", "mkdir", "true", "false", "test", "[", "sleep", "export", "set", ":"}
READERS = {"ls", "head", "tail", "grep", "rg", "wc", "find", "stat", "sort", "diff", "git", "gh", "date", "pwd", "tree",
           "file", "du", "which", "basename", "dirname", "realpath", "readlink", "md5sum", "sha256sum", "awk", "cut", "tr",
           "uniq", "jq", "less", "more", "column", "nl", "xxd", "od", "hexdump", "type", "env", "printenv", "fd", "comm"}
PYTHONS = re.compile(r"^(python3?(\.\d+)?|py)$")


class Command:
    """One simple command: its words, its redirections and its heredoc stdin."""

    def __init__(self, toks):
        self.words, self.redirects, self.stdin, self.problem = [], [], None, None
        i = 0
        while i < len(toks):
            kind, val = toks[i]
            if kind == "heredoc":
                self.stdin = val
            elif kind == "op" and val in ("(", ")"):
                self.problem = "subshell"
            elif kind == "op":
                target = toks[i + 1][1] if not val.endswith("&") and i + 1 < len(toks) and toks[i + 1][0] == "word" else None
                self.redirects.append((val, target))
                i += target is not None
            else:
                self.words.append(val)
            i += 1


# --- the replay


class Shell:
    """Replays one recorded command against `store`, which reads, writes and deletes memory files by absolute path and
    records what each step did. `snapshots` maps a statement's position to the text the transcript shows it printed."""

    def __init__(self, store, cwd):
        self.store, self.cwd, self.cwd_start, self.env = store, cwd, cwd, {"HOME": HOME}

    def expand(self, word):
        out = ""
        for part in word:
            if isinstance(part, str):
                out += part
            elif part[0] == "var" and part[1] in self.env:
                out += self.env[part[1]]
            else:
                return None
        return out

    def path(self, word):
        text = self.expand(word)
        if text is None:
            return None
        text = text.replace("\\", "/")
        if text == "~" or text.startswith("~/"):
            text = HOME + text[1:]
        return posixpath.normpath(text if text.startswith("/") or re.match(r"[A-Za-z]:/", text) else posixpath.join(self.cwd, text))

    def mentions_memory(self, cmd):
        return any(self.store.is_memory(self.path(w) or self.expand_loose(w)) for w in cmd.words + [t for _, t in cmd.redirects if t])

    def expand_loose(self, word):
        return "".join(p if isinstance(p, str) else self.env.get(p[1], "") if p[0] == "var" else "" for p in word).replace("\\", "/")

    def out_target(self, cmd):
        """Where stdout goes: (path or None for a non-file, append?) of the last stdout redirection, else None."""
        found = None
        for op, target in cmd.redirects:
            if op in (">", ">>", ">|", "1>", "1>>", "&>"):
                found = (self.path(target) if target else None, op.endswith(">>"))
        return found

    def run(self, command, failed, stdout):
        """Replay the command. A recorded failure keeps the last list from being applied: some step of it failed, and
        which one is unknown. Returns what each output step printed, for the snapshots."""
        text, bodies = heredocs(command)
        stmts = statements(tokens(text))
        cmds = [(conn, [Command(t) for t in pipe]) for conn, pipe in stmts]
        snaps = self.snapshots(cmds, bodies, stdout) if stdout is not None and not failed else {}
        last = max((i for i, (conn, _) in enumerate(cmds) if conn == ";"), default=0)
        ok = True
        for i, (conn, pipe) in enumerate(cmds):
            if failed and i >= last:
                if any(self.writes(c) for _, p in cmds[i:] for c in p):
                    self.store.effect("", "recorded_failed", "bash")
                return
            if (conn == "&&" and not ok) or (conn == "||" and ok):
                continue
            ok, piped = True, None
            for j, cmd in enumerate(pipe):
                ok = self.step(cmd, bodies, snaps.get((i, j)), piped) and ok
                name = self.expand(cmd.words[0]) if cmd.words else None
                piped = self.output(name, cmd.words, [self.expand(w) for w in cmd.words[1:]], self.stdin(cmd, bodies)) \
                    if name in ("echo", "printf") and not cmd.redirects else None

    def writes(self, cmd):
        target = self.out_target(cmd)
        name = (self.expand(cmd.words[0]) or "") if cmd.words else ""
        return bool(target and target[0] and self.store.is_memory(target[0])) or (
            self.mentions_memory(cmd) and name not in READERS | QUIET | {"cat", "echo", "printf"})

    def stdin(self, cmd, bodies):
        return bodies[cmd.stdin] if cmd.stdin is not None else None

    def step(self, cmd, bodies, snapshot, piped=None):
        """Run one simple command; False when it would have failed."""
        if not cmd.words:
            return True
        words = list(cmd.words)
        name = self.expand(words[0])
        while name in ("command", "builtin", "exec", "nohup", "time") and len(words) > 1:
            words.pop(0)
            name = self.expand(words[0])
        assign = re.match(r"^([A-Za-z_][A-Za-z0-9_]*)=", words[0][0]) if words and isinstance(words[0][0], str) else None
        if name == "export" and len(words) > 1:
            words, name = words[1:], None
            assign = re.match(r"^([A-Za-z_][A-Za-z0-9_]*)=", words[0][0]) if isinstance(words[0][0], str) else None
        if assign and len(words) == 1:
            value = self.expand([words[0][0][assign.end():]] + words[0][1:])
            self.env[assign.group(1)] = value if value is not None else "\0unknown"
            if value is None:
                self.env.pop(assign.group(1))
            return True
        target = self.out_target(cmd)
        to_memory = bool(target and target[0] and self.store.is_memory(target[0]))
        stdin = self.stdin(cmd, bodies) if piped is None else (piped, True)
        args = [self.expand(w) for w in words[1:]]
        if cmd.problem or name is None or name in CONTROL:
            return self.unparsed(cmd, "shell syntax it does not follow") if self.writes(cmd) else True
        if name == "cd":
            where = self.path(words[1]) if len(words) > 1 else HOME
            self.cwd = where or "/unknown"
            return True
        if name in ("cat", "echo", "printf", "tee"):
            text = self.output(name, words, args, stdin)
            if name == "tee":
                files = [self.path(w) for w in words[1:] if (self.expand(w) or "-").lstrip("-") == (self.expand(w) or "")]
                append = "-a" in args
                if text is None:
                    return self.unparsed(cmd, "tee input it does not know") if any(f and self.store.is_memory(f) for f in files) else True
                for f in files:
                    if f and self.store.is_memory(f):
                        self.store.put(f, text, append, "bash")
                return True
            if name == "cat" and not to_memory and snapshot is not None:
                files = [a for a in args if not a.startswith("-")]
                self.store.snapshot(self.path([files[0]]), snapshot)
                return True
            if to_memory:
                if text is None:
                    return self.unparsed(cmd, f"{name} output it cannot know")
                if target[1] and self.store.unseen(target[0]):
                    self.store.effect(target[0], "bash_file_unknown", "bash")
                else:
                    self.store.put(target[0], text, target[1], "bash")
            return True
        if to_memory:
            return self.unparsed(cmd, f"{name} writing into memory")
        if name == "rm":
            for a, w in zip(args, words[1:]):
                if a is None:
                    return self.unparsed(cmd, "rm of a path it cannot expand") if self.mentions_memory(cmd) else True
                if a.startswith("-"):
                    if "r" in a and self.mentions_memory(cmd):
                        return self.unparsed(cmd, "rm -r")
                    continue
                p = self.path(w)
                if self.store.is_memory(p):
                    self.store.delete(p)
            return True
        if name in ("mv", "cp"):
            paths = [self.path(w) for w, a in zip(words[1:], args) if not (a or "").startswith("-")]
            if not self.mentions_memory(cmd):
                return True
            if len(paths) != 2 or None in paths or not all(self.store.is_memory(p) for p in paths):
                return self.unparsed(cmd, f"{name} into or out of memory")
            src, dst = paths
            if not self.store.exists(src):
                self.store.effect(dst, "bash_file_unknown", "bash")
                return True
            if self.store.is_dir(dst):
                dst = posixpath.join(dst, posixpath.basename(src))
            self.store.put(dst, self.store.read(src), False, "bash")
            if name == "mv":
                self.store.delete(src)
            return True
        if name == "sed" and any(a and re.match(r"^-[a-zA-Z]*i", a) for a in args):
            return self.sed(cmd, words, args) if self.mentions_memory(cmd) else True
        if name == "uv" and args[:2] == ["run", "python"] or name == "uv" and args[:2] == ["run", "python3"]:
            words, args, name = words[2:], args[2:], "python"
        if PYTHONS.match(name):
            return self.python(cmd, args, stdin)
        if name in READERS or name in QUIET:
            return True
        return self.unparsed(cmd, f"{name} touching memory") if self.writes(cmd) else True

    def output(self, name, words, args, stdin):
        """What cat, echo, printf or tee prints, or None when it depends on something unknown."""
        if None in args:
            return None
        if name == "echo":
            flags = []
            while args and re.fullmatch(r"-[neE]+", args[0]):
                flags.append(args.pop(0))
            if any("e" in f for f in flags) and any("\\" in a for a in args):
                return None
            return " ".join(args) + ("" if any("n" in f for f in flags) else "\n")
        if name == "printf":
            if args[:1] == ["--"]:
                args = args[1:]
            return printf(args[0], args[1:]) if args else None
        if name == "tee":
            return stdin[0] if stdin and (stdin[1] or not re.search(r"[$`\\]", stdin[0])) else None
        files = [w for w, a in zip(words[1:], args) if not a.startswith("-")]
        if not files:
            return stdin[0] if stdin and (stdin[1] or not re.search(r"[$`\\]", stdin[0])) else None
        text = ""
        for w in files:
            p = self.path(w)
            if not (p and self.store.is_memory(p) and self.store.exists(p)):
                return None
            text += self.store.read(p)
        return text

    def sed(self, cmd, words, args):
        files, script, i = [], None, 0
        rest = list(zip(words[1:], args))
        while i < len(rest):
            w, a = rest[i]
            if a is None:
                return self.unparsed(cmd, "sed with an unknown argument")
            if re.match(r"^-[a-zA-Z]*i", a):
                if a == "-i" and i + 1 < len(rest) and rest[i + 1][1] == "":
                    i += 1
            elif a == "-e" and script is None and i + 1 < len(rest):
                script, i = rest[i + 1][1], i + 1
            elif a.startswith("-"):
                return self.unparsed(cmd, f"sed option {a}")
            elif script is None:
                script = a
            else:
                files.append(self.path(w))
            i += 1
        sub = parse_sed(script or "")
        if sub is None:
            return self.unparsed(cmd, "sed script beyond one s command")
        for f in files:
            if not (f and self.store.is_memory(f)):
                continue
            if not self.store.exists(f):
                self.store.effect(f, "bash_file_unknown", "bash")
                continue
            pattern, repl, count = sub
            self.store.put(f, "".join(pattern.sub(repl, line, count=count) for line in self.store.read(f).splitlines(True)), False, "bash")
        return True

    def python(self, cmd, args, stdin):
        if "-c" in args:
            code = args[args.index("-c") + 1] if args.index("-c") + 1 < len(args) else None
        elif (not args or args[0] == "-") and stdin:
            code = stdin[0] if stdin[1] or not re.search(r"[$`\\]", stdin[0]) else None
        else:
            return self.unparsed(cmd, "python running a script file") if self.writes(cmd) else True
        if code is None:
            return self.unparsed(cmd, "python code it cannot read")
        if "/.claude/projects/" not in code and ".claude" not in code and not self.store.is_memory(self.cwd + "/x"):
            return True
        try:
            Python(self.store, self.cwd).run(code)
        except Unparsed as e:
            return self.unparsed(cmd, f"python: {e}")
        except Unknown as e:
            self.store.effect(str(e), "bash_file_unknown", "bash")
        except PyFail:
            return False
        return True

    def unparsed(self, cmd, reason):
        target = self.out_target(cmd)
        paths = [target[0]] if target and target[0] else []
        paths += [self.path(w) for w in cmd.words[1:]]
        self.store.unparsed(next((p for p in paths if p and self.store.is_memory(p)), ""), reason)
        return True

    def snapshots(self, cmds, bodies, stdout):
        """The text each lone `cat` of one memory file printed, read off the recorded output between the literal lines
        echoed around it: {(statement, command): text}."""
        items = []
        for i, (_, pipe) in enumerate(cmds):
            if len(pipe) > 1:
                items.append(("unknown",))
                continue
            cmd = pipe[0]
            if not cmd.words:
                continue
            name = self.expand(cmd.words[0])
            if name and re.match(r"^[A-Za-z_][A-Za-z0-9_]*=", name) and len(cmd.words) == 1:
                value = self.expand([cmd.words[0][0].split("=", 1)[1]] + cmd.words[0][1:])
                if value is not None:
                    self.env[name.split("=", 1)[0]] = value
                continue
            if name == "cd":
                self.cwd = (self.path(cmd.words[1]) if len(cmd.words) > 1 else HOME) or "/unknown"
                continue
            target = self.out_target(cmd)
            if target or name in QUIET or name == "rm":
                continue
            args = [self.expand(w) for w in cmd.words[1:]]
            if name == "echo" and None not in args:
                items.append(("lit", self.output("echo", cmd.words, list(args), None)))
            elif name == "cat" and None not in args and cmd.stdin is None:
                files = [w for w, a in zip(cmd.words[1:], args) if not a.startswith("-")]
                p = self.path(files[0]) if len(files) == 1 else None
                items.append(("cat", (i, 0)) if p and self.store.is_memory(p) else ("unknown",))
            else:
                items.append(("unknown",))
        self.env, self.cwd = {"HOME": HOME}, self.cwd_start
        if not any(k == "cat" for k, *_ in items):
            return {}
        usable = {n for n, it in enumerate(items) if it[0] == "cat"
                  and all(items[m][0] == "lit" for m in (n - 1, n + 1) if 0 <= m < len(items))}
        while items and items[-1][0] == "lit" and items[-1][1].strip() == "":
            items.pop()
        pattern, groups = "", []
        for n, it in enumerate(items):
            if it[0] == "lit":
                pattern += re.escape(it[1] if n < len(items) - 1 else it[1].rstrip("\n"))
            elif it[0] == "cat" and n in usable:
                pattern += "(.*?)"
                groups.append(it[1])
            else:
                pattern += ".*?"
        m = re.fullmatch(pattern, stdout.rstrip("\n"), re.S)
        if not m:
            self.store.read_unusable("output does not match the command", len(usable))
            return {}
        self.store.read_unusable("cat beside other output", sum(1 for it in items if it[0] == "cat") - len(usable))
        return {where: m.group(g + 1) for g, where in enumerate(groups)}


def printf(fmt, args):
    """bash's printf for %s, %% and the common backslash escapes; None for anything else."""
    if re.search(r"%(?![s%])", fmt) or re.search(r"\\(?![ntr\\\"'])", fmt):
        return None
    esc = {"n": "\n", "t": "\t", "r": "\r", "\\": "\\", '"': '"', "'": "'"}
    fmt = re.sub(r"\\(.)", lambda m: esc[m.group(1)], fmt)
    slots = len(re.findall(r"%s", fmt))
    out, args = "", list(args)
    while True:
        chunk, used = "", 0
        for piece in re.split(r"(%s|%%)", fmt):
            if piece == "%s":
                chunk += args[used] if used < len(args) else ""
                used += 1
            else:
                chunk += "%" if piece == "%%" else piece
        out += chunk
        args = args[used:]
        if not slots or not args:
            return out


def parse_sed(script):
    """One `s` command of GNU sed as (compiled pattern, replacement function, count), or None."""
    m = re.fullmatch(r"s(.)(.*)", script, re.S)
    if not m:
        return None
    d, body = m.group(1), m.group(2)
    parts, cur, i = [], "", 0
    while i < len(body):
        if body[i] == "\\" and i + 1 < len(body):
            cur += body[i:i + 2] if body[i + 1] != d else d
            i += 2
        elif body[i] == d:
            parts.append(cur)
            cur = ""
            i += 1
        else:
            cur += body[i]
            i += 1
    parts.append(cur)
    if len(parts) != 3 or parts[2] not in ("", "g"):
        return None
    pattern = ""
    i, bre = 0, parts[0]
    while i < len(bre):
        c = bre[i]
        if c == "\\" and i + 1 < len(bre):
            n = bre[i + 1]
            pattern += n if n in "(){}|+?" else "\\" + n
            i += 2
        else:
            pattern += "\\" + c if c in "(){}|+?" else c
            i += 1
    try:
        compiled = re.compile(pattern)
    except re.error:
        return None
    repl = parts[1]

    def replace(match):
        out, i = "", 0
        while i < len(repl):
            if repl[i] == "\\" and i + 1 < len(repl):
                n = repl[i + 1]
                out += match.group(int(n)) or "" if n.isdigit() else "\n" if n == "n" else n
                i += 2
            elif repl[i] == "&":
                out += match.group(0)
                i += 1
            else:
                out += repl[i]
                i += 1
        return out

    return compiled, replace, 0 if parts[2] == "g" else 1


# --- the Python evaluator


class PyFail(Exception):
    """The script would have stopped: an assertion or a lookup failed on text the store holds."""


class PathV:
    def __init__(self, path):
        self.path = path


class FileV:
    def __init__(self, path, mode):
        self.path, self.mode, self.text = path, mode, ""


MODULES = {"io", "pathlib", "os", "os.path", "sys", "re", "json"}
STR_METHODS = {"replace", "rstrip", "lstrip", "strip", "splitlines", "split", "rsplit", "index", "rindex", "find",
               "rfind", "count", "startswith", "endswith", "join", "lower", "upper", "partition", "rpartition"}
LIST_METHODS = {"append", "insert", "index", "count", "extend", "pop", "remove"}
FREE = {"encoding", "newline", "errors", "keepends", "missing_ok", "exist_ok", "parents"}


class Python:
    """Evaluates the statements recorded memory scripts use: assignments, asserts, if, with, for over a list, and
    expressions over strings, lists and paths. Nothing is executed; any other node is Unparsed."""

    def __init__(self, store, cwd):
        self.store, self.cwd, self.env = store, cwd, {}

    def run(self, code):
        try:
            tree = ast.parse(code)
        except SyntaxError as e:
            raise Unparsed("code that does not parse") from e
        self.block(tree.body)

    def block(self, body):
        for node in body:
            self.stmt(node)

    def stmt(self, node):
        if isinstance(node, ast.Import):
            for a in node.names:
                if a.name not in MODULES:
                    raise Unparsed(f"import {a.name}")
                self.env[a.asname or a.name.split(".")[0]] = ("module", a.name if a.asname else a.name.split(".")[0])
        elif isinstance(node, ast.ImportFrom):
            if node.module not in MODULES:
                raise Unparsed(f"from {node.module} import")
            for a in node.names:
                self.env[a.asname or a.name] = ("module", f"{node.module}.{a.name}")
        elif isinstance(node, ast.Assign):
            value = self.expr(node.value)
            for t in node.targets:
                self.assign(t, value)
        elif isinstance(node, ast.AugAssign) and isinstance(node.op, ast.Add) and isinstance(node.target, ast.Name):
            self.env[node.target.id] = self.binop(ast.Add(), self.expr(node.target), self.expr(node.value))
        elif isinstance(node, ast.Expr):
            self.expr(node.value)
        elif isinstance(node, ast.Assert):
            if not self.expr(node.test):
                raise PyFail("assert")
        elif isinstance(node, ast.If):
            self.block(node.body if self.expr(node.test) else node.orelse)
        elif isinstance(node, ast.With):
            for item in node.items:
                value = self.expr(item.context_expr)
                if item.optional_vars is not None:
                    self.assign(item.optional_vars, value)
            self.block(node.body)
        elif isinstance(node, ast.For) and not node.orelse:
            for value in list(self.iterable(self.expr(node.iter))):
                self.assign(node.target, value)
                self.block(node.body)
        elif isinstance(node, ast.Pass):
            pass
        else:
            raise Unparsed(type(node).__name__)

    def assign(self, target, value):
        if isinstance(target, ast.Name):
            self.env[target.id] = value
        elif isinstance(target, (ast.Tuple, ast.List)):
            values = list(value)
            if len(values) != len(target.elts):
                raise PyFail("unpack")
            for t, v in zip(target.elts, values):
                self.assign(t, v)
        else:
            raise Unparsed(f"assignment to {type(target).__name__}")

    def iterable(self, value):
        if isinstance(value, (str, list, tuple)):
            return value
        raise Unparsed("iteration over something other than text or a list")

    def resolve(self, value):
        p = value.path if isinstance(value, PathV) else value
        if not isinstance(p, str):
            raise Unparsed("a path that is not text")
        p = p.replace("\\", "/")
        if p == "~" or p.startswith("~/"):
            p = HOME + p[1:]
        return posixpath.normpath(p if p.startswith("/") or re.match(r"[A-Za-z]:/", p) else posixpath.join(self.cwd, p))

    def read(self, value):
        p = self.resolve(value)
        if not self.store.is_memory(p):
            raise Unparsed("reads a file outside memory")
        if not self.store.exists(p):
            raise Unknown(p)
        return self.store.read(p)

    def write(self, value, text, append=False):
        p = self.resolve(value)
        if self.store.is_memory(p):
            if append and self.store.unseen(p):
                raise Unknown(p)
            self.store.put(p, text, append, "bash")
        return len(text)

    def expr(self, node):
        if isinstance(node, ast.Constant):
            return node.value
        if isinstance(node, ast.Name):
            if node.id in self.env:
                return self.env[node.id]
            if node.id in ("len", "print", "open", "str", "sorted", "any", "all", "list", "int"):
                return ("builtin", node.id)
            if node.id in ("True", "False", "None"):
                return {"True": True, "False": False, "None": None}[node.id]
            raise Unparsed(f"name {node.id}")
        if isinstance(node, ast.JoinedStr):
            out = ""
            for v in node.values:
                if isinstance(v, ast.Constant):
                    out += v.value
                elif v.conversion == -1 and v.format_spec is None:
                    out += str(self.plain(self.expr(v.value)))
                else:
                    raise Unparsed("formatted f-string field")
            return out
        if isinstance(node, ast.BinOp):
            return self.binop(node.op, self.expr(node.left), self.expr(node.right))
        if isinstance(node, ast.UnaryOp):
            v = self.expr(node.operand)
            if isinstance(node.op, ast.Not):
                return not v
            if isinstance(node.op, ast.USub) and isinstance(v, int):
                return -v
            raise Unparsed("unary operator")
        if isinstance(node, ast.BoolOp):
            v = None
            for n in node.values:
                v = self.expr(n)
                if isinstance(node.op, ast.And) and not v or isinstance(node.op, ast.Or) and v:
                    return v
            return v
        if isinstance(node, ast.Compare):
            left = self.plain(self.expr(node.left))
            for op, n in zip(node.ops, node.comparators):
                right = self.plain(self.expr(n))
                ok = {ast.Eq: lambda a, b: a == b, ast.NotEq: lambda a, b: a != b, ast.In: lambda a, b: a in b,
                      ast.NotIn: lambda a, b: a not in b, ast.Lt: lambda a, b: a < b, ast.Gt: lambda a, b: a > b,
                      ast.LtE: lambda a, b: a <= b, ast.GtE: lambda a, b: a >= b,
                      ast.Is: lambda a, b: a is b, ast.IsNot: lambda a, b: a is not b}.get(type(op))
                if ok is None:
                    raise Unparsed("comparison")
                if not ok(left, right):
                    return False
                left = right
            return True
        if isinstance(node, ast.IfExp):
            return self.expr(node.body) if self.expr(node.test) else self.expr(node.orelse)
        if isinstance(node, (ast.List, ast.Tuple)):
            values = [self.expr(e) for e in node.elts]
            return values if isinstance(node, ast.List) else tuple(values)
        if isinstance(node, (ast.ListComp, ast.GeneratorExp)):
            if len(node.generators) != 1 or node.generators[0].is_async:
                raise Unparsed("comprehension")
            gen, out, saved = node.generators[0], [], dict(self.env)
            for value in list(self.iterable(self.expr(gen.iter))):
                self.assign(gen.target, value)
                if all(self.expr(c) for c in gen.ifs):
                    out.append(self.expr(node.elt))
            self.env = saved
            return out
        if isinstance(node, ast.Subscript):
            value = self.expr(node.value)
            if not isinstance(value, (str, list, tuple)):
                raise Unparsed("subscript")
            if isinstance(node.slice, ast.Slice):
                parts = [self.expr(p) if p is not None else None for p in (node.slice.lower, node.slice.upper, node.slice.step)]
                return value[slice(*parts)]
            try:
                return value[self.expr(node.slice)]
            except (IndexError, TypeError) as e:
                raise PyFail("index") from e
        if isinstance(node, ast.Attribute):
            return self.attribute(self.expr(node.value), node.attr)
        if isinstance(node, ast.Call):
            return self.call(node)
        raise Unparsed(type(node).__name__)

    def plain(self, v):
        if isinstance(v, PathV):
            return v.path
        if isinstance(v, tuple) and v and v[0] in ("module", "builtin", "method"):
            raise Unparsed("a function used as a value")
        return v

    def binop(self, op, a, b):
        if isinstance(op, ast.Div) and isinstance(a, PathV) and isinstance(b, str):
            return PathV(posixpath.join(a.path.replace("\\", "/"), b))
        if isinstance(op, ast.Add) and type(a) is type(b) and isinstance(a, (str, list, int)):
            return a + b
        if isinstance(op, ast.Sub) and isinstance(a, int) and isinstance(b, int):
            return a - b
        if isinstance(op, ast.Mult) and isinstance(a, str) and isinstance(b, int):
            return a * b
        raise Unparsed("operator")

    def attribute(self, value, attr):
        if isinstance(value, tuple) and value and value[0] == "module":
            name = f"{value[1]}.{attr}"
            if name in ("os.path", "os.path.join", "os.path.exists", "os.path.isfile", "io.open", "pathlib.Path",
                        "os.remove", "os.unlink", "sys.exit"):
                return ("module", name)
            raise Unparsed(name)
        if isinstance(value, PathV) and attr in ("name", "parent"):
            return posixpath.basename(value.path) if attr == "name" else PathV(posixpath.dirname(value.path))
        return ("method", value, attr)

    def call(self, node):
        fn = self.expr(node.func)
        args = [self.expr(a) for a in node.args]
        kwargs = {k.arg: self.expr(k.value) for k in node.keywords}
        if any(isinstance(a, ast.Starred) for a in node.args) or None in kwargs:
            raise Unparsed("starred call")
        if not isinstance(fn, tuple):
            raise Unparsed("call of a value")
        kind = fn[0]
        if kind == "builtin" or kind == "module":
            name = fn[1]
            if name == "print":
                return None
            if name == "len":
                return len(args[0])
            if name in ("str", "list", "sorted", "any", "all", "int"):
                return {"str": str, "list": list, "sorted": sorted, "any": any, "all": all, "int": int}[name](*map(self.plain, args))
            if name in ("open", "io.open"):
                mode = (args[1] if len(args) > 1 else kwargs.get("mode", "r"))
                return self.open(args[0], mode)
            if name in ("pathlib.Path", "pathlib.PurePath"):
                return PathV(posixpath.join(*[self.plain(a) for a in args]) if args else self.cwd)
            if name == "os.path.join":
                return posixpath.join(*[self.plain(a) for a in args])
            if name in ("os.path.exists", "os.path.isfile"):
                p = self.resolve(args[0])
                return self.store.exists(p) if self.store.is_memory(p) else self.unknown_outside()
            if name in ("os.remove", "os.unlink"):
                p = self.resolve(args[0])
                if self.store.is_memory(p):
                    self.store.delete(p)
                return None
            if name == "sys.exit":
                raise PyFail("exit") if args and args[0] not in (0, None) else Unparsed("sys.exit")
            raise Unparsed(f"call of {name}")
        _, obj, attr = fn
        if set(kwargs) - FREE:
            raise Unparsed(f"keyword {sorted(set(kwargs) - FREE)}")
        if isinstance(obj, PathV):
            if attr == "read_text":
                return self.read(obj)
            if attr == "write_text":
                return self.write(obj, args[0])
            if attr == "exists" or attr == "is_file":
                p = self.resolve(obj)
                return self.store.exists(p) if self.store.is_memory(p) else self.unknown_outside()
            if attr == "unlink":
                p = self.resolve(obj)
                if self.store.is_memory(p):
                    self.store.delete(p)
                return None
            if attr == "open":
                return self.open(obj, args[0] if args else kwargs.get("mode", "r"))
            raise Unparsed(f"Path.{attr}")
        if isinstance(obj, FileV):
            if attr == "read" and "r" in obj.mode:
                return self.read(obj.path)
            if attr == "write" and obj.mode[0] in "wa":
                obj.text += args[0]
                return self.write(obj.path, args[0], append=True) if obj.mode[0] == "a" else self.write(obj.path, obj.text)
            if attr in ("close", "flush"):
                return None
            raise Unparsed(f"file.{attr}")
        if isinstance(obj, str) and attr in STR_METHODS:
            try:
                return getattr(obj, attr)(*map(self.plain, args), **{k: v for k, v in kwargs.items() if k == "keepends"})
            except ValueError as e:
                raise PyFail(attr) from e
        if isinstance(obj, list) and attr in LIST_METHODS:
            try:
                return getattr(obj, attr)(*args)
            except (ValueError, IndexError) as e:
                raise PyFail(attr) from e
        raise Unparsed(f"method {attr}")

    def open(self, path, mode):
        if not isinstance(mode, str) or "b" in mode or "+" in mode or mode[0] not in "rwa":
            raise Unparsed(f"open mode {mode!r}")
        f = FileV(path, mode)
        if mode[0] == "w":
            self.write(path, "")
        return f

    def unknown_outside(self):
        raise Unparsed("a check on a file outside memory")
