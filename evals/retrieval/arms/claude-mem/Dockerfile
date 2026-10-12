FROM ghcr.io/astral-sh/uv:0.12.19-python3.12-trixie-slim@sha256:10b94fe3550b7a96e3982860bb82cdefba6300264a7a40bb9334acf21a450974

# git, which claude-mem runs to name a project, and procps, whose ps its supervisor reads, from Debian as it was on the
# day claude-mem 13.35.0 was published.
RUN printf '%s\n' 'Types: deb' 'URIs: http://snapshot.debian.org/archive/debian/20261009T000000Z' \
      'Suites: trixie trixie-updates' 'Components: main' 'Signed-By: /usr/share/keyrings/debian-archive-keyring.gpg' '' \
      'Types: deb' 'URIs: http://snapshot.debian.org/archive/debian-security/20261009T000000Z' 'Suites: trixie-security' \
      'Components: main' 'Signed-By: /usr/share/keyrings/debian-archive-keyring.gpg' > /etc/apt/sources.list.d/debian.sources \
 && apt-get -o Acquire::Check-Valid-Until=false update \
 && apt-get install -y --no-install-recommends git=1:2.47.3-0+deb13u1 procps=2:4.0.4-9 \
 && rm -rf /var/lib/apt/lists/*

# Bun 1.4.2, the release that was current when claude-mem 13.35.0 was published (its release workflow installs the
# latest Bun). Both archives are fetched so one Dockerfile builds on x86_64 and arm64; the one for this machine is kept.
ARG BUN=https://github.com/oven-sh/bun/releases/download/bun-v1.4.2
ADD --checksum=sha256:36368faef7527875d5ffa52e53cd48021741f2a83eb6208a8dd64068d422a913 $BUN/bun-linux-x64.zip /opt/bun/x86_64.zip
ADD --checksum=sha256:54328bbc2d9c8e0c9f892c544d66c57a83b84139e34909e5ee81758f1ac8fda7 $BUN/bun-linux-aarch64.zip /opt/bun/aarch64.zip
RUN python -c 'import platform, zipfile; m = platform.machine(); d = {"x86_64": "bun-linux-x64", "aarch64": "bun-linux-aarch64"}[m]; open("/usr/local/bin/bun", "wb").write(zipfile.ZipFile(f"/opt/bun/{m}.zip").read(f"{d}/bun"))' \
 && chmod 755 /usr/local/bin/bun && rm -rf /opt/bun && bun --version

# claude-mem 13.35.0 from npm, checked against its sha256. Its plugin/ is what Claude Code installs; Bun installs the
# plugin's dependencies from its lockfile, as claude-mem's installer does.
ADD --checksum=sha256:a5d7b646541e1a839572095a31682b0e121929c1b3c6e80e29b27eb0a702a67a https://registry.npmjs.org/claude-mem/-/claude-mem-13.35.0.tgz /opt/claude-mem.tgz
RUN mkdir /opt/claude-mem && tar -xzf /opt/claude-mem.tgz -C /opt/claude-mem --strip-components=1 && rm /opt/claude-mem.tgz \
 && cd /opt/claude-mem/plugin && bun install --frozen-lockfile --production --ignore-scripts && chmod -R a+rX /opt/claude-mem

# chroma-mcp as claude-mem launches it, on Python 3.13.15, resolved with what PyPI held when claude-mem 13.35.0 was
# published and kept in uv's cache, which the build user may write; the build runs it offline.
ENV UV_CACHE_DIR=/opt/uv/cache UV_PYTHON_INSTALL_DIR=/opt/uv/python UV_EXCLUDE_NEWER=2026-10-09T03:44:44Z
RUN uv python install 3.13.15 \
 && uvx --python 3.13 --with 'onnxruntime>=1.20' --with 'protobuf<7' --with chromadb==1.5.9 --with 'pydantic<2.14' \
      --from chroma-mcp==0.2.6 chroma-mcp --help > /dev/null \
 && chmod -R a+rwX /opt/uv
ENV UV_OFFLINE=1 UV_PYTHON_DOWNLOADS=never HOME=/tmp/home PYTHONUNBUFFERED=1

COPY build.py clock.js /app/
ENTRYPOINT ["python", "/app/build.py"]
