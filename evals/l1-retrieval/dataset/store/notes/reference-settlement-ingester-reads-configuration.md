---
id: 01KJJP5HD79B1RCMH9SKD4H16N
created: 2026-02-28T14:53-03:00
sources:
  - "doc: Ingester runbook"
---

# settlement-ingester reference

settlement-ingester reads its configuration from the file `/etc/ledgerlark/ingester.toml` and takes the SFTP host from the environment variable `LARK_SFTP_HOST`. So: config file path is `/etc/ledgerlark/ingester.toml`; SFTP host is not in that file, it comes from `LARK_SFTP_HOST`. If the host looks wrong, check the environment of the process first, then the file.

This note is a quick reference for the Ledgerlark component that pulls card-processor settlement files in. Ledgerlark reconciles those files against internal ledger entries and flags mismatches for review, and settlement-ingester is the first step of that path.

## Naming

The component used to be called `fileslurp`. It is called `settlement-ingester` now. Old branches, dashboards, runbooks, Terraform variable names and chat history may still say `fileslurp`. They mean the same thing as `settlement-ingester`. When searching logs or repos, try both names. Use `settlement-ingester` in anything new.

## Configuration

- Config file: `/etc/ledgerlark/ingester.toml`. This is where the settings other than the SFTP host live.
- SFTP host: environment variable `LARK_SFTP_HOST`. The service picks it up from the environment of the process, so it has to be set wherever the service is deployed (container spec, unit file, or whatever Terraform renders).
- Keep the two sources separate in your head. Editing the toml file will not change the SFTP host, and setting `LARK_SFTP_HOST` will not change anything else in the file.

## What it does

It connects to the SFTP server of the card processor, fetches new settlement files, parses them and hands the records on to the rest of Ledgerlark. The handoff goes through Apache Kafka, and the downstream matching works against PostgreSQL ledger data. It is written in Go, like most of the project. I did not re-check the exact topic names or schemas while writing this, so look in the code before relying on them.

## Gotchas and checks

- If the service cannot connect, look at `LARK_SFTP_HOST` before anything else. A missing or stale value is the most likely cause.
- If it starts but behaves oddly, confirm that `/etc/ledgerlark/ingester.toml` is the file actually mounted in that environment and not a leftover copy.
- Do not hardcode the host in the toml file expecting it to win; the host is meant to come from `LARK_SFTP_HOST`.
- When reading old incident notes that mention `fileslurp`, translate to `settlement-ingester` before drawing conclusions about current behaviour.

## Open items

- Confirm which deployment sets `LARK_SFTP_HOST` and where that value is managed.
- Add retry and failure behaviour here once someone has verified it in the code.
