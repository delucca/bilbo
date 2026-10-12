FROM ghcr.io/vectorize-io/hindsight-api:0.10.3@sha256:f80f304db313cbc8775acd601fda7d094f5854ebf9bc252d8134eb9276d56b40

# The Claude Code integration's hooks run on Node: the binary alone, from Node's official image.
COPY --from=node:24.21.0-trixie-slim@sha256:173f125896c3b47ddf056734c7ea789d04595a6a08769a8f78e0df642781fb66 /usr/local/bin/node /usr/local/bin/node
# hindsight-coding-agents 0.8.0 as npm publishes it. The registry pins it by an sha512 integrity
# (sha512-CViWmh2fnVjhQM8kqOJ2GIGC0kp/Gv4t5X+j+kzpzP+T54PiKQPsrQOunvmluEE3JCGnRQOIcz91Vu/0QUcZow==), which BuildKit cannot check; this is the same file's sha256.
# The image, Node and tarball pins here, and compose.yaml's pgvector, are repeated in build.py's manifest: change them together.
ADD --checksum=sha256:afc489206154f2e7254057e52b455be8751e8b8bbf5e4a720a104c136fbeed7c \
    https://registry.npmjs.org/@vectorize-io/hindsight-coding-agents/-/hindsight-coding-agents-0.8.0.tgz /opt/coding-agents.tgz
USER root
RUN mkdir -p /opt/coding-agents && tar -xzf /opt/coding-agents.tgz -C /opt/coding-agents --strip-components=1 \
 && rm /opt/coding-agents.tgz && node --version && node /opt/coding-agents/dist/claude-stop-hook.js </dev/null
USER hindsight

# The build runs as the host user, whose HOME is not the image's: HF_HOME points at the cross-encoder baked in the image.
ENV HOME=/tmp/live/home TMPDIR=/tmp/live/hooks PYTHONUNBUFFERED=1 HF_HUB_OFFLINE=1 HF_HOME=/home/hindsight/.cache/huggingface
COPY build.py /app/eval/
ENTRYPOINT ["python", "/app/eval/build.py"]
