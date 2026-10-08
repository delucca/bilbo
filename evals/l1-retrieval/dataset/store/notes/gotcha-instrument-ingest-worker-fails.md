---
id: 01KN4JCWPWWCAG4JRR4E4J4THM
created: 2026-04-01T10:05-03:00
---

# instrument-ingest-worker fails on files an instrument is still writing

instrument-ingest-worker picks up instrument output files and hands them on to the rest of LabNotebook Sync. If it opens a file while the instrument is still writing it, the read fails with this exception:

```
System.IO.IOException: The process cannot access the file because it is being used by another process
```

That is the whole gotcha. The worker is not broken and the file is not corrupt. The instrument still holds the file, and the worker got there too early. Everything below is what I know about why it happens, how to recognise it, what not to do about it, and what to check before touching the code. It is written fast, from what I remember and from what the team keeps running into, so treat the details as general unless a section says otherwise.

Naming first, because it trips people up. instrument-ingest-worker used to be called `tapline`. Old logs, old dashboards, old queue bindings, old runbooks, old chat threads and some deployment scripts still say `tapline`. It is the same component. If you search for `tapline` and find hits, those are instrument-ingest-worker under its previous name. If you are writing anything new, call it `instrument-ingest-worker`.

## Symptom

What you see is the exception text above, raised from the worker when it tries to open an instrument output file for reading. It shows up in the worker log, usually right after the worker logs that it noticed a new file or received a message about one. The message is nonspecific about who holds the file. Windows reports it the same way whether the holder is the instrument software, an antivirus scanner, a backup agent or another copy of the worker.

Things that point to the instrument as the cause, and not something else:

- The failure happens for files from one particular instrument or instrument type, and other instruments are fine.
- The failure happens close to the time the run is finishing, not long after.
- The same file opens fine a little later, by hand or on a retry.
- Large outputs fail more often than small ones, because a large output takes longer to write.
- Instruments that stream results to a single file during a run fail more than instruments that write the file once at the end.

Things that do not point to the instrument: a different exception type, a permission error, a path-not-found error, or a failure that repeats forever on the same file after the instrument is known to be finished. Those are other problems. Do not paste the exception text into a ticket and assume it is this one without checking the timing.

The exception is an `IOException`, so a broad catch for I/O errors will swallow it along with the real failures. Keep that in mind when reading the code.

## Why it happens

Instruments, or the vendor software that drives them, create the output file at the start of a run and keep it open for writing until the run is done. Many of them open it with a sharing mode that does not let other processes read it. Some allow reads but not while a flush is in progress. The file therefore exists, with a name and a size and a modification time, well before it is complete.

The worker finds files by watching a location and reacting to what appears there. A file appearing is not the same as a file being finished. The creation event fires when the instrument starts writing. Change events fire repeatedly while it writes. If the worker treats the first event as a signal that the file is ready, it opens the file while the instrument still has it, and the open fails.

There is a second flavour of the same problem. The file is finished from the instrument's point of view, but the vendor software is doing post-processing or copying, and still holds a handle for a short time after the last write. The worker sees a quiet file, assumes it is done, and still fails to open it. So a quiet file is also not a reliable sign that the file is released.

A third flavour: the instrument writes to a temporary name and then renames. Usually that is the friendly case, since the final name appears only when done. But it is not universal, and the worker cannot assume it for every instrument.

None of this is a bug in .NET. File sharing rules on the operating system decide it. The C# code just surfaces the failure.

## What the worker does with the file

In broad terms: a file shows up, the worker is told about it, it opens the file, reads the content, parses it into the shape the notebook side needs, stores the raw output in Azure Blob Storage, records what happened in SQL Server, and publishes a message on RabbitMQ so that the notebook entry can be linked to the data. The audit trail depends on the raw file being captured as it was produced, so reading a half-written file is worse than failing: it would record incomplete data as if it were the real run.

That is why the failure matters. The exception is annoying but it is also protective. It stops the worker from ingesting a partial file. Any fix that makes the error go away by reading anyway, for example by opening the file with a permissive sharing mode, trades a loud failure for a quiet data integrity problem. Compliance officers care about exactly that difference.

If the exception propagates and the message is not handled, the outcome depends on how the consumer is set up. Depending on acknowledgement behaviour, the message may be redelivered at once, dead-lettered, or lost. Check what the current behaviour actually is before relying on it. I have not verified it for every code path, and it has changed over time.

## What not to do

Short list of tempting fixes that are wrong or risky:

- Do not open the file with a sharing mode that tolerates writers and then read whatever is there. It hides the symptom and can ingest a partial file, which breaks the audit trail guarantee.
- Do not swallow the exception and move on. The file would never be ingested and nobody would know. A run would silently be missing from its notebook entry.
- Do not add a single fixed sleep before the first read and call it solved. It works on a fast instrument on a good day and fails on a slow one. Run length and file size vary a lot, and a sleep long enough for the worst case makes everything else slow.
- Do not retry in a tight loop. It burns CPU, floods the log and can interfere with the instrument if the instrument's own software is sensitive to contention on the file.
- Do not copy the file first and read the copy. Copying needs the same read access, so it fails in the same way, and if it does succeed on a half-written file you get a partial copy with a clean-looking name.
- Do not move or rename the file from the worker side to claim it. The instrument software may expect the file at its original name and fail the run, or write again to a new file.
- Do not assume file size stable for a moment means finished. See the second flavour above.

## What to do instead

