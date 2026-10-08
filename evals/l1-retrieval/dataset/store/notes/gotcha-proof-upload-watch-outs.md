---
id: 01KN3NW52GRN0DPQZCJP8QFWRH
created: 2026-04-01T01:47-03:00
---

# proof-upload-worker: things to watch when changing it

Notes for anyone touching proof-upload-worker. It is the piece that takes a proof of delivery captured on a driver's phone and gets it to the backend, usually long after the driver has left the doorstep and often with no signal. Most bugs here do not show up at a desk with good wifi. They show up in a van, in a basement car park, on a phone that was killed by the OS an hour ago. Read this before you "simplify" anything.

The short version: this component is the last line of defence for a courier's evidence. If a proof is lost, a driver can be blamed for a parcel they did deliver. If a proof is duplicated or overwritten, support sees a mess and nobody trusts the data. Both failure modes are quiet. Nothing crashes, the record is just wrong.

## Retries, idempotency and double uploads

The worker will run more than once for the same proof. Assume that. The scheduler can restart it after the process dies, after a constraint change, after a reboot, or after a result you thought was final. Network calls can succeed on the server and still look like failures on the phone, because the response never arrived. So every step has to be safe to run again.

Things to check whenever you change the upload path:

- The identity of a proof must come from the device at capture time, not from the worker run and not from the server. If you generate an id inside the upload step, a retry creates a second record. Keep the id stable from the moment the driver confirms the delivery.
- Writes to Cloud Firestore should be keyed by that stable id, so a repeat is an overwrite of the same content and not a new document. Do not switch to auto-generated document ids "for convenience". It looks harmless and it breaks dedupe.
- Be careful with any field that depends on when the write happened. A server timestamp set on each attempt will move forward on every retry and can make an old proof look recent. Capture time and upload time are different facts. Keep both, name them clearly, and never reuse one for the other.
- If the upload has several stages (media first, then the metadata record, then a status update), decide what a half-finished run looks like and make sure the next run can pick it up from the middle. Writing the metadata record before the media is safely stored means the backend can point to a photo that does not exist. Writing the media first and failing before the record means orphaned files. The second one is the lesser evil, but you still need a way to tell the stages apart on retry.
- Do not mark a proof as uploaded locally until the server has acknowledged the write that matters. "The call returned" is not the same as "the server has it". Check what the acknowledgement actually covers, especially when Firestore's own offline cache is in play, because a write that is accepted by the local cache has not reached the backend yet.
- Backoff matters. A tight retry loop on a bad connection drains the battery and gets the app throttled. A very long backoff leaves proofs sitting on the phone while the driver is already sure they are done. Look at who suffers before changing the policy.
- Separate failures that are worth retrying from those that are not. A dropped connection is worth retrying. A rejected payload, a permission problem or a malformed record will fail the same way forever. If the worker keeps retrying those, it blocks the queue behind it and burns resources. If it gives up too eagerly on something transient, it drops proofs. Classify errors on purpose and keep the classification in one place.
- Work must be able to stop at any moment. Cancellation and process death are normal. Do not hold state only in memory between stages, and do not assume a coroutine will reach its cleanup block.

A small reminder of what the pieces are, so nobody reaches for the wrong layer:

```
proof-upload-worker
  Kotlin + Android Jetpack   -> scheduling, constraints, local queue
  Protocol Buffers           -> payload shape on the wire
  Cloud Firestore / Firebase -> backend records and storage
```

## Offline behaviour and the local queue

Offline support is the whole reason this component exists, so treat every change as an offline change first. Ask what happens if the phone has no connection at the moment you add a step, and what happens if it comes back halfway through.

- The local queue is the source of truth for what still has to go out. Do not infer pending work from the screen, from a cached list, or from what the server says. If the queue and the UI disagree, the queue wins and the UI is wrong.
- Order is not guaranteed unless you make it so. If a driver captures several proofs and the first one is large or stuck, should the others wait? Today's behaviour is a deliberate tradeoff, so find out what it is before you change it. A stuck item at the head of the queue can hide every proof behind it, and a driver will not notice until dispatch calls.
- Constraints on the scheduled work (network type, battery, storage) are a trade between delivery reliability and phone health. Tightening them makes proofs wait longer. Loosening them can upload large media over a metered connection and cost the driver real money. Drivers often use their own data plans or cheap devices. Do not change constraints without thinking about that.
- Low storage on the device is a real condition. Captured media sits on disk until the upload is confirmed. If you delete local files too early, a failed upload is unrecoverable. If you delete them too late, the phone fills up and capture itself starts to fail. Cleanup should happen only after confirmed success, and it should be tolerant of files that are already gone.
- Be wary of anything that touches the queue schema in the local database. Phones in the field run old app versions for a long time. A migration that drops or rewrites pending rows can destroy proofs that were captured but never sent. Test the upgrade path with a non-empty queue, not an empty one.
- App updates, reinstalls and "clear data" are all things drivers do when something feels broken. Think about what survives each. If the answer is "the queue does not survive", say so out loud in the change description so someone can decide whether that is acceptable.
- Time on the phone can be wrong. Drivers change timezones, and some devices have a bad clock. Do not use device time for anything that needs to be ordered or trusted on the server without a plan for what happens when it is off.
- Background limits differ by manufacturer. Some vendors kill background work aggressively. A change that only works when the scheduler runs promptly is fragile. Keep a path that also drains the queue when the app is next opened in the foreground, and do not remove it because the background path "always works" on your test phone.
- Do not show the driver a "sent" state that is really "saved on the phone". The wording and the state behind it should match. A driver who believes a proof is delivered will stop worrying about it.

