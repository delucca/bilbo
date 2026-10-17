---
license: odc-by
pretty_name: bilbo retrieval eval
viewer: false
---

# bilbo retrieval eval

The data of [bilbo](https://github.com/delucca/bilbo)'s retrieval eval: which memory strategy best remembers what
happened in real coding sessions? The code, and how each file here was made, is in
[evals/retrieval](https://github.com/delucca/bilbo/tree/next/evals/retrieval).

- `corpus-<sha12>.tar.zst`: 291 real coding sessions from seventeen public projects, untouched, one file per session.
  `<sha12>` starts the archive's sha256. The first five projects were chosen by metadata, the other twelve by a
  screen for procedure.
- `store-<arm>-<key>.tar.zst`: the memory one strategy built from the corpus. `<key>` hashes what shapes it: the
  corpus, the embedder, the LLM for an arm that uses one, and the arm's files.

The eval's scripts download each file at the commit recorded in the eval's `dataset.lock` and check its sha256 against
the lock, so a run always gets the bytes it was pinned to.

## Credits

The sessions come from [SWE-chat](https://huggingface.co/datasets/SALT-NLP/SWE-chat) (Baumann et al., *SWE-chat:
Real-World AI Coding Sessions in the Wild*, COLM 2026), released under ODC-BY 1.0.
