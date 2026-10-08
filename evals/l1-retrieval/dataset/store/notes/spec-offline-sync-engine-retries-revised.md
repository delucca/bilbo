---
id: 01KTEZYH30S59FFSA4QBPG16DM
created: 2026-06-06T14:33-03:00
---

# offline-sync-engine spec: retry backoff

This note replaces the earlier note about "offline sync engine retries". The new value: offline-sync-engine now starts its retry backoff at 60 seconds, where the earlier start value was 30 seconds. Anything that still says 30 seconds for the first retry is out of date.

## Scope

This spec covers how offline-sync-engine retries failed uploads of delivery records from the courier's phone to the backend. It does not cover how proof-of-delivery data is captured, and it does not cover the Firestore security rules. The component's internal codename is `lazarus`. You will see that name in older logs, branch names and chat threads. In this note and in new docs, use `offline-sync-engine`.

## What changed

The first retry after a failed sync attempt now waits 60 seconds. Before, it waited 30 seconds. The later steps of the backoff still grow from that starting point, so every wait in the sequence moves later because the base moved. The shape of the growth and the upper cap are not changed by this note. Check the code for the exact multiplier and the cap before relying on them.

## Why the start value went up

Couriers often work in places with poor signal: stairwells, basements, rural stretches. A 30 second start meant the app woke up, tried again, and failed again while the device was still without a usable network. That burned battery and filled the retry queue with attempts that had no chance. A longer first wait gives the radio time to recover, and it reduces the burst of requests that hits Cloud Firestore when many drivers regain signal together, for example when a depot Wi-Fi comes back.

## Effects to watch

- A record captured offline may take half a minute longer to reach the backend after the first failure. Dispatch screens that show "synced" status will lag a little in the bad-network case.
- Support staff who tell couriers "it will retry in about half a minute" should now say about a minute.
- Tests that assume the old start value need updating. Any test that fakes the clock and advances by the old amount will see no retry fire.
- A manual "sync now" action from the courier is not affected by the backoff.

## Open points

- Whether the start value should be remote-configurable through Firebase instead of fixed in the app. Not decided.
- Whether jitter should be added on top of the start value. Not decided here; the change in this note is only the base wait.
- Queued Protocol Buffers payloads keep their format. The backoff change does not touch the wire schema.
