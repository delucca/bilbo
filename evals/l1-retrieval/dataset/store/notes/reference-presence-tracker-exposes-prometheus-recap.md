---
id: 01KX9TFVCJ9QCBJ21NM09YG8KR
created: 2026-07-11T21:09-03:00
---

# presence-tracker exposes Prometheus metrics (loose notes)

These are rough notes on how presence-tracker shows up in Prometheus. I wrote them from memory of reading the service and from what the on-call people usually ask, so treat it as partial. Where I would normally paste a value I say "the usual value" or "the configured limit" and you should look it up in the config rather than trust me.

The short version: presence-tracker is the Elixir service that knows who is connected to which event room over WebSockets. Phoenix channels report joins and leaves to it, it keeps a per-room view of attendees, and it merges that view across nodes. It exposes a metrics endpoint that Prometheus scrapes on the usual interval. The endpoint is served by the same release as the rest of the service but on a separate listener from the public socket traffic, so it is not reachable from the browser side of the event. Producers and community managers never see it. Only the platform people and the dashboards do.

## What the endpoint is and how it is wired

The metrics are produced through the standard Elixir telemetry route. Phoenix, the Ecto layer talking to CockroachDB, and the presence code itself all emit telemetry events. A reporter process subscribes to those events and turns them into Prometheus text format when the scraper asks. There is no push gateway involved. Everything is pull.

The reporter is started in the supervision tree of presence-tracker, early, before the channel endpoint comes up. That ordering matters because if the reporter starts late you lose the first joins after a deploy, and the counters look like they started from nothing. It is not a bug, just a gap, but people have misread it as a dip in attendance.

The listener for scraping is configured through the runtime config, with the bind address and port coming from environment variables at boot. I am deliberately not writing the port here. Look at the runtime config or the deployment manifest. The scrape path is the conventional one for Prometheus exporters, and the scrape job in the monitoring repo points at the service by its internal name, not by node address. Because there are several nodes behind that name, the job uses service discovery so each node is scraped separately. If you scrape through a load balancer you get a random node each time and the numbers jump around. That is the single most common reason a panel looks broken.

Authentication on the endpoint is network level. The listener is only exposed inside the cluster network and there is no token on it. If someone proposes adding one, check that the scraper config supports it first, because the scrape job would need credentials added at the same time.

### Labels and cardinality

The metrics carry a small set of labels. The ones that matter are the node, the event tier (large, standard, rehearsal, roughly), and in a few places the channel topic type, meaning whether the room is the main stage, the Q&A room, the poll room, or the moderation room. We do not label by event id and we do not label by user id. This was a decision, not an oversight. With large virtual events there can be many simultaneous events and attendee churn is high, so putting the event id on every series would blow up the number of series in Prometheus. If you need per-event numbers, the answer is the admin tooling that reads from the database, or a log query, not a new label.

If you are tempted to add a label, ask first what the worst-case number of distinct values is. Anything that grows with attendees or events is out. Anything bounded by a handful of fixed categories is fine.

### Metric families, described rather than listed

I am not going to list exact metric names here, because they have been renamed once already and I would rather you read the reporter module than trust my memory. The families are these.

Connection and presence gauges. There is a gauge for how many sockets the node currently holds, a gauge for how many distinct presence entries it tracks, and a gauge for how many rooms are active on the node. The presence entries gauge is not equal to the sockets gauge: one person with two tabs open is two sockets but should be one presence entry, depending on how the key is chosen. When these two drift apart a lot, it usually means the key is wrong for some client or that a client is reconnecting and leaving ghosts behind.

Join and leave counters. Monotonic counters for joins and for leaves, split by the room type label. The difference between them over a long window should roughly track the gauge. If joins minus leaves keeps climbing while the gauge stays flat, something is counting leaves in a different place than joins, which has happened before after a refactor of the terminate callback.

Merge and sync timings. Histograms for how long the cross-node merge of presence state takes, and for how long a full state sync to a newly joined node takes. These are the ones that tell you the tracker is struggling before users notice. The buckets are the defaults the reporter uses with a few extra at the high end, because a merge during a big event can be slow.

Broadcast and fan-out timings. How long it takes to push a presence diff to the subscribers of a room. Again a histogram. This is affected by room size more than anything else, which is why the room type label helps: the main stage room is the largest and will always look worse than the poll room.

Queue and mailbox gauges. A gauge reading the message queue length of the main tracker processes. This one is sampled periodically rather than event-driven, so it is a snapshot and can miss a short spike. When the queue gauge starts staying above the configured warning level for more than a few scrapes, that is the early sign of overload.