## Payloads, schema and compatibility

Proofs are described with Protocol Buffers. That gives you a compact, versionable format, and it also gives you a compatibility contract that is easy to break without noticing, because both sides compile and run fine.

- Treat field identity as permanent. Never reuse or renumber a field, and never change a field's type in place. Old phones will keep sending the old shape for as long as they are in use, and the backend will keep seeing it. Add new fields, mark old ones as retired, and leave them retired.
- Queued items may have been serialized by an older version of the app than the one that is now trying to send them. The worker has to read what it wrote earlier. If you change how the payload is built, check what happens to an item that was built the old way and has been waiting in the queue.
- Defaults can lie. A missing field and a field set to its default value look the same on the receiving side in some cases. If a new field changes meaning when absent (for example, "no signature captured" versus "signature not recorded by this app version"), add an explicit marker instead of relying on the default.
- Unknown fields should be tolerated and preserved where the code passes a message along. Do not rebuild a message field by field in a way that silently drops fields you do not know about yet.
- Size matters. Large media and big batches interact with request limits, memory on cheap phones and slow links. Do not load a whole file into memory to send it. Stream, or chunk, and make sure a retry does not restart from the beginning more often than it has to.
- Keep sensitive data in mind. A proof can carry a photo of a doorstep, a signature, a recipient name and a location. Do not log those. Do not put them into crash reports, analytics events or error text. When you add logging for debugging, strip it again, or make sure it contains identifiers and states only.
- Location and timestamps in the payload are evidence. Changing how they are captured, rounded or attached changes what the proof can be used for in a dispute. Talk to whoever owns the product side before altering their meaning.
- Generated code should be regenerated, not hand-edited. If the build output and the schema disagree, trust the schema and find out why the generated files are stale.

## Backend rules, rollout and checking your work

The backend side is Firebase and Cloud Firestore. Security rules, indexes and server-side functions are part of this component's behaviour even though they live elsewhere in the repo or in the console.

- A change in the worker that writes a new field or a new document shape will be rejected quietly if the security rules do not allow it. Check the rules first. The same applies in reverse: tightening a rule can strand every old app version that is still in the field.
- Authentication state is a failure source. A driver's session can expire while proofs are waiting. The worker should be able to get a fresh credential or pause cleanly, not fail every item and give up. Do not discard queued proofs because a token was stale at the time of the attempt.
- Firestore has its own offline persistence and write queue. The worker has its own queue too. Two queues stacked on each other can hide problems: one thinks a write is done while the other still holds it. When you change how writes are made, be clear about which queue you rely on, and do not rely on both without understanding how they interact.
- Cost counts. Every extra read, write or listener multiplies by the number of drivers and the number of retries. An innocent "check if it exists first" before each write doubles traffic on a flaky connection. Prefer a write that is safe to repeat over a read followed by a write.
- Rollouts are slow and uneven. Old and new app versions will talk to the same backend at the same time. Ship backend changes that accept both before shipping the app change that uses the new form, and remove old support only when the old versions are gone, which is later than you think.
- Feature flags and remote config can help, but a flag that changes upload behaviour also needs an answer for a phone that is offline and has never fetched the flag. Decide what the default is and make it the safe one.

How to check a change before you trust it:

- Run it with the network off, turn it on in the middle of an upload, turn it off again, and kill the process at a few different points. Then look at the queue, the local files and the backend record and confirm each proof exists exactly once.
- Try a proof built by the previous app version, sitting in the queue, sent by the new code.
- Try a large media file on a slow, lossy link, and a very small phone with little free storage.
- Try an expired session and a rejected write, and confirm that one bad item does not block the good ones.
- Read the logs from those runs for anything personal that should not be there.
- Look at the tests that exist for the worker. If they only cover the happy path, add a test for the case you just thought about before you merge, not after.

If you are unsure whether a change is safe for proofs already waiting on drivers' phones, treat that as a sign to slow down, write down the assumption, and ask. A wrong guess in this component is paid for by a driver who cannot prove they did their job.
