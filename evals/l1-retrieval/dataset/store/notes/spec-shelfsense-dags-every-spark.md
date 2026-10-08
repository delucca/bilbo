---
id: 01K3QJK5XWW5CFCV0EPB9B62B2
created: 2025-08-28T02:32-03:00
---

# shelfsense-dags: Spark task retry spec

This is the retry rule for Spark work in shelfsense-dags. Every Spark task in shelfsense-dags must set `retries=2` with a retry delay of 10 minutes. That is the whole rule. The rest of this note explains where it applies, why it exists, what it does not cover, and what to check when reviewing a change to the DAGs. If you only need the rule, you have it. If you are about to change a DAG or add a task, read on, because most of the trouble with this rule comes from the edges: what counts as a Spark task, what happens when a retry meets a partial write, and how the retry delay interacts with the rest of the schedule.

ShelfSense predicts store-level stockouts and generates replenishment orders for grocery chains. Merchandising analysts read the output, so a late or missing run shows up on their screens as stale recommendations. The DAGs in shelfsense-dags are the Airflow side of that pipeline. They launch Spark jobs written in Scala, those jobs read and write Delta Lake tables, and some downstream steps push results into Snowflake. The retry rule is meant to absorb short-lived infrastructure trouble without anyone being paged for it.

## The rule in plain terms

Each Spark task declares `retries=2`. Each Spark task declares a retry delay of 10 minutes. Both values are set on the task itself, or on a shared helper that every Spark task goes through, so that a reader of a single DAG file can see them or can see the one place they come from. A task that inherits them silently from some distant default is hard to audit, so prefer explicit or one obvious helper.

So the first attempt can fail, then a retry happens after 10 minutes, and if that fails a second retry happens after another 10 minutes. After that the task is marked failed and the normal failure path takes over. Total retries is the number in `retries=2`, not the total number of attempts. People mix this up. The attempt count is one more than the retry count. Keep that in mind when reading logs: a task that shows the original attempt plus two more is a task that used its full budget.

The delay is a fixed wait. We do not use exponential backoff for Spark tasks here. The fixed wait of 10 minutes was chosen to be long enough for a cluster hiccup, a spot capacity gap, or a brief storage throttle to clear, and short enough that the nightly chain still finishes before analysts start their day.

## What counts as a Spark task

A Spark task is any Airflow task whose job is to run a Spark application: a submit operator, a task that calls a cluster API to start a Spark job and waits for it, or a wrapper operator we wrote that does the same. If the task ends up running Scala code on a Spark cluster, it is a Spark task and the rule applies.

Things that are not Spark tasks, and so are not governed by this note: sensors that wait for files or table partitions, tasks that only run SQL in Snowflake, small Python callables that move metadata around, and notification tasks. Those can have their own retry settings. Do not copy the Spark values onto them just to look consistent. A sensor that retries every ten minutes on top of its own poll interval is a good way to hide a real upstream delay.

Gray area: a task that runs a Spark job but is triggered through a generic shell-style operator. Treat it as a Spark task. The operator type is not what matters, the work is. If a reviewer is unsure, the answer is to apply the rule.

Another gray area: a task group that wraps several Spark tasks. The rule applies to each task inside, not to the group as a whole. Airflow does not retry a group, it retries tasks, so the setting has to live on the tasks.

## Why these values

The failures we see on Spark tasks are mostly transient. Executors get lost when nodes are reclaimed. The driver cannot get a slot because the cluster is briefly full. Object storage returns a throttling answer under heavy parallel reads. Metastore calls time out for a moment. None of these need a human, and most of them are gone within a short while.

Without retries, each of these becomes a failed run and a page. With too many retries, a real bug in the Scala code gets rerun over and over, burns cluster time, and delays the alert that someone needs to look. The budget in `retries=2` is a compromise: enough chances to ride out a transient problem, few enough that a genuine defect surfaces quickly.

The delay of 10 minutes matters as much as the count. Retrying immediately tends to hit the same broken condition, for example the same full cluster, and wastes an attempt. Waiting 10 minutes gives autoscaling or the storage layer time to recover. It also gives anyone watching a window to notice a pattern before the task is finally marked failed.

## Interaction with Delta Lake writes

Retries are only safe if rerunning the job gives the same result as running it once. Delta Lake helps here because commits are atomic: a job that dies before committing leaves the table as it was. But it does not make every write idempotent for free.

Rules of thumb for the Scala jobs that these tasks launch:

- Overwrites of a partition or a date range are safe to retry, because the second run replaces whatever the first run might have left.
- Plain appends are not safe by themselves. If the first attempt committed and then something failed afterward, such as the task losing its connection to the cluster, a retry would append the same rows again. Jobs that append need a key-based merge or a dedupe step.
- Merges keyed on a stable business key are safe to retry.
- Jobs that write to more than one table are the risky ones, because the first attempt may have finished one write and not the other. Make the second write tolerant of the first having already happened.

If you add a Spark task whose job is not safe to rerun, do not lower the retries to dodge the problem. Fix the job so it is safe, or raise it in review. The rule is uniform on purpose, and exceptions should be rare and written down.

## Interaction with Airflow settings

The retry values sit alongside other Airflow settings, and some of them interact.

Task timeouts: if a Spark task has an execution timeout, it applies to each attempt. A hung job that hits the timeout counts as a failure and will be retried like any other. Make sure the timeout is shorter than whatever window the downstream tasks need, because the worst case is the original attempt plus each retry plus two waits of 10 minutes each. Add that up when you reason about whether the chain can finish on time.