Database related. Because presence summaries and some audit information are written to CockroachDB, there are the usual query timing and pool metrics from the Ecto telemetry. They are named by the library, not by us. Retries from the database side show up as their own counter, which matters because CockroachDB can ask the client to retry a transaction under contention, and a rise in that counter during a big event is expected up to a point.

VM metrics. Memory, run queue, scheduler utilization, and process counts from the BEAM, through the standard VM measurements. These are what you check first when the tracker feels slow, before looking at anything presence specific.

## How to read it during a live event

Producers care about a handful of things: how many people are in the room, whether the number is moving sensibly, and whether moderation is keeping up. The metrics from presence-tracker answer the first two. The third lives in a different component, so do not go looking for moderation latency here.

For the attendee count, use the presence entries gauge summed over nodes. Do not use the socket gauge as the headline number unless you also mean to count duplicates. The dashboard panel for the headline number is built from the presence entries gauge for that reason. If someone says the dashboard number disagrees with the number in the producer UI, check first whether the UI is reading from a cached summary that is refreshed on an interval, because the UI number lags by design.

For the movement, use a rate over join and leave counters with a window that is several times the scrape interval. A window that is too short gives empty gaps. The usual guidance for rate in Prometheus is at least a few scrape intervals, and I just follow that. When an event opens, expect a steep wave of joins, and when a poll closes or a keynote ends, expect a wave of leaves. These waves are normal and should not page anyone.

### What a bad pattern looks like

A few shapes I have learned to recognize.

A sawtooth in the presence entries gauge on one node only. That tends to mean a node is repeatedly losing and regaining its cluster membership, and each time it re-syncs, so the entries drop and refill. Check the sync timing histogram and the cluster membership logs for that node.

Presence entries gauge much higher than the socket gauge. Should not happen, since entries are derived from sockets. If it does, there are stale entries that were not removed after a crash of a channel process or after a netsplit. The tracker has a cleanup path for that, with a grace period before it drops entries from a node it believes is gone. If the gauge stays high past the grace period, the cleanup is not running.

Merge timing creeping upward across an event while attendee count is flat. That points at a leak, usually in the amount of metadata attached to each presence entry. People have added fields to the presence payload that look harmless and then multiplied by every attendee. Keep the payload small. The tracker diffing cost scales with the size of the metadata, not just the number of entries.

Flat line in everything from one node. Either the node is down or the scrape for it is broken. Compare against the up series for that job before deciding that nothing is happening.

Counters resetting to zero. After a deploy or a restart, counters reset and Prometheus handles that in rate calculations. But a gauge reset can look like a sudden drop in attendance. During a rolling deploy in the middle of an event the sum drops for a moment and then recovers when sockets reconnect to the new nodes. Avoid deploying during a large event when you can; if you must, tell the producers that the number will wobble.

### Alerts

Alert rules live in the monitoring repo, not in this service. The ones related to presence-tracker are about: the tracker being unreachable by the scraper, the mailbox of the main tracker process staying high, the merge timing being above the configured limit for a sustained period, and the gap between sockets and entries growing too large. The thresholds are set by event tier in a few cases, since a large event is allowed more latency than a rehearsal before it counts as bad, or the other way round depending on who you ask. I do not remember the exact thresholds and I do not want to state a stale one. Read the rule files.

Alerts should route to the platform on-call during events, and to the normal queue otherwise. The event calendar drives a silence or an escalation, I believe, but I did not check how that is implemented.

## Gotchas I have seen or heard about

The scraper hits the endpoint on the usual interval, and building the response is not free. The reporter reads the sampled gauges at scrape time, and some of those samples, such as the mailbox length and the room count, require calling into processes. If the main tracker process is overloaded, the sampling call itself can wait, and the scrape gets slow or times out. So the metrics endpoint is least reliable exactly when you most need it. The reporter is supposed to use a short timeout for those calls and report a missing value rather than blocking, but verify that if you change the sampling code. A missing value is a better outcome than a hung scrape.

Another one: the telemetry handlers run in the process that emits the event. A slow handler therefore slows the channel process that triggered it. Keep handlers tiny: increment a counter, observe a histogram, nothing else. Do not log from handlers at a high rate and do not do lookups there. A previous attempt to enrich events with room metadata inside the handler made joins noticeably slower under load and was reverted.

