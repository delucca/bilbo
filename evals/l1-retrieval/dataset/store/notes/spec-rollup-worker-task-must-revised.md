---
id: 01KSACEC1M0TY0MNXV6FRYFBM8
created: 2026-05-23T09:20-03:00
---

# rollup-worker task time limit

This note replaces the earlier note about "rollup worker task must". The new value: the soft_time_limit of rollup-worker tasks is now 900 seconds, which replaces the earlier 600 seconds. Anything that still assumes the old limit is out of date.

## What the limit is

rollup-worker is the Celery worker in ClassroomCompass that builds the aggregated progress views. It takes per-student results recorded against curriculum standards and combines them into class-level and school-level summaries. Teachers see these summaries in the Vue.js front end, and the exercise suggestions use them as input.

Each rollup-worker task runs with a soft_time_limit of 900 seconds. When a task passes that limit, Celery raises SoftTimeLimitExceeded inside the task. The task gets a chance to catch it, save what it has finished and exit cleanly. This is different from the hard time limit, which kills the worker process with no chance to clean up. The soft limit is the one this note is about, and it is now 900 seconds.

## Why it was raised

The earlier value of 600 seconds was too tight for the larger rollups. Schools with many classes and many standards ran long, especially when Elasticsearch was slow to return the underlying records. Tasks hit the soft limit part-way through and had to be retried. Retries restarted the work from the beginning, so the same slow rollup could fail again and again and leave teachers looking at stale numbers.

Raising the soft limit to 900 seconds gives these tasks room to finish on the first attempt. It does not make the rollup itself faster. If a rollup needs more than 900 seconds, the right fix is to split the work into smaller tasks, not to raise the limit again.

## What to check when changing or reading this

- Keep the hard time limit above the soft limit. The gap must be big enough for the task to handle the soft exception and write its partial state.
- The broker visibility timeout, if the broker uses one, must be longer than the soft limit. If it is shorter, the broker can hand the same task to a second worker while the first is still running.
- Any alert that fires on long-running tasks should be moved to match the new soft limit. An alert tuned to the earlier 600 seconds will now fire on normal runs or never fire when it should.
- Task retry delays were chosen for the old limit. Look at them again if retries pile up.
- Do not copy the value into other workers by habit. Only rollup-worker tasks have this limit. Other Celery workers in the project keep their own settings.

## Behavior to expect

Normal rollups finish well inside the limit. Only the largest ones get close. After the change, there should be fewer soft-limit exceptions and fewer retries in the rollup-worker logs. If the exceptions keep happening at the new limit, treat that as a sign of a slow dependency, such as Elasticsearch queries or a database lock, and look there first.

A task that runs for the full 900 seconds holds a worker slot for that whole time. With several large rollups queued together, the other rollup tasks wait. Watch queue depth after the change, and if waiting becomes a problem, add worker capacity or break the big rollups into pieces.

## Open points

- Nobody has measured the longest real rollup since the change. Do that and record the number here if it is close to the limit.
- Check that the hard limit and the broker timeout were reviewed together with this change.
- If the limit is changed again, update this note and say what replaced what, the way this one does.
