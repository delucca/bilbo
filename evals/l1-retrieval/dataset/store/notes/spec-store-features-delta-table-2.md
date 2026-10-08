---
id: 01KEZ872HR2V3F2R55Y2GQFT40
created: 2026-01-14T18:57-03:00
---

# store-features-delta spec: history retention

This note specs how long the store-features-delta table keeps its history and what depends on that. The rule is short: the table must keep history for 30 days through `delta.logRetentionDuration` set to `interval 30 days`. Everything below is the reasoning, the places where the setting is easy to break, and what to check when someone touches the table. If you only need the rule, that sentence is it. If you are about to change the table, retention, or the jobs that write to it, read the rest first.

store-features-delta is the Delta Lake table in ShelfSense that holds the per-store feature rows the stockout model reads. Spark jobs written in Scala build and update it, Airflow schedules those jobs, and downstream pieces read it both for training and for scoring. Merchandising analysts at grocery chains never see the table directly, but they see its effects: the stockout predictions and the replenishment orders that come out of them. When the table is wrong or its history is gone, the analysts get orders they cannot explain.

## What the requirement says

The table property `delta.logRetentionDuration` must be set to `interval 30 days` on store-features-delta. That property controls how long Delta Lake keeps transaction log entries before they are eligible for cleanup. In practice it decides how far back you can go with time travel on the table, whether by version or by timestamp. With the property at `interval 30 days`, a reader can ask for the table as it looked at any point in roughly the last 30 days, provided the data files for that point also still exist (see the vacuum section below, because the log setting alone does not guarantee that).

The requirement is about the table, not about a particular job. Any job, notebook, or one-off script that recreates the table, replaces it, or clones it must carry the property over. The property lives in the table metadata, so a plain overwrite of the data is fine, but dropping and recreating the table without setting the property again is not. Treat the property as part of the table definition, the same as the schema.

The value is written exactly as `interval 30 days`, with the word interval first. Delta parses this as an interval string. A looser spelling may be rejected or, worse, parsed into something other than intended, so copy the string as written rather than retyping it from memory.

## Why 30 days

Three needs drive the number, and none of them is satisfied by the default retention that Delta ships with.

The first is reproducing a past order. When an analyst questions why a replenishment order for a store looked the way it did, someone has to rebuild the feature rows the model saw at the time. The features table changes as new sales and inventory signals land, so the only reliable way to see what the model saw is to read the table as of the run. Orders get questioned well after they are generated, often at the end of a review cycle, so the window has to be long enough to cover a normal review cycle with room to spare.

The second is backtesting and debugging model drift. When predictions get worse for a group of stores, the first question is whether the inputs changed or the model did. Comparing the table across versions answers the input half of that question. A short history forces people to guess.

The third is recovery from a bad write. A faulty upstream feed or a buggy transformation can corrupt a batch of rows. With enough history, the fix is to restore the table to the version before the bad write and rerun, instead of rebuilding features from raw data. A bad write that goes unnoticed for a few days is common, especially over a weekend, so the window has to be well beyond a couple of days.

The 30 days figure is a compromise between those needs and storage cost. Keeping history means keeping old data files, and the features table is rewritten often. Going longer than needed has a real storage bill; going shorter has hit us in review conversations. If someone proposes a different window, the change should come with a reason tied to one of the needs above, and this note should be updated in the same change.

## What the setting does and does not do

It is easy to read `delta.logRetentionDuration` as a promise that old versions stay readable. It is narrower than that.

The log retention property governs the transaction log. Log entries older than the window can be removed during checkpoint cleanup. If the log entries for a version are gone, time travel to that version fails regardless of what data files remain. So a shorter log window cuts history directly.

The log is only half of what time travel needs. The data files that a past version points to must also still exist. Those files are governed by a separate mechanism, the vacuum command, and by a separate table property for deleted file retention. If vacuum removes the files a past version needs, time travel to that version fails even though the log entry is still there. So keeping 30 days of history means the log window and the file retention window both have to cover that period.

The practical consequence is that setting `delta.logRetentionDuration` to `interval 30 days` is necessary but not sufficient. The deleted file retention for the table must be at least as long as the log window, or the log will advertise versions that cannot be read. When reviewing the table configuration, always look at both properties together. When a vacuum job exists for this table, check what retention it uses, because an explicit retention argument on the command can override the table property and quietly shorten the real history.

The property also does not affect current reads. Readers of the latest version are unaffected by the window. The only people who notice a retention problem are those who ask for the past, which is why a mistake here can go unnoticed for a long time and then show up all at once during an incident.

## Where it can get broken

Most breakage here comes from a few repeating situations, so it is worth listing them.

Table recreation. A Scala Spark job that does a create or replace on the table, or a migration that drops and recreates it, resets table properties unless the job sets them again. After any change that recreates the table, verify the property. The same applies to restoring from a backup or cloning into a new location: some paths carry properties and some do not, and you cannot assume either.

Environment drift. A development or staging copy of the table may be created by hand and never get the property. That is mostly harmless, but it makes tests of restore and time travel misleading. If you are validating a restore procedure, validate it against a table that has the real setting, not one that happens to pass because nothing has been cleaned up yet.

Vacuum with a shorter retention. Someone runs vacuum with a short explicit retention to save space or to clear a problem, and the history inside the window is gone. Delta normally protects against very short retention with a safety check, and turning that check off to make a vacuum run is a red flag. If you find yourself disabling it for this table, stop and reconsider.

