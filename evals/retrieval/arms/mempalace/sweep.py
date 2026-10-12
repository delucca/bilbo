"""`mempalace sweep <transcript>` with the embedding client's timeout raised for this run only. MemPalace waits 120 s
for each embeddings request and does not retry (mempalace/embedding.py:589, 667, read as a module global at each call);
a request of 64 sweep messages, several of them cut at the embedder's whole context, takes longer on a CPU embedder.
The command is MemPalace's own entry point (`mempalace.cli:main`) with the same arguments."""

import sys

import mempalace.embedding
from mempalace.cli import main

mempalace.embedding._EF_API_TIMEOUT = 3600
sys.argv = ["mempalace", "sweep", *sys.argv[1:]]
main()
