# retrieval

Which memory strategy best remembers what happened in real coding sessions?

## Prerequisites

- [Nix](https://nixos.org/download) with flakes enabled. Every script runs in the eval's pinned shell, so there is
  nothing else to install.
- A [Hugging Face token](https://huggingface.co/settings/tokens), only to rebuild the corpus. Its account must accept
  [SWE-chat](https://huggingface.co/datasets/SALT-NLP/SWE-chat)'s terms.

## The shell

`flake.nix` and `flake.lock` pin the tools every script uses. From the repository root, open the shell with:

    nix develop ./evals/retrieval

or run one command in it:

    nix develop ./evals/retrieval --command evals/retrieval/scripts/fetch-corpus.sh

A script started outside the shell enters it on its own.

## The corpus

`dataset/corpus-1235aec0592a.tar.zst` holds 145 real Claude Code sessions from five open-source projects, untouched:
one file per session, grouped by project, sorted by time.

To check it, or rebuild it from SWE-chat:

    HF_TOKEN=hf_... evals/retrieval/scripts/fetch-corpus.sh

The script downloads each session at a pinned dataset revision and packs them the same way every time. Before it
finishes, it checks the archive against the sha256 pinned in the script; the file name starts with that hash. If the
archive is already there and valid, it says so and exits, with no token needed.

## How it works

1. Each strategy reads a project's sessions in order, one turn at a time, the way the tool is normally fed, and builds
   its own memory.
2. Each project has questions whose answers are somewhere in its sessions.
3. Each strategy searches its memory for every question, and its answers are scored against the answer key, along with
   what the strategy cost.

## Credits

The sessions come from SWE-chat (Baumann et al., *SWE-chat: Real-World AI Coding Sessions in the Wild*, COLM 2026),
released under ODC-BY 1.0.
