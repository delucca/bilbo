---
id: 01KJ51RPB3T5D55QD8BGAF6Y4R
created: 2026-02-23T07:47-03:00
---

# keyvane-etcd-store weekly recap

This is the recap for the week on keyvane-etcd-store. It is written fast and from memory, so treat it as a trail of what got touched and what still bothers me, not as a spec. Nothing here fixes a value for anything. Where a number would normally go, I say what kind of thing it is and leave it there. The real settings live in config and in the review threads.

The week had four threads: lease and rotation bookkeeping on top of etcd, the watch path that feeds rotation workers, trust and identity on the client connections, and test and operational noise. Most of the time went into the second and third. The first one made progress but is not done. The fourth ate more hours than I wanted.

## Where the week went

Roughly in order of time spent:

- Watch handling in keyvane-etcd-store, mostly reconnect behavior and what the consumer sees when the stream drops and comes back.
- Identity on the etcd client side, meaning how SPIFFE-issued certificates get loaded and reloaded, and what happens to open connections when they change.
- Lease bookkeeping for short-lived credentials, including how we record that a credential was handed out and when it should be considered dead.
- Flaky tests, plus one long afternoon chasing something that turned out to be a test fixture problem and not a store problem.
- Reading and replying to review comments from the security engineers on the rotation changes.

I did not touch the Vault integration directly this week. It came up because the store sits underneath the code that talks to Vault, and a couple of the problems I hit looked like Vault problems from the outside. They were not. More on that below.

The component still does the same basic job. It holds the state that lets Keyvane rotate secrets without a redeploy: which credentials exist, who they were issued to, what generation they belong to, and what the rotation workers need to do next. etcd is the source of truth for that. Nothing in the store is meant to hold secret material itself, only references and metadata, and I kept checking that this stayed true while editing. It did.

## Watch path and rotation workers

This was the biggest piece. The rotation workers learn about work by watching key ranges in etcd. When the watch is healthy this is boring. When it is not, the failure modes are quiet, and quiet failures in a rotation system are the bad kind: a credential just does not get rotated and nobody notices until someone asks why a service is still using an old one.

What I found early in the week is that our reconnect handling treated a dropped watch and a compacted-away revision as the same event. They are not. A dropped stream that can resume from where it left off is cheap. A resume point that history no longer covers means the worker has to re-list the range and reconcile, because it may have missed changes. The old code path resumed blindly in both cases and in the second case would have skipped events without saying so. I do not have evidence that this bit us in production, and I want to be clear about that. I found it by reading the code and then forcing the situation in a local test cluster, where it reproduced.

What I changed, in words:

- The watch wrapper now distinguishes the case where history is gone and surfaces it as its own condition instead of folding it into a generic retry.
- On that condition the consumer does a full list of the relevant range, rebuilds its in-memory view, and only then starts a new watch from the revision the list returned. This ordering matters. If you start the watch first and list second you can double-apply or reorder, and if you list first and watch from the wrong point you can miss a change in between. I went back and forth on this and I am fairly sure the list-then-watch-from-list-revision ordering is the right one, but I asked for a second pair of eyes because it is the sort of thing that is easy to get subtly wrong.
- The reconnect backoff is separated from the reconcile step so a slow re-list does not get multiplied by the backoff.

Things I am not happy about yet:

- The reconcile step is correct but heavy for a large range. For the sizes we deal with today it is fine. If key counts grow a lot it will need pagination or a smarter diff. I left a note in the code and did not try to solve it now.
- There is still a window where two workers can both believe they own the same pending rotation, right after a reconcile. The lease on the work item is supposed to prevent that, and in tests it does, but the reasoning relies on clock behavior on the worker side in one place. I would rather it relied only on etcd revisions. This is on the list for next week.
- Logging on the watch path is either too chatty during a flap or silent in the one case where I most want a line. I started rebalancing it and did not finish. The compaction case now logs once per occurrence with enough context to find the range, and nothing about secret contents, which is the rule here anyway.

One thing worth recording for whoever picks this up: when I forced the compaction scenario locally, the first few attempts did not reproduce because the cluster had automatic compaction that never kicked in during a short test. I had to trigger compaction explicitly in the harness. If someone else tries to reproduce this and sees nothing, that is probably why, and it does not mean the bug is absent.

The workers also had a habit of acknowledging work before the state change was durable in etcd. In the normal flow the order is fine because the write happens right away. Under a slow cluster, though, the acknowledgement could be seen by a caller before the write committed. I moved the acknowledgement to after the commit returns. This makes the worker slightly slower on the happy path and I think that is the right trade. Nobody has pushed back, but it is only in review and has not been run under load.

