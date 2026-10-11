FROM ghcr.io/astral-sh/uv:0.12.19-python3.12-trixie-slim@sha256:10b94fe3550b7a96e3982860bb82cdefba6300264a7a40bb9334acf21a450974

ENV UV_COMPILE_BYTECODE=1 UV_LINK_MODE=copy UV_PYTHON_DOWNLOADS=never UV_PROJECT_ENVIRONMENT=/app/.venv
WORKDIR /app
COPY pyproject.toml uv.lock ./
RUN uv sync --locked --no-install-project --no-cache

# The entity types Graphiti's MCP server ships for coding agents, from the commit of graphiti's v0.30.2 tag.
ARG MCP=https://raw.githubusercontent.com/getzep/graphiti/eaa4128681bc53487138a4bbc22d58336ebe70d2/mcp_server/src/models
ADD --checksum=sha256:ae289c09a76aa46a5192f83edf7d208a480483971102d7eb53579b06291d5736 $MCP/entity_types.py /app/mcp/
RUN chmod a+r /app/mcp/*

# Telemetry would post to PostHog when Graphiti starts. SEMAPHORE_LIMIT, read when graphiti_core is imported, is the
# MCP server's default (mcp_server/src/graphiti_mcp_server.py:94).
ENV PATH=/app/.venv/bin:$PATH GRAPHITI_TELEMETRY_ENABLED=false SEMAPHORE_LIMIT=10 HOME=/tmp PYTHONUNBUFFERED=1
COPY episodes.py build.py ./
ENTRYPOINT ["python", "/app/build.py"]