Tooling that sets properties wholesale. Some deployment scripts apply a full set of table properties from a config file. If that config does not list the retention property, a script that replaces the property set rather than merging into it can drop the setting. Keep the retention property in whatever config the deployment reads, so a re-apply does not remove it.

Silent default. If the property is missing, Delta uses its own default log retention, which is shorter than what we need. Nothing errors. The table works, writes succeed, and the only symptom appears later when time travel to an older version fails. That is the worst kind of failure for this requirement, so the check needs to be active rather than waiting for an incident.

## How to check and how to set

To see whether the table has the right value, read the table properties of store-features-delta, either through the describe detail output or by listing the table properties through Spark SQL, and confirm that `delta.logRetentionDuration` shows `interval 30 days`. Do the same for the deleted file retention property and confirm it is not shorter. A person doing a quick check should look at both lines in the same output.

To set it, alter the table properties so that `delta.logRetentionDuration` becomes `interval 30 days`. This is a metadata change. It does not rewrite data files and does not need downtime, and it applies from that point on. It does not bring back history that was already cleaned up before the property was set, so setting it late does not recover anything. If the property was missing for a while, assume history older than the default window is gone and say so to anyone who asks about past versions.

When creating the table in code, set the property at creation time in the same statement or builder call that defines the schema. That way there is no gap between a table existing and a table having its retention. In the Scala jobs, keep the property next to the other table definition code rather than in a separate step that might get skipped.

After any change, record the check in the pull request or change description: which property values were seen and where. This is cheap and saves the next person from redoing it.

## Interaction with the jobs around it

Airflow schedules the Spark jobs that write to store-features-delta, and those jobs are the most frequent writers. Frequent writes produce many versions and many small log entries, and Delta checkpoints the log periodically so that readers do not need to replay everything. The cleanup of expired log entries happens as part of that checkpoint activity, which means the actual removal of old entries is not exactly at the window boundary. History can last a little longer than the window; it should never be shorter. Do not build anything that relies on versions disappearing at an exact moment.

Because history includes every write, a job that rewrites the entire table on each run makes every version large. If a pipeline change turns an incremental update into a full rewrite, storage under the 30 days window grows quickly. That is a cost issue, not a correctness issue, but it is the usual reason people ask to shorten retention. The better response is usually to fix the write pattern. Raise it before touching the retention value.

Restores deserve a note of their own. Restoring the table to an earlier version is itself a write and creates a new version. The restore only works if the target version's log entry and data files are present. Anyone planning a restore should confirm the target is inside the window first, and should not wait until the last days of the window, since the cleanup timing means the margin is less certain than it looks.

Airflow tasks that depend on time travel, such as a backfill or comparison job that reads a past version of the table, should handle the case where the version is no longer available. They should fail with a clear message rather than fall back to the latest version, because silently reading current data in place of past data would produce results that look right and are wrong.

Snowflake sits further downstream. Parts of ShelfSense that live in Snowflake read feature data that originates from this table, usually through an export or a load step. Snowflake has its own notion of history and its own retention, which is separate from this one. The two should not be confused: the retention on store-features-delta says nothing about how long Snowflake keeps anything, and a Snowflake setting says nothing about this table. When someone asks how far back we can reproduce features, the answer for the Delta side is the window set here, and the answer for the Snowflake side has to be checked there.

## Readers and what they can rely on

Training jobs normally read the latest version, or a version pinned for a specific training run. A pinned version is only usable as long as it stays inside the window. If a training run needs to be reproducible for longer than the window, the run should copy what it needs or record enough to rebuild it, instead of counting on the table to hold the version. The table guarantees the window and nothing beyond it.

Scoring jobs read the latest version and do not care about history. They are not affected by this requirement, apart from the fact that the table they read carries more files because of it.

Analysts and support engineers who investigate a specific order are the main consumers of history. For them, the useful statement is simple: for about the last 30 days, the feature table can be read as it was at a given time. Older than that, it cannot, and the investigation has to work from what the order itself recorded.

Anyone writing a new reader that uses time travel should read by timestamp or version, handle the unavailable-version error explicitly, and avoid caching assumptions that a version will exist later. A version that exists today may be cleaned up by next month.

## Things to do when changing this

If you change the retention value, change it in the table definition code, in the deployment config, and in this note, and verify the deleted file retention still covers the same window. If you change vacuum behavior for the table, check the retention it uses against this requirement. If you recreate the table, set the property at creation and verify it afterward. If you add a new environment copy, decide whether it needs the property and write the decision down.

If you find the table without the property, set it right away and tell whoever owns the pipeline, because history before the fix is probably already lost and reviewers should hear that rather than discover it. If you find the file retention shorter than the log window, fix that too, since the effective history is the shorter of the two.

## Open points

A few things are not settled and should be confirmed by whoever next works on this table. First, whether any scheduled vacuum job exists for store-features-delta and what retention it passes. Second, whether the deployment config that applies table properties merges or replaces. Third, whether any downstream job quietly depends on versions older than the window, which would be a bug but would only show up when it fails. None of these has been verified in this note, so treat them as questions, not facts.

The settled part is the requirement: `delta.logRetentionDuration` set to `interval 30 days` on store-features-delta, with file retention that does not undercut it.
