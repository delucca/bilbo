"""The command tree: help works, an unknown command is a usage error, a missing module says so."""

import subprocess
import sys

import pytest

from bilbo_evals import cli
from bilbo_evals.common import Refused


def run(*args):
    return subprocess.run([sys.executable, "-m", "bilbo_evals.cli", *args], capture_output=True, text=True)


def test_help_exits_zero():
    p = run("--help")
    assert p.returncode == 0 and "l1" in p.stdout


@pytest.mark.parametrize("words", [w for w, *_ in cli.COMMANDS], ids=lambda w: " ".join(w))
def test_every_command_has_help(words):
    p = run(*words, "--help")
    assert p.returncode == 0 and p.stdout.startswith("usage:")


def test_unknown_command_is_a_usage_error_with_empty_stdout():
    p = run("frobnicate")
    assert p.returncode == 2 and p.stdout == ""
    assert p.stderr.count("bilbo-evals: ") == 1


def test_missing_required_option_is_a_usage_error():
    p = run("l1", "run")
    assert p.returncode == 2 and p.stdout == ""


def test_module_not_written_yet_is_refused(capsys):
    args = cli.build().parse_args(["dataset", "check"])
    args.handler = "nosuchmodule:cmd"
    with pytest.raises(Refused, match="dataset check is not implemented yet"):
        cli.dispatch(args)


def test_hidden_embedder_url_is_accepted_but_not_listed():
    assert "--embedder-url" not in run("l1", "run", "--help").stdout
    args = cli.build().parse_args(["parity", "--bilbo", "b", "--embedder-url", "http://x"])
    assert args.embedder_url == "http://x"


def test_dataset_default_is_the_notes_synth_folder():
    args = cli.build().parse_args(["dataset", "check"])
    assert str(args.dataset).endswith("datasets/notes-synth/v1") and args.split == "all"
