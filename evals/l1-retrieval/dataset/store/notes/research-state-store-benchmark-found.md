---
id: 01KGY5YBF2103MQWXFM4NAM71W
created: 2026-02-08T05:29-03:00
---

# state-store: WAL mode write throughput benchmark

A benchmark of the state-store found that turning on WAL mode raised its write throughput from 1,800 to 7,200 inserts per second. That is a fourfold gain on the insert path, from a journal setting alone. This note records what was measured, what it does and does not tell us, and what to check before relying on it.

## Result

The state-store is the SQLite-backed store PatchPilot uses to keep upgrade run state. With WAL mode the benchmark reached 7,200 inserts per second. Without it, the same workload reached 1,800 inserts per second.

```
before: 1,800 inserts/sec
after (WAL): 7,200 inserts/sec
```

## Why it matters

PatchPilot opens pull requests across many repositories and runs targeted tests for each. Every step writes a row: a candidate upgrade, a test result, a status change. When many repositories are processed at once, the state-store write rate is a real limit. A fourfold gain on inserts leaves a lot of room before the store slows the pipeline.

## What the benchmark measured

The benchmark measured inserts into the state-store, counted per second. It compared the default journal behavior against WAL mode. It says nothing directly about reads, update-heavy workloads, or database size on disk.

## What the benchmark did not measure

- Read latency under concurrent writers.
- Update and delete rates.
- Behavior on network-mounted storage.
- Behavior inside a container with a mounted volume.

Treat the gain as established for inserts only. Anything else needs its own measurement.

## Why WAL helps writes

In the default rollback-journal mode, each commit has to force changes to the main database file and sync more than once. In WAL mode, commits append to a separate log and sync far less often. The main file is updated later in batches. That cuts the per-commit cost, which is what a high insert rate pays for most.

## Concurrency effect

WAL also lets readers keep reading while a writer commits. In the default mode, readers and the writer block each other. PatchPilot's reporting and status checks read the state-store while workers write to it, so this is a second reason to prefer WAL, even though the benchmark only counted inserts.

## Docker considerations

PatchPilot ships in Docker. WAL mode creates extra files next to the database file, and it relies on shared memory between connections in the same host. The database file and its companions must sit on the same local filesystem. Do not split them across mounts. A bind mount from a remote or unusual filesystem may not behave as expected, so check this before assuming the gain carries over.

## GitHub Actions considerations

When the state-store runs on a GitHub Actions runner, the database usually lives on the runner's local disk and is short-lived. The benchmark gain should hold there. If a job caches or uploads the database as an artifact, the log file must be folded back into the main file first, or the artifact may miss recent writes.

## Durability tradeoff

WAL can be paired with a relaxed sync setting, which raises throughput further but risks losing the latest commits on a crash. The benchmark result should not be read as using that setting. Do not loosen durability to chase a bigger number. Upgrade run state is cheap to recompute, but losing it silently would produce confusing duplicate pull requests.

## Checkpointing

The log file grows until a checkpoint moves its content into the main database. If a long-running reader holds a snapshot open, checkpoints cannot finish and the log keeps growing. Watch for this in any long-lived PatchPilot process that holds read transactions open.

## Backup and copying

Copying only the main database file while WAL is active can give an incomplete copy. Use SQLite's own backup mechanism, or checkpoint first and then copy. This matters for anyone who snapshots the state-store for debugging.

## Risks

- A deployment that mounts the database on an unsupported filesystem may fail to enable WAL or may corrupt state.
- Tools that open the database file read-only without access to the companion files can see stale data.
- Extra files appearing beside the database can surprise cleanup scripts.

## Recommendation

Keep WAL mode on for the state-store. The insert gain from 1,800 to 7,200 inserts per second is large and the cost is small if the storage is local. Document the filesystem requirement in the Docker setup notes.

## Open questions

- Does the gain hold with realistic row sizes, or only with the small rows the benchmark used?
- How does the read side behave when many workers write at once?
- Is the log file size bounded in practice under normal PatchPilot load?

## Next steps

- Rerun the benchmark with a mixed read and write workload.
- Rerun it inside the Docker image with the real volume setup.
- Add a startup check that confirms WAL mode is actually active, and log a warning when it is not.

## Related work

Any change to how the state-store opens its connections should be compared against this result, so a regression back to the slower write rate is caught early.