Pools and concurrency: while a task waits out its retry delay it is not holding a worker slot in the usual sense, but it will want its slot again when the delay ends. If many Spark tasks fail at once for the same cause, they will all come back at the same moment. That can recreate the load that caused the failure. If you see this thundering-herd pattern, adjust pool sizes or stagger the tasks, not the retry delay.

SLAs and alerts: an SLA miss alert can fire while a task is still inside its retry budget. That is expected. Do not widen SLAs to hide it without checking whether the retries are being consumed regularly, which is a sign of a real problem.

Backfills and manual clears: clearing a task resets its retry count for the new run. A person clearing a failed Spark task over and over is doing the retrying by hand and bypassing the delay. Fine for debugging, but do not use it as a routine fix.

## How to set it in code

Prefer one shared place. The cleanest pattern is a small helper or a default-arguments block used only for Spark tasks, containing `retries=2` and a retry delay of 10 minutes expressed as a time delta. Every Spark task is built through it. Then a change to the policy is a change in one spot, and a reviewer can check new tasks by asking only whether they go through the helper.

If a DAG sets default arguments for the whole DAG, be careful. DAG-wide defaults also apply to the non-Spark tasks, which we said should have their own settings. Either override on the non-Spark tasks or keep the Spark values off the DAG-wide defaults and put them on the Spark tasks.

When a task sets its own retry values explicitly, they must match the rule. A task that sets a different count or delay is a violation unless there is a recorded exception, see the exceptions section below.

Do not rely on cluster-side retry settings as a substitute. Spark has its own internal task and stage retries, and those handle small failures inside a running job. They are separate from the Airflow retry, and they do not replace it. The Airflow retry handles the case where the whole application fails.

## Snowflake and downstream effects

Some DAGs load Spark output into Snowflake after the Spark step finishes. The Spark retry rule does not change how those load tasks behave, but the two interact through timing. A Spark task that burns its whole retry budget pushes the load later by up to the sum of the waits plus the extra runtimes. The load tasks should be written to wait on the Spark task finishing rather than on a clock time.

Loads into Snowflake should themselves be safe to rerun, for the same reason as the Delta writes: if a downstream task is cleared after an upstream retry succeeds, it must not double-load. Use replace or merge semantics rather than blind inserts.

If a Spark step ultimately fails after its retries, downstream Snowflake loads must not run on stale or partial data. Leave the default dependency behavior so that an upstream failure blocks the load, and do not add trigger rules that let loads run anyway unless there is a clear and reviewed reason.

## Failure handling after retries run out

When the retry budget is used up the task is failed. At that point the usual path applies: the failure alert goes out, the downstream tasks do not run, and someone on the team looks at it. The alert should say which task, which DAG run, and that retries were exhausted, so the person on call knows this is not a one-off blip.

First things to look at, in order. Was it the same error on every attempt? If yes, it is likely a code or data problem, not infrastructure. Did the attempts fail for different reasons? That points at an unstable cluster or storage. Did the first attempt die very early? Look for configuration or permissions. Did the last attempt die late? Look for data volume or memory.

Keep the logs from every attempt. Airflow keeps them per attempt, and the pattern across attempts is often more useful than the final one alone.

If the same task burns through its retries repeatedly across nights, treat it as a bug even if the second retry usually works. A task that only succeeds on its last chance is one bad night from an outage. Open an issue, find the cause, and do not raise the retry count as the fix.

## Exceptions

There should be very few. A task might legitimately need a different setting if, for example, the job is very expensive and a rerun has a real cost, or if a vendor system on the other end forbids repeated calls. If you think a task needs an exception, write it down next to the task in a comment with the reason, get it reviewed, and add a line to this note listing the task by name. An exception without a written reason is just a mistake.

Do not grant an exception because the job is flaky. Flaky jobs are exactly what the rule is built for, and the real fix is to make them less flaky.

Do not shorten the delay to speed up testing in the shared environment. If you need fast failure while developing, use a local or test configuration that is clearly separate and cannot be merged into the production DAGs by accident.

## Review checklist

For any change to shelfsense-dags that adds or edits a Spark task, check these:

- The task sets `retries=2`, directly or through the shared helper.
- The retry delay is 10 minutes, directly or through the shared helper.
- The Scala job behind it is safe to run twice with the same inputs, especially if it appends to a Delta table or writes more than one table.
- The task timeout, if any, leaves enough room for the worst case of all attempts and both waits.
- Downstream loads wait on the task and will not run on a failed upstream.
- Non-Spark tasks in the same DAG did not get the Spark values by accident through DAG-wide defaults.
- Any deviation from the rule has a written reason and is listed as an exception.

A reviewer who finds a missing or different value should ask for it to be fixed rather than approve with a note to fix it later. These are cheap to get right at the start and annoying to find during an incident.

## Open questions

A few things are not settled and are worth deciding at some point.

Whether the fixed wait should become a growing wait for the heaviest jobs. Right now the answer is no, the single value is easier to reason about, but if cluster recovery times turn out to vary a lot we may revisit it. Any change would be a change to this spec first, then the code.

Whether we should add a check in CI that scans the DAG definitions and fails if a Spark task lacks the right values. It would catch drift early and make the review checklist shorter. Nobody has built it yet.

Whether the alert on exhausted retries should carry more context, such as a short summary of the error from each attempt, so the person on call can tell transient from systematic without opening the logs.

Until those are decided, the rule stands as written at the top: each Spark task in shelfsense-dags uses `retries=2` and a retry delay of 10 minutes.
