FROM ghcr.io/astral-sh/uv:0.12.19-python3.12-trixie-slim@sha256:10b94fe3550b7a96e3982860bb82cdefba6300264a7a40bb9334acf21a450974

ENV HOME=/tmp PYTHONUNBUFFERED=1
COPY build.py /app/
ENTRYPOINT ["python", "/app/build.py"]
