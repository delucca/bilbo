FROM ghcr.io/astral-sh/uv:0.12.19-python3.12-trixie-slim@sha256:10b94fe3550b7a96e3982860bb82cdefba6300264a7a40bb9334acf21a450974 AS fetch

# Claude Code 2.1.296's native Linux binaries from the release bucket, checked against the sha256 in its GPG-signed
# manifest (https://downloads.claude.ai/claude-code-releases/2.1.296/manifest.json). Both are fetched so one Dockerfile
# builds on x86_64 and arm64; the final stage keeps the one for this machine.
ARG RELEASE=https://downloads.claude.ai/claude-code-releases/2.1.296
ADD --checksum=sha256:24972e3bc859fab2b46ed4c1e51f7d6130f06d3bd550811a114640de3370d0de $RELEASE/linux-x64/claude /opt/claude/amd64
ADD --checksum=sha256:f1f6e96e0d8342b9dbf41d7e88255397a6a52ce3d8736ad6a4c6b59c9b62fefa $RELEASE/linux-arm64/claude /opt/claude/arm64

FROM ghcr.io/astral-sh/uv:0.12.19-python3.12-trixie-slim@sha256:10b94fe3550b7a96e3982860bb82cdefba6300264a7a40bb9334acf21a450974
ARG TARGETARCH
COPY --from=fetch --chmod=755 /opt/claude/$TARGETARCH /usr/local/bin/claude
RUN HOME=/tmp DISABLE_AUTOUPDATER=1 claude --version

ENV HOME=/tmp/home PYTHONUNBUFFERED=1
COPY build.py /app/
ENTRYPOINT ["python", "/app/build.py"]
