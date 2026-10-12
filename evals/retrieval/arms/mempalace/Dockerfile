FROM ghcr.io/astral-sh/uv:0.12.19-python3.12-trixie-slim@sha256:10b94fe3550b7a96e3982860bb82cdefba6300264a7a40bb9334acf21a450974

ENV UV_COMPILE_BYTECODE=1 UV_LINK_MODE=copy UV_PYTHON_DOWNLOADS=never UV_PROJECT_ENVIRONMENT=/app/.venv
WORKDIR /app
COPY pyproject.toml uv.lock ./
RUN uv sync --locked --no-install-project --no-cache

# libfaketime, Debian trixie's package (0.9.10+2024-06-05+gba9ed5b2-0.6) from the archive snapshot that holds it, by
# sha256. Both arches are fetched so one Dockerfile builds on x86_64 and arm64; the one for this machine is installed.
# build.py preloads it into each hook so MemPalace files every drawer at the time its event happened.
ARG FAKETIME=https://snapshot.debian.org/archive/debian/20250127T205014Z/pool/main/f/faketime
ADD --checksum=sha256:4ecb98d7f6149e4c297a5946c4fa231a1b794d203053962e295dd3a18a47f269 $FAKETIME/libfaketime_0.9.10%2B2024-06-05%2Bgba9ed5b2-0.6_amd64.deb /opt/faketime/x86_64.deb
ADD --checksum=sha256:79750b97bc7d0a35796a248ed65d7ff01aaec12ba869e39e19d56bf3374a0abc $FAKETIME/libfaketime_0.9.10%2B2024-06-05%2Bgba9ed5b2-0.6_arm64.deb /opt/faketime/aarch64.deb
RUN dpkg -i /opt/faketime/$(uname -m).deb && rm -rf /opt/faketime && test -f /usr/lib/$(uname -m)-linux-gnu/faketime/libfaketime.so.1

# chromadb's telemetry is on by default; MEMPALACE_PYTHON is the interpreter the hook spawns its detached mine with
# (mempalace/hooks_cli.py:100-113).
ENV PATH=/app/.venv/bin:$PATH HOME=/tmp/home PYTHONUNBUFFERED=1 ANONYMIZED_TELEMETRY=False MEMPALACE_PYTHON=/app/.venv/bin/python
COPY build.py sweep.py ./
ENTRYPOINT ["python", "/app/build.py"]
