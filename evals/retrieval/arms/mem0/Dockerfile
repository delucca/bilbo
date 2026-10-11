FROM ghcr.io/astral-sh/uv:0.12.19-python3.12-trixie-slim@sha256:10b94fe3550b7a96e3982860bb82cdefba6300264a7a40bb9334acf21a450974

ENV UV_COMPILE_BYTECODE=1 UV_LINK_MODE=copy UV_PYTHON_DOWNLOADS=never UV_PROJECT_ENVIRONMENT=/app/.venv
WORKDIR /app
COPY pyproject.toml uv.lock ./
RUN uv sync --locked --no-install-project --no-cache

# mem0's BM25 encoder fetches Qdrant/bm25 from Hugging Face on first use: fetch it here and refuse any other revision.
# Offline, fastembed accepts the cached files only if every file it lists for the model is there, and the repo lacks
# two: the mock.file placeholder (qdrant/fastembed#675) and tamil.txt. Bm25 reads only English stopwords, so both stay empty.
ENV PATH=/app/.venv/bin:$PATH FASTEMBED_CACHE_PATH=/opt/fastembed
RUN python -c 'from fastembed import SparseTextEmbedding; SparseTextEmbedding("Qdrant/bm25")' \
 && test "$(cat /opt/fastembed/models--Qdrant--bm25/refs/main)" = 22b8d2af71a76161e18dd432d2cee0eefa66e412 \
 && cd /opt/fastembed/models--Qdrant--bm25/snapshots/22b8d2af71a76161e18dd432d2cee0eefa66e412 && touch mock.file tamil.txt && cd /app \
 && HF_HUB_OFFLINE=1 python -c 'from fastembed import SparseTextEmbedding; SparseTextEmbedding("Qdrant/bm25")' \
 && chmod -R a+rwX /opt/fastembed

# mem0's Claude Code plugin 0.3.3, from the commit of mem0's v2.2.1 tag: build.py feeds the sessions through its code.
ARG PLUGIN=https://raw.githubusercontent.com/mem0ai/mem0/94c3fe9f238f3dbf29c9ce98643bd71eb13077cd/integrations/claude-code-plugin
ADD --checksum=sha256:35700da6ffd37fb5008e5ad326be69adf0991a36254be8104c4473bed76ab919 $PLUGIN/core/memory_core.py /app/plugin/
ADD --checksum=sha256:853f1ca2c81f7e742e186f86b8ee63663f1f783373b9189ee94ef29ba7560a09 $PLUGIN/core/telemetry.py /app/plugin/
ADD --checksum=sha256:27c7fd41a194f36129c67e96b826c29bfc4d23baa4b8598ae26e509bc9bea144 $PLUGIN/core/_harness_id.py /app/plugin/
ADD --checksum=sha256:34a60c572b55fc53f36532746f9189b33dc01919bf32a45df429ba714bc4bf77 $PLUGIN/adapters/claude/transcript.py /app/plugin/
RUN chmod a+r /app/plugin/*

ENV HF_HUB_OFFLINE=1 HF_HUB_DISABLE_TELEMETRY=1 MEM0_TELEMETRY=False MEM0_DIR=/tmp/mem0 HOME=/tmp PYTHONUNBUFFERED=1
COPY build.py ./
ENTRYPOINT ["python", "/app/build.py"]