Histogram buckets are fixed at startup. If the real latencies fall outside the buckets, the quantile estimates are junk, since everything lands in the first or last bucket. When the traffic pattern changes, for example when larger events become common, revisit the bucket boundaries. Changing them creates new series and breaks continuity on graphs, so do it deliberately and tell whoever owns the dashboards.

Naming and units. The reporter follows the Prometheus convention of base units, so durations are in seconds and not milliseconds, even though the Elixir telemetry events carry native time units that need conversion. This conversion is done in the reporter definitions. If you add a new timing metric, declare the unit conversion explicitly, or you will get values that are off by a large factor and nobody will notice for a while because the shape of the graph is right.

Rolling restarts and the node label. The node label contains the node name, and node names can change on redeploy if they include an address or a generated suffix. Over time that makes extra series appear and old ones go stale. It is bounded by the number of deploys within the retention window, so it is acceptable, but it makes per-node graphs messy. Aggregate with sum by the tier or room type unless you are debugging one node.

Test and staging environments expose the same endpoint, with the same metrics, but traffic is tiny and synthetic, so histograms are mostly empty. Do not tune alert thresholds on staging data.

The WebSocket layer and the metrics endpoint share the BEAM, so if the VM is starved the scrape suffers like everything else. Do not treat a failed scrape as proof that the service is down. Check the user-facing health check as well.

Presence is eventually consistent across nodes. Two nodes can report different entry counts for the same room for a short moment. If you sum per node gauges you may double count entries that exist in both views during a merge, depending on how the gauge is defined. As I understand it the gauge counts entries owned by the local node, which avoids the double count, but I am not certain it is consistent for every room type. If the totals look slightly too high during churn, that is the first suspect.

### Things not covered

I have not documented the exact set of dashboards. There is one for the event overview, one for the tracker internals, and I think one that producers can look at, which is fed by a different path and not by Prometheus directly. Ask the dashboard owner.

I have not covered the recording rules. A few heavy queries, such as the summed attendee count per tier, are precomputed to keep the dashboards fast. If a panel shows a number that differs from the raw query, check whether it reads the recorded series, which has a slightly different freshness.

I have not covered retention or remote write. That belongs to the monitoring setup and is configured outside this project.

## Changing or extending the metrics

When adding a metric to presence-tracker, the rough process I follow is this. First decide whether it needs to exist at all: can an existing metric, plus a rate or a ratio, answer the question? Then decide the type. Counters for things that happen, gauges for things that are, histograms for durations and sizes. Then pick labels from the small fixed set, and refuse any label with unbounded values. Then add the telemetry event at the point in the presence code where the thing happens, keep the measurement a number and the metadata a few small atoms or strings, and add the definition in the reporter module next to the others, with a unit and a short description. Then run the service locally, hit the endpoint, and confirm the new series appears with sensible values under a fake load. Finally tell the people who own the dashboards and alerts, because a new metric that nobody looks at is wasted and a renamed one silently breaks panels.

Do not rename existing metrics casually. Prometheus has no notion of an alias, so a rename means a gap in history and broken queries. If a rename is truly needed, emit both names for a transition period, move the dashboards and alerts, then drop the old one. That is the approach used last time and it worked fine, apart from the extra series for a while.

For tests, the project has a few checks that the reporter starts and that the endpoint answers with the text format, but nothing that validates the metric names or labels. A cheap improvement would be a test that scrapes the endpoint in the test environment and asserts that no label has an event or user identifier in it. That would catch the cardinality mistake before it ships. I have not written it.

### Open questions

Is the gap between sockets and presence entries something we want an alert on at all, or is it too noisy because of multi-tab users? I lean towards alerting on the change in the gap rather than the gap, but that has not been tried.

Should the sampled gauges move to event-driven updates so the scrape does not need to call into the tracker processes? It would remove the scrape-time risk described above, at the price of more work in the hot path. The hot path is already sensitive, so I would want measurements first.

Should the metrics listener get its own small pool of resources so a flood of public socket connections cannot starve it? Right now it shares the VM and a flood competes with it. Probably fine for now, worth revisiting if scrapes start failing during the largest events.

Is per-event visibility something the producers actually ask for? If yes, the right design is likely a separate path that aggregates per event in memory and exposes it through the admin API with an expiry, not through Prometheus labels. I would not build that without a real request.

Last, the existing notes and the code might disagree with what I wrote here on details, since I wrote this without rereading them side by side. If something here conflicts with the reporter module or the runtime config, the code wins, and this note should be fixed or merged into the other reference on the same subject.
