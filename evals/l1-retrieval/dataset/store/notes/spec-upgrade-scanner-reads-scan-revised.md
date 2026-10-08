---
id: 01KV6TTCHY8FMEFXXP352AQPFZ
created: 2026-06-15T20:45-03:00
---

# upgrade-scanner scan interval default

This note replaces the earlier note about "upgrade scanner reads scan". The new value is that `upgrade-scanner` now defaults `UPGRADE_SCAN_INTERVAL` to `4h`, replacing the earlier default of `6h`. Anything in the old note that says the default is `6h` is out of date and should be ignored.

The rest of this note is the context a later session needs so it does not have to work the same things out again: what the setting controls, how to reason about the shorter default, where it can bite, and what to check when something looks off. Where I do not know a detail, I say so instead of guessing. Only add what you have verified in the code or in a running instance.

## What changed

The `upgrade-scanner` is the part of PatchPilot that looks across the repositories a platform team maintains and decides which dependencies have newer versions worth proposing. It does not open the pull requests itself and it does not run the targeted tests. It produces the list of candidate upgrades, and later stages pick those up. How often it produces that list is set by `UPGRADE_SCAN_INTERVAL`.

The default for that setting is now `4h`. Before, it was `6h`. That is the whole change in behavior. The setting name is the same, the format of the value is the same, and the meaning is the same. Only the value used when nobody sets anything is different.

A minimal example of setting it explicitly, using the new default:

```sh
UPGRADE_SCAN_INTERVAL=4h
```

If a deployment already sets the variable to some value, nothing changes for that deployment. The new default applies only to deployments that leave the variable unset. An instance that was relying on the old default will start scanning more often after it picks up the new build, with no config edit on anyone's part. An instance with an explicit value keeps its own cadence until someone edits it.

A changed default is neither a changed ceiling nor a floor. The scanner does not enforce that the value be at or below the default. It is just what you get when you say nothing.

### Why it was shortened

The motivation was freshness. With the longer interval, a release that landed just after a scan could sit unnoticed for most of a working half day before the scanner even considered it. Platform engineers who maintain many repositories said this made the pull requests feel late, especially for security-related releases where they want the proposal to appear the same working morning. Shortening the default is a cheap way to reduce the worst-case delay between a new release appearing upstream and a candidate upgrade existing in PatchPilot.

The worst-case delay from a scan cycle is roughly one interval, plus however long the scan itself takes. Going from `6h` to `4h` cuts the first part by a third. It does not change the time spent by the later stages, such as opening the pull request or waiting for the targeted tests to finish. If someone reports that an upgrade is still late, check whether the lateness comes from the scan or from those later stages before touching this value again.

It was a default change, not a redesign. We did not alter what a scan looks at, how candidates are ranked, or how results are stored. If you find a behavior difference after the change that is not explained by scans simply happening more often, treat it as a separate bug and write a separate note.

## How the interval behaves

This section describes how I understand the interval to work, in terms a reader can check against the code. Where the behavior depends on code I did not re-read for this note, I flag it.

The scanner runs on a timer. When the process starts, it reads `UPGRADE_SCAN_INTERVAL`, falls back to `4h` if the variable is absent or empty, and schedules scans on that cadence. The value is a duration string with a unit suffix, and `4h` is hours. Do not pass a bare number without a unit and assume it means hours; check the parsing code first, because a bare number may be rejected or interpreted in some other unit.

The value is read at startup. Changing the environment of a running process does nothing until that process restarts. In the Docker setup this means the container has to be recreated or restarted with the new environment. In the GitHub Actions setup, where the scanner may run as a scheduled job instead of a long-lived process, the interval is better thought of as how often the job is triggered, and the variable may be irrelevant or may only matter for how the job decides that a previous result is stale. I have not confirmed which of those applies to every deployment shape, so confirm before you promise anyone a specific cadence in an Actions-driven install.

### Scan overlap and slow scans

A shorter interval raises the chance that a scan is still running when the next one is due. The safe behaviors are skipping the next tick or delaying it until the current scan finishes. The unsafe behavior is starting a second scan in parallel against the same state. Read the scheduling code and look for a guard that prevents overlap. If there is no guard, add one before anyone shortens the interval further. With the current default, a scan on a large set of repositories is not expected to approach the interval, but that expectation is based on how things have behaved so far and not on a measured guarantee.

The interval counts from when a scan starts or from when the previous one finished, depending on how the timer is written. These differ when scans are slow. With start-to-start timing, a slow scan eats into the idle time; with finish-to-start timing, the effective cadence is the interval plus the scan duration. Anyone reasoning about how often candidates are refreshed should know which one the code uses. I have left this as something to confirm.

### Interaction with stored state

The scanner records what it has seen in SQLite. More frequent scans mean more reads of upstream version information and more writes of scan results, but the stored data is meant to be idempotent: seeing the same available version twice should not create a duplicate candidate. If a more frequent schedule ever produces duplicate candidates or duplicate pull requests, that is a bug in the de-duplication path and not a reason to put the interval back. Fix the de-duplication.

The database grows with the number of scans only to the extent that scan history is kept. If history is retained per scan, a shorter interval grows it faster, in proportion to the ratio of the two defaults. If disk use matters on a small host, look at how long history is retained and whether old rows are pruned, and tune that, rather than tuning the interval to protect the disk.

### Rate limits and upstream load