## Identity, SPIFFE, and mTLS on the client side

The store talks to etcd over mTLS, and the client identity comes from SPIFFE. The workload gets a short-lived certificate from the local agent and presents it to etcd. That much was already there. What I spent time on is what happens when the certificate rotates underneath an open connection.

The situation: the SPIFFE material is short-lived by design, so it rotates often relative to the life of the process. Go's TLS stack lets you supply the client certificate through a callback at handshake time, which means new connections pick up the new certificate, but connections that already exist keep the old one until they are torn down. For etcd that matters in two ways. First, the server may enforce the validity of the peer certificate on new handshakes only, so an old connection can quietly outlive the certificate it was opened with. Second, long-lived watch streams are exactly the connections that live the longest, so they are the ones most likely to be holding a stale identity.

What I did this week:

- Confirmed by reading and by a local experiment that existing streams do not renegotiate when the source of the certificate changes. This is expected behavior, but I wanted to see it and not assume it.
- Added a periodic recycling of long-lived client connections so that identity on the wire does not drift too far from identity in the agent. The interval is a config knob and I deliberately did not hard-code a value in the change. The recycling goes through the same watch-resume logic described above, so it depends on that work being correct. That is one more reason to get the watch path right first.
- Made the certificate loader fail loudly when the material it gets back is incomplete, for example a chain with no leaf or a key that does not match. Before, it could end up holding a half-updated pair for a moment. Now it keeps the last good pair until the new one validates as a whole.

There was also a trust bundle question. The client verifies the etcd server against a bundle, and that bundle also changes over time. I checked that the bundle reload path and the certificate reload path are independent, so a failure to refresh one does not block the other. They are independent in the code. They are not independent in the sense that a bad bundle will still break new connections, which is correct, and I did not want to paper over it.

Questions I raised and did not settle:

- Should the store itself check that the SPIFFE ID of the etcd server matches what it expects, in addition to normal chain verification? Right now we rely on the chain and on the server name. A SPIFFE-aware check would be stricter and fits the rest of the system. It would also add one more thing that can fail during a rotation event. I have an opinion, which is to do it, but this belongs with the security engineers and I put it in the review thread as a question and not as a change.
- Whether a rotation of the client certificate should trigger an immediate recycle of connections, or just the periodic one. Immediate is tidier. It is also a thundering-herd risk if many instances rotate together. I leaned toward periodic with jitter for now and said so.

One gotcha for future me: while testing I kept getting handshake failures that looked like trust problems and were actually clock skew between my local harness containers. The certificates are short-lived, so a small amount of skew shows up fast. Check time first before touching trust config.

## Lease and credential bookkeeping

This is the less dramatic thread, and it moved slower. The store records, for each issued credential, enough to answer three questions later: who has it, which generation of the secret it came from, and when it stops being valid. Rotation workers and the revocation path both read that.

This week I looked at how the records tie to etcd leases. The design intent is that when a credential's lease expires, the record expires with it, so there is no separate cleanup job that can fall behind. In practice there are two kinds of keys: ones attached to a lease and ones that are not, because they describe longer-lived things like the generation history. The line between those two was drawn by habit and not written down anywhere. I wrote it down, in the package docs, in plain words. I did not change behavior.

While doing that I noticed:

- One write path attaches the lease after the key is created in a separate step. If the process dies between the two steps, you get a key that never expires. It is a small window, and the fix is to do both in one transaction. I made that change. It is small and I think it is clearly right, but it touches a hot path, so it is in review and not merged.
- Another path refreshes a lease and then reads the record to decide what to do, and the read can see state from before the refresh in the rare case where a different writer got in. I did not change this. I wrote a test that describes the intended behavior and marked it as expected to fail for now, with a comment pointing to the discussion. I would rather have the failing description in the repository than only in my head.
- The code that computes whether a credential is still valid uses a mix of the lease's own notion of time and the local clock. I would like it to use one source. This is the same clock concern as in the watch section, and I suspect the fix is shared. I did not start it.

On generations: the store keeps a history of secret generations so a rollback is possible and so in-flight users of the previous generation are not cut off the instant a new one appears. The overlap behavior is a policy, owned by the rotation layer and not by this component, and I stayed out of it. What I did check is that the store never garbage-collects a generation that something still references. That invariant holds in the code I read, and there is a test for it. I added a second test for the case where a reference is added concurrently with a cleanup pass. It passes, though concurrency tests of this shape can pass by luck, so I ran it in a loop for a while and did not see a failure. That is weaker than a proof and I am recording it that way.

