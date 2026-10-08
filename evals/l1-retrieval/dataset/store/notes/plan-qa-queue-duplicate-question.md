---
id: 01M0EZG5N61TFKM3YQSP4B4587
created: 2026-08-20T03:59-03:00
---

# qa-queue duplicate-question clustering plan

Plan to add duplicate-question clustering to qa-queue, using trigram similarity with a threshold of 0.82, targeted for the v2.4 release. Written quickly; details below stay general on purpose and need filling in once work starts.

## Naming

The component used to be called `askline`. It is called `qa-queue` now. Old branches, dashboards, log lines and docs may still say `askline`; treat them as the same thing as `qa-queue`. New code, docs and notes should use `qa-queue` only.

## Goal

Large events get many near-identical questions. Moderators waste time reading the same thing ten times. Clustering groups the duplicates so a moderator sees one entry with a count, not a wall of repeats.

## Approach

Compare each incoming question to the existing open ones using trigram similarity. When the score is at or above 0.82, the new question joins the existing cluster. Below 0.82, it starts a new cluster. The threshold is 0.82 and should live in config, not be hardcoded, so it can be tuned per event later.

## Target release

Aim for v2.4. If it slips, the feature should ship behind a flag rather than hold the release.

## Why trigrams

They are cheap, tolerate typos and small wording changes, and need no model hosting. CockroachDB has trigram support, so a first version could do the matching in the database. An in-process check in the Elixir side is the fallback if the database route is too slow under load.

## Where it runs

Matching happens when a question is accepted into qa-queue, before it is broadcast over the WebSocket channels. Moderators and the Next.js moderation view then receive the already-clustered result.

## Normalization

Lowercase the text, strip punctuation and collapse whitespace before comparing. Decide whether to drop common filler words; test on real event data before choosing.

## Cluster representative

One question is shown as the representative of the cluster. Default to the earliest one. A moderator should be able to pick a different one.

## Moderator controls

Moderators need to split a wrongly merged cluster and merge clusters by hand. Manual decisions must win over automatic ones, and later automatic matching must not undo them.

## Attendee experience

Attendees should not be told their question was merged in a way that feels like it was thrown away. Show that it was received. Upvotes on merged questions should count toward the cluster.

## Data model

Clusters need a stable identity and a link from each question to its cluster. Keep the original question text untouched. Migration on CockroachDB should be additive so it can roll out without downtime.

## Performance

Comparing against every open question gets expensive in a big event. Limit comparison to open, unanswered questions and consider indexing. Measure with a large synthetic event before committing to the database route.

## Risks

A threshold that is too low merges different questions. One that is too high misses duplicates. Short questions score oddly with trigrams. Non-English text may behave differently.

## Testing

Unit tests for normalization and scoring around the threshold. A replay test using recorded event traffic. A load test for the matching path.

## Rollout

Ship behind a flag, enable for a few internal events, review merge quality with producers and community managers, then enable by default.

## Open questions

Is a single global threshold enough? Should answered questions still attract new duplicates? How are clusters shown on the public display?

## Next steps

Confirm the matching location (database or in-process), write the migration, add the config setting, build the moderator controls, then test and roll out in time for v2.4.
