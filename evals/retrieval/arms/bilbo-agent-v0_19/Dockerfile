FROM ghcr.io/astral-sh/uv:0.12.19-python3.12-trixie-slim@sha256:10b94fe3550b7a96e3982860bb82cdefba6300264a7a40bb9334acf21a450974

ENV UV_COMPILE_BYTECODE=1 UV_LINK_MODE=copy UV_PYTHON_DOWNLOADS=never UV_PROJECT_ENVIRONMENT=/app/.venv
WORKDIR /app
COPY pyproject.toml uv.lock ./
RUN uv sync --locked --no-install-project --no-cache

# bilbo 0.19.0's Linux release binaries, checked against the sha256 the release publishes. Both are fetched so one
# Dockerfile builds on x86_64 and arm64; the one for this machine is kept and checked again once unpacked.
ARG RELEASE=https://github.com/delucca/bilbo/releases/download/v0.19.0
ADD --checksum=sha256:ea3a6e210b062034d200e4589d10d023b722a189b71898ccb40f56184e5e5921 $RELEASE/bilbo-x86_64-unknown-linux-gnu.tar.xz /opt/bilbo/x86_64.tar.xz
ADD --checksum=sha256:0b0c80addc8ad6c9ebc76e9000a910c43bd87d790936775633036bcc4051fafe $RELEASE/bilbo-aarch64-unknown-linux-gnu.tar.xz /opt/bilbo/aarch64.tar.xz
RUN m=$(uname -m) \
 && python -c "import tarfile,sys; tarfile.open(f'/opt/bilbo/{sys.argv[1]}.tar.xz').extractall('/opt/bilbo', filter='data')" "$m" \
 && install -m 755 "/opt/bilbo/bilbo-$m-unknown-linux-gnu/bilbo" /usr/local/bin/bilbo && rm -rf /opt/bilbo \
 && case $m in x86_64) s=52e4eb13b5213f994e56400af4a8e0c7ff6b12b3d9787df0d2c5507e3228c31e ;; \
                aarch64) s=00c0677058d461081e3b822194b4f24c1c35f7de118b7f0c2d56655af9436fb0 ;; esac \
 && echo "$s  /usr/local/bin/bilbo" | sha256sum -c - && bilbo --version | grep -qx 'bilbo 0.19.0'

# The note skill as v0.19.0 ships it (the tag's commit); writer.py pins the same sha256.
ADD --checksum=sha256:4c4a1ac0670be19b9e71a7fe7da7338e698c242ccd372b5c225056766978b601 https://raw.githubusercontent.com/delucca/bilbo/f6fb15338ce9a1134d6f413eafc482d28d2badcc/plugins/bilbo/skills/note/SKILL.md /app/skill/SKILL.md

# tiktoken's o200k_base, under the file name tiktoken caches it by (sha1 of its URL); the load must not reach the network.
ADD --checksum=sha256:446a9538cb6c348e3516120d7c08b09f57c36495e2acfffe59a5bf8b0cfb1a2d https://openaipublic.blob.core.windows.net/encodings/o200k_base.tiktoken /opt/tiktoken/fb374d419588a4632f3f557e76b4b70aebbca790
ENV PATH=/app/.venv/bin:$PATH TIKTOKEN_CACHE_DIR=/opt/tiktoken HOME=/tmp PYTHONUNBUFFERED=1 PYTHONDONTWRITEBYTECODE=1
RUN chmod -R a+rX /opt/tiktoken /app/skill \
 && HTTPS_PROXY=http://127.0.0.1:9 HTTP_PROXY=http://127.0.0.1:9 python -c 'import tiktoken; tiktoken.get_encoding("o200k_base")'

COPY l3/ /app/l3/
COPY build.py ./
ENTRYPOINT ["python", "/app/build.py"]