The Vault boundary showed up here. Some credentials are dynamic secrets that Vault issues and the store only tracks. When a record in the store and a lease in Vault disagree, the question is which one wins. Today the answer is that Vault is authoritative for whether a credential is valid and the store is authoritative for who was given it. I checked that the code follows that split. I did not find a place where it crosses it, though one helper has a name that suggests it does, and I left a comment rather than a rename because renames make review harder this week.

## Tests, flakiness, and operational noise

A fair amount of time went to things that are not the store being wrong.

The test suite uses an embedded etcd for most tests and a real multi-member cluster for a smaller set. The embedded one is fast and has been stable. The cluster ones are where flakiness lives. Two of them failed intermittently this week. One was a real timing assumption: the test waited a fixed interval for a leader election and sometimes the election took longer on a loaded machine. I replaced the fixed wait with a wait on the condition itself, bounded by a generous overall limit. The other turned out to be a fixture that reused a data directory between runs when a previous run had been interrupted, so it started with leftover state. I made the fixture create a fresh directory every time and clean up on exit. This was the afternoon I lost. The symptom looked like a watch bug, which is why it cost so much. It was not one.

What I would like next, and have not done: a small helper that dumps the relevant cluster state when a cluster test fails, so the next person does not have to guess. Even a few lines about membership, leader, and revision would have saved me real time.

Other operational notes:

- Metrics for the store are thin. We can see request outcomes, but not how often the watch path takes the reconcile branch. Given the work this week, that count is exactly what I would want on a dashboard, as a signal that something upstream is compacting too aggressively or that consumers are falling behind. I sketched the metric and did not wire it in.
- Alerting for a rotation that did not happen is the real gap. A stuck watch and a healthy idle one look the same from outside. Something like a heartbeat key that the worker expects to see change would turn silence into a signal. I wrote this idea down for discussion. I am not proposing it as a decision, and it has costs, mainly extra writes to etcd and one more thing to explain to the application teams.
- Dependency updates for the etcd client library and the Go toolchain are due again. I did not do them this week because I did not want to mix them with the watch changes. If the watch changes land, the update goes right after, so any behavior shift in the client library is not confused with my edits.
- One review comment asked whether the store should expose a read-only mode for use during maintenance on the cluster. It is a reasonable idea. It touches the API surface, so it needs a proper design pass and not a drive-by patch. I said I would write it up and have not.

Docs and hygiene: I fixed a few misleading comments in the watch code that described the old behavior. Stale comments in this part of the code are dangerous because they make the wrong ordering look intentional. I also deleted a small unused helper that nobody called and nobody could explain. Version control still has it if someone misses it.

## Open items and what I would do next

Things I would pick up first, in rough priority:

- Get the watch and reconcile changes through review and merged. Everything else this week leans on them: the connection recycling, the heartbeat idea, and the metric. If review turns up a flaw in the list-then-watch ordering, the rest has to wait, and I would prefer to learn that early.
- Look hard at the double-ownership window after a reconcile and make it depend only on etcd revisions, not on worker clocks. Same for the validity calculation in the lease code. I think these are one fix seen from two sides, but I have not proven it.
- Run the new watch behavior under realistic load in a cluster with real compaction, not just the forced one. So far all my evidence is from small local runs, and I should not talk about it as if it were more than that.
- Get an answer from the security engineers on stricter server identity checking and on immediate versus periodic connection recycling. I do not want to guess at either, since both are about trust boundaries and not about code style.
- Add the failure dump helper to the cluster tests. Cheap, and it pays back the first time anything goes wrong.
- Wire in the reconcile metric and think about whether a heartbeat is worth its cost.

Things I am deliberately not doing yet:

- Pagination or a smarter diff in the reconcile step. Not needed until the key volume says so.
- A read-only maintenance mode. Needs design first.
- Any change to the generation overlap policy. That is not this component's call.

Risks I want to keep in view. The first is that the quiet failure mode of the watch path is still the main danger in this component. The fixes this week narrow it, but the lack of an external signal means we could still miss a stall. The second is that identity rotation and watch resumption are now coupled by the recycling change, so a bug in one can look like a bug in the other. When something odd shows up, check both. The third is that several of my conclusions rest on local experiments with small data and short-lived clusters. They are good enough to justify the changes and not good enough to claim the problem is gone.

If you are reading this cold and need a starting point: read the watch wrapper in keyvane-etcd-store first, then the connection setup code, then the lease attachment code. That order follows the dependencies, and it is the order I wish I had read them in at the start of the week. Ask before touching the Vault boundary helper with the confusing name, and check clocks and fixture directories before suspecting the store when a cluster test misbehaves.
