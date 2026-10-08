---
id: 01KJJQZNFS9PGQB20MBWVXNCCR
created: 2026-02-28T15:25-03:00
---

# recon-results-table design (tallyhawk)

The recon-results-table stores one row per settlement and entry pair, with the column `recon_status` holding one of three values: matched, mismatch or pending. That is the core of the design. A pair is one line from a card-processor settlement file plus one internal ledger entry it was compared against. Every pair gets exactly one row, and `recon_status` says where that pair stands. The internal codename of recon-results-table is `tallyhawk`, so you will see that name in older tickets, dashboards and chat. In this note the component is always called `recon-results-table`.

## What a row means

One row per settlement and entry pair. There is no row for a settlement alone and no row for an entry alone, except as part of a pair. When a settlement line has no counterpart in the ledger yet, the pair is still recorded, with the entry side empty and `recon_status` set to pending. The same applies in the other direction when the ledger has an entry and the processor has not reported it yet.

The three values of `recon_status` mean this:

- matched: the settlement line and the ledger entry agree on the fields we compare, amount and currency among them. No one needs to look at it.
- mismatch: both sides exist but something differs, or the comparison found a clear conflict. These rows are what finance operations reviews.
- pending: the pair is not resolved. Usually one side has not arrived. A pending row can later turn into matched or mismatch when the missing data shows up.

## Status transitions

New pairs start as pending unless both sides are already present at comparison time, in which case they go straight to matched or mismatch. From pending a row moves to matched or mismatch. A mismatch can move to matched if a correction lands in the ledger or a later settlement file fixes the processor side. A matched row should not move back, and if it does, treat that as a bug or a real reversal and look at the history before trusting the value.

Keep the status as the only source of truth for review queues. Do not derive "needs review" from other columns, because that drifts from what the reconciler decided.

## Who writes and who reads

The Go reconciler is the only writer. It consumes settlement records and ledger entry events from Kafka, pairs them, and upserts the result. Upserts are keyed on the pair, so replaying a Kafka topic does not create duplicates; it just rewrites the same row with the same or a newer status. Readers are the review UI backend and reporting jobs, which reach the data through gRPC services rather than querying PostgreSQL directly. Anything that needs to filter by state should filter on `recon_status` and let the service handle the rest.

## Storage notes

The table lives in PostgreSQL. Review queues mostly ask for mismatch and pending rows, and those are a small share of the total once a period is closed, so an index that leads with `recon_status` is the useful one. Matched rows dominate volume and are rarely read after the first pass. Schema changes go through the Terraform-managed database setup and the normal migration flow, not by hand on a live instance.

Retention for matched rows is a separate question from retention for mismatch rows. Finance teams usually want mismatches and their resolution trail kept longer. Nothing about that is settled here; check with the owners before adding any cleanup job.

## Gotchas

- Do not read pending as a problem. Late settlement files make many rows pending for a while, and alerts should not fire on pending alone.
- Do not add a fourth status value without updating every consumer. The review UI and reports assume exactly matched, mismatch and pending.
- Searching old material for `tallyhawk` finds the same thing as `recon-results-table`. Do not create a second table or note because the names differ.
- Counting rows is counting pairs, not settlements and not entries. A single settlement line that is split across several entries yields several rows.

## Open questions

- Whether a separate resolved or ignored state is wanted for mismatches that reviewers close as accepted. Today that is handled outside `recon_status`.
- How long to keep matched rows.
- Whether pending rows should get an age limit that turns them into mismatch automatically.
