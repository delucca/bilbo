FROM ghcr.io/astral-sh/uv:0.12.19-python3.12-trixie-slim@sha256:10b94fe3550b7a96e3982860bb82cdefba6300264a7a40bb9334acf21a450974

# ai-memory 2.6.3's Linux release binaries, checked against the sha256 the release publishes. Both are fetched so one
# Dockerfile builds on x86_64 and arm64; the one for this machine is kept. SQLite and libgit2 are linked in.
ARG RELEASE=https://github.com/akitaonrails/ai-memory/releases/download/v2.6.3
ADD --checksum=sha256:933be9bb8fe6930336933f0b8adf0695007e644f945898f9f030fd312326f0f3 $RELEASE/ai-memory-linux-x86_64.tar.gz /opt/ai-memory/x86_64.tar.gz
ADD --checksum=sha256:e4e01409fe80148a4d561aef98ac5d526c488c4a621faa759f9d0b3ed2606088 $RELEASE/ai-memory-linux-aarch64.tar.gz /opt/ai-memory/aarch64.tar.gz
RUN tar -xzf /opt/ai-memory/$(uname -m).tar.gz -C /usr/local/bin ./ai-memory \
 && chmod 755 /usr/local/bin/ai-memory && rm -rf /opt/ai-memory && ai-memory --version

ENV HOME=/tmp/home PYTHONUNBUFFERED=1
COPY build.py /app/
ENTRYPOINT ["python", "/app/build.py"]
