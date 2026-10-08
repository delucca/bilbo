---
id: 01M00J6CQXSMHRMCZ5KN5B8STY
created: 2026-08-14T13:37-03:00
---

# retention-sweeper hard-delete window: after 45 days

This note replaces the earlier note "retention sweeper soft deletes"; the new value is that retention-sweeper hard-deletes soft-deleted entries after 45 days, where the old note said 30 days.

The change follows the new records policy. Nothing else about how soft deletes work was changed by this decision. Anyone reading only this note should come away knowing the window, why it moved, what it touches, and what to check before touching it again.

## Decision

retention-sweeper now hard-deletes soft-deleted entries after 45 days. The earlier window was 30 days. The window is counted from the moment an entry was soft-deleted, not from when the entry was created or last edited. Until the window has passed, a soft-deleted entry stays recoverable and stays in the audit trail like any other record. Once the window has passed, the next sweep run removes the entry for good.

This is a policy-driven number. It is not a tuning knob for performance or storage cost. If someone wants to change it again, the request should come from whoever owns the records policy, not from an engineering preference.

## Why it changed

The records policy was revised and the longer window is what it requires. The compliance officers who use LabNotebook Sync asked that the sweeper line up with it, so that a soft-deleted entry can still be restored and still be produced during a review for the full period the policy names. The old shorter window would have destroyed data the policy says must still exist.

The reasoning is simple: deleting too early is the expensive mistake here. Deleting too late costs some storage and nothing else. So when the policy and the old behavior disagreed, the policy won.

## What is affected

The behavior change is limited to the retention-sweeper component. In practice that means:

- Soft-deleted notebook entries live longer in SQL Server before the sweeper removes them.
- Anything the sweeper removes along with an entry, such as linked instrument output held in Azure Blob Storage, is removed later as well, because it follows the entry.
- Reports and dashboards that count soft-deleted entries will show a larger number than before, since entries stay in that state for longer.

The sync path between the electronic notebook and instrument output is not changed. Messages carried over RabbitMQ are not changed either. Only the point at which a hard delete happens has moved.

## Transition behavior

When the new value took effect, entries that were already soft-deleted did not get a special case. They are judged against the new window like everything else. An entry that had already sat past the old window but had not yet been swept is now simply not due until the new window has passed. An entry that had been swept under the old window is gone and cannot be brought back by this change.

So there is a one-way effect: the longer window protects entries still present, and does nothing for entries already hard-deleted. Do not promise anyone a restore of something removed before the change.

## Things to check if behavior looks wrong

If entries seem to disappear too early or linger too long, check these in order:

- The configured retention value actually in use in the environment, not just the default in source. A stale deployment setting can keep the old window alive.
- The clock reference used for the soft-delete timestamp. The window depends on that timestamp being recorded consistently.
- Whether the sweeper ran at all, and whether a failed run left work for the next one.
- Whether a legal or compliance hold exists on the entry. Holds are meant to override the window, and a held entry should never be swept.

Compare against the number in this note first. If the live behavior does not match after 45 days, that is a bug or a configuration drift, not an intended difference.

## Audit trail implications

The product exists to enforce audit trails, so a hard delete is a sensitive event. The sweeper's removal of an entry must still leave a record that the removal happened, who or what did it, and under which rule. That requirement is unchanged. What changed is only the rule's window.

Reviewers who read old audit records may see removals attributed to the earlier window. That is correct history and must not be rewritten. Records made after the change refer to the new window. If a reviewer asks why two periods look different, point them to this note and to the records policy revision.

## Operational notes

Because entries now stay soft-deleted for longer, expect the pool of soft-deleted rows to be larger at steady state. Watch table growth and index health in SQL Server for a while after the change, and watch blob storage use for linked instrument output. Neither should be alarming, but a larger steady state is a predictable outcome and not a leak.

The first sweeps after the change may find little to remove, since fewer entries are due. That is expected. A quiet sweeper in the days after the switch is not a sign that it is broken.

If a sweep has to be paused for maintenance, pausing is safe: entries only get older, and the next run catches up. Pausing never causes an early delete.

## Testing and verification

Tests that assumed the old window needed their dates moved. Any test that builds a soft-deleted entry with a timestamp just inside or just outside the window should be checked against the new value and not against a remembered one. Boundary cases matter most: an entry just before the window must survive a sweep, and an entry just after must be removed.

A manual check on a non-production environment is worth doing after any change to this area. Soft-delete a test entry, advance the clock past the window, run a sweep, and confirm both the removal and the audit record that explains it. Also confirm a held entry survives.

## Open questions and follow-ups

- If the records policy changes again, this note should be updated in place with the new value, not duplicated.
- It is not yet settled whether different kinds of entries could one day need different windows. For now there is a single window for all soft-deleted entries, and that is the decision recorded here.
- Documentation aimed at scientists should state the window plainly, since people often assume a delete is immediate or permanent in a way it is not. Compliance officers will want the same wording used in their review material.

For the record once more: retention-sweeper hard-deletes soft-deleted entries after 45 days, replacing the earlier 30 days, to match the new records policy.
