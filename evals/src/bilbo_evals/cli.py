"""The bilbo-evals command tree, with each handler imported on first use."""

from __future__ import annotations

import argparse
import importlib
import sys
from pathlib import Path

from bilbo_evals.common import DEFAULT_DATASET, Refused, UsageError, err

SPLIT = ("--split", {"choices": ["dev", "test"]})
SPLIT_REQ = ("--split", {"choices": ["dev", "test"], "required": True})
SPLIT_ALL = ("--split", {"choices": ["dev", "test", "all"], "default": "all"})
DATASET = ("--dataset", {"type": Path, "default": DEFAULT_DATASET, "help": "dataset folder"})
BILBO = ("--bilbo", {"type": Path, "help": "bilbo binary"})
BILBO_REQ = ("--bilbo", {"type": Path, "required": True, "help": "bilbo binary"})
MODEL = ("--model", {"type": Path, "help": "embedding GGUF (default: the pinned model in bilbo's cache)"})
LLAMA = ("--llama-server", {"default": "llama-server", "help": "llama-server binary"})
EMBEDDER_URL = ("--embedder-url", {"help": argparse.SUPPRESS})
DRAFT = ("--draft", {"action": "store_true", "help": "run on a draft dataset"})

# (command words, handler, help, options, positionals)
COMMANDS: list[tuple[tuple[str, ...], str, str, list, list]] = [
    (("dataset", "check"), "dataset:cmd_check", "check a dataset against the rules", [DATASET, SPLIT_ALL], []),
    (("dataset", "freeze"), "dataset:cmd_freeze", "freeze a dataset: MANIFEST and FROZEN", [],
     [("dir", {"nargs": "?", "type": Path, "default": DEFAULT_DATASET})]),
    (("dataset", "verify"), "dataset:cmd_verify", "verify a dataset against its MANIFEST", [],
     [("dir", {"nargs": "?", "type": Path, "default": DEFAULT_DATASET})]),
    (("generate", "profile"), "generate.profile:cmd", "write the rounded profile of a store",
     [("--store", {"type": Path, "required": True, "help": "store root to profile"}), DATASET], []),
    (("generate", "world"), "generate.world:cmd", "generate projects, components and aliases", [DATASET], []),
    (("generate", "facts"), "generate.facts:cmd", "generate facts", [DATASET], []),
    (("generate", "notes"), "generate.notes:cmd", "render notes", [DATASET], []),
    (("generate", "fidelity"), "generate.fidelity:cmd", "check rendered notes against their facts", [DATASET], []),
    (("generate", "library"), "generate.library:cmd", "stage and land the library sources", [DATASET, BILBO], []),
    (("generate", "queries"), "generate.queries:cmd", "generate queries", [DATASET, SPLIT_REQ], []),
    (("generate", "prompts"), "generate.prompts:cmd", "generate digest prompts", [DATASET, SPLIT_REQ], []),
    (("generate", "filter"), "generate.filter:cmd", "reject leaking queries, mark zero overlap", [DATASET, SPLIT], []),
    (("pool",), "pool:cmd", "pool candidates and judge them",
     [DATASET, BILBO, MODEL, LLAMA, SPLIT_ALL,
      ("--apply", {"action": "store_true", "help": "apply the reviewer's resolutions"}), EMBEDDER_URL], []),
    (("review", "sample"), "review:cmd_sample", "draw the review sample", [DATASET, SPLIT_ALL], []),
    (("review", "check"), "review:cmd_check", "check the review sheet", [DATASET, SPLIT_ALL], []),
    (("review", "apply"), "review:cmd_apply", "apply the review sheet to the dataset", [DATASET], []),
    (("power",), "stats:cmd_power", "size the test split from a dev run", [DATASET],
     [("run", {"metavar": "RUN", "help": "dev run folder"})]),
    (("parity",), "embedder:cmd_parity", "check bilbo embeds what the harness predicts",
     [BILBO_REQ, DATASET, MODEL, LLAMA, EMBEDDER_URL], []),
    (("l1", "run"), "runner:cmd_run", "run the retrieval arms on a split",
     [SPLIT_REQ, ("--arms", {"default": "all", "help": "`all` or a comma list"}), BILBO_REQ, DRAFT, DATASET, MODEL, LLAMA,
      ("--run-id", {}), ("--keep-root", {"action": "store_true", "help": "keep the sandbox root"}), EMBEDDER_URL], []),
    (("l1", "digest"), "digest:cmd", "run the digest hook on a split",
     [SPLIT_REQ, BILBO_REQ, ("--sweep", {"action": "store_true", "help": "sweep the thresholds"}), DRAFT, DATASET, MODEL,
      LLAMA, ("--run", {"type": Path, "metavar": "RUN_DIR", "help": "add to this run instead of a new one"}),
      EMBEDDER_URL], []),
    (("report",), "results:cmd_report", "print the report of a run", [],
     [("run", {"metavar": "RUN"})]),
    (("compare",), "results:cmd_compare", "compare a run with a baseline", [],
     [("baseline", {"metavar": "BASELINE"}), ("run", {"metavar": "RUN"})]),
]


class _Parser(argparse.ArgumentParser):
    def error(self, message: str):
        self.print_usage(sys.stderr)
        err(message)
        raise SystemExit(2)


def build() -> argparse.ArgumentParser:
    root = _Parser(prog="bilbo-evals", description="Retrieval evals for bilbo.")
    top = root.add_subparsers(dest="command", metavar="COMMAND", required=True, parser_class=_Parser)
    groups: dict[str, argparse._SubParsersAction] = {}
    for words, handler, help_, options, positionals in COMMANDS:
        if len(words) == 1:
            p = top.add_parser(words[0], help=help_, description=help_)
        else:
            if words[0] not in groups:
                g = top.add_parser(words[0], help=f"{words[0]} commands")
                groups[words[0]] = g.add_subparsers(dest="subcommand", metavar="COMMAND", required=True, parser_class=_Parser)
            p = groups[words[0]].add_parser(words[1], help=help_, description=help_)
        for name, kw in positionals:
            p.add_argument(name, **kw)
        for name, kw in options:
            p.add_argument(name, **kw)
        p.set_defaults(handler=handler, words=" ".join(words))
    return root


def _fill_defaults(args: argparse.Namespace) -> None:
    if hasattr(args, "model") and args.model is None:
        try:
            from bilbo_evals import embedder
            args.model = embedder.default_gguf()
        except (ImportError, AttributeError):
            pass


def dispatch(args: argparse.Namespace) -> int:
    module, func = args.handler.split(":")
    full = f"bilbo_evals.{module}"
    try:
        fn = getattr(importlib.import_module(full), func)
    except ModuleNotFoundError as e:
        if e.name and (full == e.name or full.startswith(e.name + ".")):
            raise Refused(f"{args.words} is not implemented yet") from e
        raise
    except AttributeError as e:
        raise Refused(f"{args.words} is not implemented yet") from e
    _fill_defaults(args)
    return fn(args)


def main(argv: list[str] | None = None) -> int:
    try:
        args = build().parse_args(argv)
    except SystemExit as e:
        return e.code if isinstance(e.code, int) else 0
    try:
        return dispatch(args)
    except Refused as e:
        err(str(e))
        return 1
    except UsageError as e:
        err(str(e))
        return 2


if __name__ == "__main__":
    sys.exit(main())