The approach that fits is to treat "the instrument is done with this file" as a condition to be established, not assumed, and to retry the open with backoff until it is true or a limit is reached.

Concretely, the pieces I would want:

- Open the file for reading in the default restrictive way, the way that fails if anyone else holds it for writing. That failure is the signal that the file is not ready. It is the right check, and it is what already happens. The mistake is only in how the failure is handled.
- On that specific exception, wait and try again. Back off, with some jitter, and cap the total wait. The cap should be generous enough for the longest realistic run of the slowest instrument, and it should be configurable per instrument type, not one global number.
- Separate that case from other I/O failures. Match on the exception and its message, or better on the underlying error code, so that a missing file or a permissions problem is not retried for a long time as if it were a lock.
- After the cap, give up in a visible way: log it at a level that alerts, mark the file as not ingested in SQL Server, and let the message go to a dead-letter path where a person can find it. Never drop it.
- Require stability as well as openability. Once the open succeeds, check that the size and modification time have not moved across a short interval before trusting the content, to catch instruments that release and reacquire the file.
- Where an instrument offers a completion marker, such as a sidecar file, a rename at the end, or a status message, prefer it over guessing. It is the only reliable signal, and it should be configured per instrument.

The ordering matters: the open-with-retry is the base layer and works for every instrument. Markers are an improvement for the ones that have them.

## Where to look in the code

Without giving paths I cannot confirm: look at the part of instrument-ingest-worker that handles a new-file notification and the part that first opens the file. The bug lives at the boundary between them. Check these things in turn:

- Whether the notification is raised on create, on change, or after some delay.
- Whether there is any debouncing, and whether it is based on time since the last event.
- Whether the first open is wrapped in a try block, what it catches, and what it does after.
- Whether the retry logic, if any, is in the worker or left to the message broker redelivery.
- Whether the same file can be processed twice at once, for instance by two events for the same file, which would produce this exact exception for the second reader. This is a case where the holder is another instance of the worker, not the instrument. The symptom looks the same.

Because the project was `tapline` before, older code, comments and configuration keys may use that name. Search for both names when tracing behaviour. A config key with the old name can still be the live one. Do not rename keys without checking how deployed configuration refers to them.

## How to reproduce

Reproduction is easy and does not need a real instrument. Any process that opens a file for writing, without sharing, and keeps it open while writing slowly will do. Point the worker at the location, start the slow writer, and watch for the exception. Then let the writer finish and confirm that the file opens fine afterwards.

A useful test shape for the retry fix, in C# with whatever test framework the solution already uses:

- A fake writer that holds the file for a controlled time and then releases it. The worker should fail the first attempts, succeed after release, and ingest the full content.
- A fake writer that never releases. The worker should stop at its cap, report the file as not ingested, and not lose the message.
- A writer that releases, then reacquires. The worker should not ingest until the file is stable.
- A missing file. The worker should fail fast and not sit in the lock retry path.

Make the timing in these tests injectable so they run fast. A test suite that sleeps for real is a test suite people stop running.

Do the reproduction on the same operating system as production. File sharing behaviour differs between platforms, and a Linux run can pass where Windows would fail, because the rules on conflicting opens are not the same. The instruments in practice sit on Windows machines, and the worker may run elsewhere and read through a share, which adds its own timing and caching effects. A network share can make a file look released on one side and held on the other for a moment.

## Operational notes

For whoever is on call, and for compliance officers asking why a run is not in the notebook:

- If you see the exception once for a file and then a normal ingest message for the same file, nothing is wrong. That was a retry doing its job, or a redelivery.
- If you see it over and over for one file and the instrument is known to be finished, look for another holder: antivirus, backup or indexing software scanning the location, or a stuck instrument process that never closed its handle. Closing the instrument's software session often releases it.
- If it appears for many files across instruments at the same time, suspect the share or the storage, not the instruments.
- A file that never ingested is an audit gap. Do not close the ticket until the file is ingested or its absence is explained and recorded. The audit trail is the product here.
- Do not hand-copy a file into Azure Blob Storage or hand-insert rows in SQL Server to patch a gap. Re-run ingest through the worker so that the audit record is created by the normal path and has the correct provenance.
- Messages sitting in a dead-letter queue on RabbitMQ for this reason are safe to replay once the instrument is finished and the file opens. Replaying earlier just fails again.

When talking to someone who knows the old name, the same advice applies: `tapline` and instrument-ingest-worker are one thing, and the file-in-use failure is the same failure under both names.

## Open questions

Things I have not pinned down and that a later session should check before changing behaviour:

- What the real maximum write time is for each instrument type, which sets the cap on waiting. I do not have trustworthy numbers, so I have left them out on purpose.
- Which instruments give a completion marker, and of what kind.
- Whether the current acknowledgement behaviour on failure loses messages. If it can, that is a bigger problem than the exception and should be handled first.
- Whether duplicate notifications for the same file cause the worker to race itself.
- Whether any deployed configuration still uses names from the `tapline` era in a way that would break on cleanup.
- Whether reads over a network share need a longer stability window than local reads.

## Summary of the gotcha

When an instrument is still writing its output file, instrument-ingest-worker fails with `System.IO.IOException: The process cannot access the file because it is being used by another process`. The right response is to wait and retry with a limit, check that the file is stable, and surface a permanent failure loudly, never to read the file through a permissive open and never to ignore the error. The component was previously called `tapline`, and it is called `instrument-ingest-worker` now, so expect old material to use the old name.