The scanner talks to remote services to learn about new versions and to see repository state. More frequent scans mean more requests per day. For a team with many repositories this can approach whatever request limits apply to the credentials in use. The symptom would be scans that start failing or slowing down part way through, usually with a clear rate-limit style response from the remote side. If you see that after this change, the right responses are, in order of preference: make the scanner cache or conditionally request where it can, spread repositories across the cycle rather than hitting them all at the start, and only then raise `UPGRADE_SCAN_INTERVAL` for that deployment. Raising it explicitly for one deployment is fine and does not require reverting the default for everyone.

## Rollout and operations

This is how I would roll the new default out and how I would tell whether it is behaving.

Before the new build reaches an instance, decide whether that instance should keep its old cadence. Some teams deliberately want fewer pull requests per day. For those, set `UPGRADE_SCAN_INTERVAL` explicitly to the value they want, which can be the old `6h`, before upgrading, so that the default change does not alter their experience.

For everyone else, the upgrade is passive. They will see scans happen more often. The visible effect should be that candidate upgrades appear sooner after upstream releases. The effect on the number of pull requests should be small, since the same candidates are found, only earlier. If the number of open pull requests jumps noticeably after the change, the likely cause is not the interval but some limit on open pull requests that was previously never reached because candidates trickled in slowly. Check how PatchPilot limits concurrent pull requests per repository and whether that limit is configured sensibly.

### What to look at after the change

A short checklist, in the order I would do it:

- Confirm the running process actually uses the new default. Look at the startup log line for the effective interval if there is one, and if there is not, consider adding it.
- Confirm that no deployment script, compose file, or workflow still pins the old value by accident. A leftover explicit `6h` somewhere will silently override the new default and make it look like the change did not take effect.
- Watch a few cycles for overlap. If a scan is still running when the next is due, note how the scanner reacts.
- Watch for rate-limit style failures from the remote services during scans, especially for instances with a large number of repositories.
- Check that the database is not growing faster than the host can comfortably handle.

None of these are expected to turn up problems, and I have not seen any. They are the places where a more frequent schedule would show strain first.

### Docker

In the Docker deployment the variable is passed through the container environment. The image itself does not bake in a value other than the code default, so the new default comes from the application, not from the image build. A rebuilt image is enough to pick up the new default if the environment does not set the variable, and an old image with the variable unset keeps the old behavior until it is replaced. If someone reports that their container still scans on the old cadence, first check whether they are running an old image, and second check whether their environment sets the variable explicitly.

Restarting is required for any change to take effect. A container restart also resets the in-memory timer, so the first scan after a restart may come sooner or later than the previous schedule would have implied. That is normal.

### GitHub Actions

Where PatchPilot is driven from GitHub Actions, the schedule that triggers the workflow is defined in the workflow file and not by this variable alone. Changing the default in the application does not change a workflow's cron-style trigger. If a team wants Actions-driven scans to follow the new cadence, the workflow schedule has to be changed on their side. The application default governs the long-running scanner process, and a workflow trigger is its own setting.

Scheduled workflows are not guaranteed to fire at exact times, and under load they can be delayed. Do not promise a team an exact moment for their scan from a scheduled workflow.

## Things to confirm and open questions

These are the items I could not settle from the information behind this note. Each is phrased as a question so that whoever picks it up knows what an answer looks like.

First, does the scheduling code prevent overlapping scans? An answer is a pointer to the guard, or a statement that there is none. If there is none, the shorter default raises the risk and a guard should be added.

Second, is the interval measured from scan start or from scan finish? An answer is a sentence plus a pointer to the timer code. Then update the section above so it no longer says this is unknown.

Third, how does the parser treat values without a unit, with an unknown unit, or with an empty string? An empty string should fall back to the default; I believe it does, but I have not verified it. A malformed value should fail loudly at startup and not quietly fall back, because a silent fallback hides typos. If it currently falls back silently, that is worth fixing.

Fourth, is there a lower bound on the interval? Someone setting an extremely small value could hammer remote services. If there is no floor, consider adding one with a clear error message, or at least a warning in the log. The new default is not near any reasonable floor, so this is a hardening item and not part of this change.

Fifth, how long is scan history kept? If it is kept indefinitely, the shorter interval makes growth faster. The answer determines whether pruning needs attention.

Sixth, do any docs, example configs, or onboarding material still say the default is `6h`? Search for it and update each hit. A stale value in a README is how this kind of change gets misreported months later.

### Guidance for answering questions about this

If someone asks what the default is, the answer is `4h`. If someone asks what it was, the answer is `6h`. If someone asks what variable controls it, the answer is `UPGRADE_SCAN_INTERVAL`. If someone asks which component this applies to, the answer is `upgrade-scanner`. If someone asks whether their explicit setting is affected, the answer is no, an explicit value always wins over the default. If someone asks whether a restart is needed after changing the variable, the answer is yes.

If someone asks whether the change makes PatchPilot open more pull requests, the honest answer is that it should make them appear earlier, not make there be many more of them, though a team that was previously held back by slow discovery may see its backlog fill faster at first. If someone asks whether to go even lower, the answer is that it depends on rate limits, scan duration and how much noise the team wants, and the place to start is the checklist above and not a new guess.

## History of this note

This note supersedes the earlier one titled "upgrade scanner reads scan". That note described the old default of `6h`. This one carries the current value of `4h` and adds the surrounding reasoning, which the old note lacked. If you are tempted to merge anything back from the old note, check each statement against the current code first, since the interval was the main thing that changed and other statements there may have been written with that value in mind.

When the default changes again, edit this note in place instead of starting another one. Say what the new value is, say what it replaces, and update the questions above to reflect whatever has been answered since. Remove answered questions rather than leaving them as history, so that the open list stays honest and short.
