---
id: 01KVQKBB6KBC85CKBYFXPSBQR0
created: 2026-06-22T09:02-03:00
---

# export-worker: options survey

Notes on how the export-worker could be built. Nothing here is settled; it is a map of the options so the next session does not redo the survey. The job is simple to say: take the poll results and Q&A history of an event and hand the producer a file they can open elsewhere.

## What the export-worker has to cope with

Large virtual events produce a lot of rows, and the data keeps changing while moderation is still running. Exports are requested by event producers and community managers, usually right after an event, so load comes in bursts. The worker should not slow down live polls or the WebSocket side of the Phoenix app.

## Where the worker runs

Three shapes came up:

- Inside the main Phoenix app as a supervised process tree. Cheapest to build, shares deploys, but shares memory and schedulers with live traffic.
- A separate Elixir release that talks to the same database. Isolates load, costs one more thing to deploy and monitor.
- A job queue library with workers on dedicated nodes. Gives retries and visibility for free, adds a dependency and a jobs table to look after.

My lean is the job queue shape, but it needs a look at how the queue behaves on CockroachDB before anyone commits.

## Reading the data

CockroachDB is the source. Options are one big query, keyset pagination in chunks, or a changefeed into a staging area. Chunked reads with a consistent snapshot time look like the safest middle path. Long transactions on a distributed store can hit retry errors, so short reads are preferred. Changefeeds are probably too heavy for a once-per-event export.

```elixir
Repo.stream(query, max_rows: chunk_size)
```

That is the shape of streaming through Ecto; chunk size is left open.

## Output formats

CSV is what most producers want first. JSON suits people who feed other tools. Spreadsheet formats are friendly but heavy to generate for big events. Streaming writers matter more than the format, since building the whole file in memory will not hold up.

## Delivering the file

Options: write to object storage and give a time-limited link, stream straight to the browser through the Next.js front end, or email a link when done. Object storage plus a link fits long jobs best. Direct streaming ties up a connection and breaks when the user closes the tab.

## Open questions

- Should moderation-hidden items be included, flagged or dropped?
- Do producers need personal data of participants, and who may see it?
- How long are finished files kept?
- Does the export-worker report progress over the existing WebSocket channel, or just poll for status?

## Next steps

Try the queue option against CockroachDB with a realistic data set, measure memory with the streaming writer, and then write the choice down as a separate decision note.
