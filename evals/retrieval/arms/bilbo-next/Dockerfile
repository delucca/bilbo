# Claude Code 2.1.327, the `claude` bilbo's guards were read from, for a build through the tool, checked
# against the sha256 in its GPG-signed manifest (https://downloads.claude.ai/claude-code-releases/2.1.327/manifest.json).
# One stage a machine, so a build fetches its own alone.
FROM scratch AS claude-amd64
ADD --checksum=sha256:343c744baa14bfd38286a7cb90871051f04379956da6c5b803a9e66019a76ac9 https://downloads.claude.ai/claude-code-releases/2.1.327/linux-x64/claude /claude
FROM scratch AS claude-arm64
ADD --checksum=sha256:acf0ef2f9da8d5ab08f193f007e19a06cbfb608b96a2f1f758d63a92a386cff8 https://downloads.claude.ai/claude-code-releases/2.1.327/linux-arm64/claude /claude
FROM claude-$TARGETARCH AS claude

FROM ghcr.io/astral-sh/uv:0.12.19-python3.12-trixie-slim@sha256:10b94fe3550b7a96e3982860bb82cdefba6300264a7a40bb9334acf21a450974

ENV UV_COMPILE_BYTECODE=1 UV_LINK_MODE=copy UV_PYTHON_DOWNLOADS=never UV_PROJECT_ENVIRONMENT=/app/.venv
WORKDIR /app
COPY pyproject.toml uv.lock ./
RUN uv sync --locked --no-install-project --no-cache

# Built by binary.sh from the checkout under test; its sha256 is in the store's key.
COPY bin/bilbo /usr/local/bin/bilbo
RUN bilbo --version
COPY --from=claude --chmod=755 /claude /usr/local/bin/claude
RUN HOME=/tmp DISABLE_AUTOUPDATER=1 claude --version

ENV PATH=/app/.venv/bin:$PATH HOME=/tmp PYTHONUNBUFFERED=1
COPY build.py cache.py tool.py ./
ENTRYPOINT ["python", "/app/build.py"]
