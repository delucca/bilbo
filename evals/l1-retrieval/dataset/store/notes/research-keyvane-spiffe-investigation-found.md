---
id: 01KEVBCNTNAPYMEVDDKGSVE1V7
created: 2026-01-13T06:36-03:00
---

# keyvane-spiffe-bridge: streaming vs polling for X.509 SVIDs

We looked at how keyvane-spiffe-bridge should get X.509 SVIDs from the Workload API socket. The result is clear enough to act on: using the streaming FetchX509SVID call used 40 percent less CPU than polling every 5 seconds. Streaming is the better default. This note keeps the finding, how it was reached, what it does not prove, and what to check before changing anything else around it.

## Question

keyvane-spiffe-bridge sits between the SPIFFE Workload API and the parts of Keyvane that issue short-lived credentials. It needs a current SVID and trust bundle so it can present identity over mTLS to Vault and to other Keyvane services. The first version asked the Workload API for the SVID on a timer. Someone noticed the bridge was not idle on quiet nodes, and that its CPU use did not track how often identities actually changed. So the question was whether a streaming subscription would be cheaper than the timer, and whether it would cost us anything in correctness or recovery behaviour.

The two shapes being compared:

- Polling: a loop that wakes every 5 seconds, makes a fetch against the Workload API socket, parses the result, compares it with what it already holds, and goes back to sleep.
- Streaming: one long-lived FetchX509SVID call on the same socket. The agent pushes a new response when the SVID set or the bundle changes. The bridge blocks on the stream and does work only when a message arrives.

## Finding

With the streaming FetchX509SVID call, keyvane-spiffe-bridge used 40 percent less CPU than with polling every 5 seconds. Same node type, same workload set, same socket, same build apart from the fetch strategy.

The reason is not mysterious. Polling pays the full cost of a request, a response decode, certificate parsing and a comparison on every tick, and almost every tick finds nothing new. Streaming pays that cost only when the agent has something to say. Between rotations the streaming bridge is blocked in a read and does close to nothing. The saving is therefore mostly the removed no-op ticks, not a faster code path for real updates.

The first response on a stream still carries the full set, so the cost of a cold start is about the same either way. The gain shows up in steady state, which is where the bridge spends nearly all its time.

## How it was measured

We ran both variants against the same kind of workload on comparable nodes and compared the CPU time the bridge process consumed over a long steady-state window. Rotation was left at the normal cadence for the environment, so the streaming variant did receive real updates during the run and was not flattered by a quiet period. Each variant was run more than once, and the ordering was the same each time. The headline figure is the difference we saw consistently, not a best case.

Things we held constant on purpose:

- The Workload API agent version and its configuration.
- The number of workload identities the bridge asked about.
- Log level. Debug logging on every poll would have inflated the polling numbers, so it was off for both.
- Garbage collection settings for the Go runtime.

We looked at CPU only through process accounting. We did not do a profile-level breakdown of where the polling cycles went, so the explanation above is a reasoned one and not a measured one.

## Caveats

The 40 percent figure belongs to this setup. It depends on the poll interval: a longer interval would shrink the gap, a shorter one would widen it. It also depends on how often identities change. On a node where SVIDs rotate constantly, the stream delivers messages nearly as often as polling would fetch, and the saving gets smaller. Do not quote the number as a general property of SPIFFE or of Go clients.

We did not measure memory, latency of picking up a rotated SVID, or load on the agent side. The last one matters. A stream keeps a connection open per subscriber on the agent. With many bridges per node that could add up, though with one bridge per node we saw no sign of trouble. Pickup latency should be better with streaming, since the bridge learns of a change when it happens and not up to a full interval later, but we have not timed it, so treat that as expected and not shown.

Also note that the comparison was between two correct implementations. Streaming was not cheaper because it skipped work that polling did, such as validating the chain. Both variants validated the same way before handing the SVID on.

## Failure and recovery behaviour

Streaming has failure modes that polling hides. If the agent restarts, the stream ends and the bridge must notice, reconnect and take the first message as a fresh full state. If that reconnect loop is written badly, it can spin and burn more CPU than polling ever did. So the reconnect needs a backoff with a ceiling and some jitter, and it must reset the backoff only after a message is actually received, not just after the connection opens.

A stream can also stay open and go silent, for example if the agent is wedged. Polling would eventually fail a call and surface the problem. For streaming we rely on the fact that SVIDs carry an expiry: the bridge should treat an SVID nearing expiry with no update as a fault and re-establish the stream, and it should report that in its health output. Without that, a silent stream could let identity lapse unnoticed, and a lapsed identity breaks mTLS to Vault and then breaks credential issuance for everything downstream.

Another point: when a message arrives, the bridge replaces its held state as a whole. It should not merge. A removed identity must disappear, and a bundle with a dropped root must stop being trusted promptly.

## Decision and follow-ups

Use streaming FetchX509SVID as the default in keyvane-spiffe-bridge. Keep the polling path available behind a setting for now, as a fallback for environments where long-lived streams are cut by something between the bridge and the socket. Remove it once streaming has run in every environment without incident.

Follow-ups, in rough order of importance:

- Add the expiry-based watchdog and a health signal for a stream that has gone quiet.
- Put backoff with jitter on reconnect and test it by killing the agent repeatedly.
- Time how long a rotated SVID takes to reach the bridge under each strategy, so the latency expectation becomes a measurement.
- Check agent-side cost if we ever run several bridges on one node.
- Repeat the CPU comparison on a node with unusually frequent rotation, to see how far the saving shrinks.
- Add a profile of the polling loop so the explanation of the saving rests on data.

If someone reruns this comparison, keep the setup the same as listed above, or the numbers will not be comparable with the 40 percent figure recorded here.
