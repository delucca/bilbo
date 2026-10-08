---
id: 01JWFJEM5DDZNZ0N0V0XNM7QVS
created: 2025-05-30T00:05-03:00
---

# blob-archive-client: retried upload fails with BlobAlreadyExists unless overwrite is enabled

blob-archive-client fails a retried upload with Azure.RequestFailedException: The specified blob already exists. (ErrorCode: BlobAlreadyExists) unless overwrite is enabled. The first attempt can succeed on the storage side while the caller never learns it. The retry then writes to the same blob name, and Azure Blob Storage refuses it. Anyone who sees this error after a timeout, a dropped connection or a message redelivery should first suspect that the original upload actually landed.

This note records why it happens, how to recognise it, what is safe to do, and what is not. It is written for the next engineer or agent who touches blob-archive-client in LabNotebook Sync and wonders whether the error is a bug in the client, a storage fault, or something to suppress.

## Symptom

The visible failure is an exception of type Azure.RequestFailedException with the message The specified blob already exists. and the error code BlobAlreadyExists. It shows up in the logs of whichever service calls blob-archive-client, and usually not on the first try. A typical log sequence looks like this: an upload starts, a transient problem is logged (a timeout, a reset connection, or a cancelled request), a retry begins, and then the retry fails with BlobAlreadyExists.

The operator-facing symptoms vary by path:

- An instrument output file is archived, but the sync job reports it as failed and keeps reporting it as failed on every later pass.
- A notebook entry attachment appears in the audit view as pending, while the blob can be found in the container.
- A RabbitMQ message for the archive step goes back to the queue repeatedly, and each delivery ends in the same exception.
- A compliance officer asks why the audit trail shows an archive failure for a file that is plainly present in storage.

The last one is the most confusing for people outside engineering. The failure is real as far as the code is concerned, but the data is fine. The record in SQL Server and the state in blob storage have drifted apart.

## Why it happens

By default the upload in blob-archive-client does not overwrite. That is deliberate for an audit-trail product: silently replacing an archived blob would destroy evidence. With overwrite off, Azure Blob Storage rejects any write to a name that already has a blob, and it answers with BlobAlreadyExists.

The trap is the combination with retries. A retry is meant to repeat an operation that did not happen. But an upload that timed out on the client side may have completed on the service side. The client then cannot tell the difference between a first attempt that failed and a first attempt that succeeded but whose response was lost. When the retry goes out, the service sees a write to an existing name and refuses.

So the error does not mean something is wrong with the storage account, the credentials, or the container. It means the name is taken. The question to answer is who took it: the earlier attempt of this same upload, or something else.

There are a few ways the name can already be taken:

- The earlier attempt of the same upload succeeded and only its acknowledgement was lost. This is the common case.
- A message was delivered twice by RabbitMQ and two workers handled it, each trying to upload the same blob. One wins, the other gets BlobAlreadyExists.
- A previous run of the sync job archived the file and the local bookkeeping was never updated, for example because the process stopped between the upload and the database write.
- Two different source files map to the same blob name. This is a naming problem and is the dangerous case, because the existing blob has different content.

The fourth case is rare but it is the one that makes blanket suppression of the error a bad idea.

## How to confirm which case you are in

Do not guess. Before changing anything, look at the existing blob and compare it with the source.

Start with the metadata of the blob that is already there. Check its size and its content hash against the source file. If the client stored a hash at upload time, compare that too. If the size and hash match the source, the earlier attempt succeeded and the retry was redundant. If they do not match, stop: you have a name collision or a partial write, and overwriting would destroy something.

Next, look at the bookkeeping in SQL Server for that entry. If the entry is still marked as not archived while the blob exists with matching content, the drift described above is confirmed. The fix in that case is to mark the entry as archived, not to upload again.

Then check the queue side. If the same message has been delivered more than once, look at how the consumer acknowledges. A consumer that acknowledges only after the whole handler finishes will see redelivery after any crash or timeout in the middle, which produces exactly this error on the second delivery.

Finally, read the log around the first failure, not the last. The last line is always BlobAlreadyExists. The interesting line is the transient failure just before the first retry.

## What to do about it

There are three reasonable responses, and which one fits depends on the caller.

### Treat it as already done, after verifying

For most archive paths the right behaviour on BlobAlreadyExists is to check that the existing blob matches the content being uploaded and, if it does, report success. This keeps the no-overwrite protection, keeps the audit trail honest, and makes the upload idempotent from the caller's point of view. The check must compare content, not just the presence of the name. Presence alone is what produced the confusion in the first place.

If the content does not match, treat it as a hard failure, surface it, and do not retry. A human should look at it.

### Enable overwrite for a specific call

Some flows really do replace a blob on purpose, for instance when a derived artifact is regenerated from the same instrument output and the previous version is superseded. For those, overwrite can be enabled on that call. Do this per call site and with a comment saying why. Do not flip it on globally in blob-archive-client to make the error go away.

If overwrite is enabled, think about what the audit trail needs. The earlier version of the blob is gone unless versioning or soft delete is turned on for the container. Compliance officers care about that. Check with whoever owns the retention policy before enabling overwrite on any path that stores primary data.

### Make the name unique per attempt of the logical operation

Another approach is to avoid the collision by naming. If the blob name is derived from stable identifiers of the entry and the source file, a retry of the same logical upload always lands on the same name, which is what we want for idempotence but also what triggers the error. If instead each distinct logical object has its own stable name and the retry logic first checks existence, you get the same safe result without needing overwrite. The check-then-write has a race in it, so the conflict error must still be handled even then.

## What not to do

A few tempting fixes are wrong and have a cost in an audit-trail system.

- Do not catch Azure.RequestFailedException broadly and swallow it. The same exception type covers authorization failures, throttling and missing containers. Only the BlobAlreadyExists error code means the name is taken. Match on the error code, not on the exception type or the message text.
- Do not turn on overwrite globally to stop the log noise. It removes the one protection that stops a bad run from replacing archived evidence.
- Do not delete the existing blob and upload again. That is overwrite with extra steps and a window in which the blob does not exist.
- Do not mark an entry archived just because BlobAlreadyExists came back. Verify the content first. A name collision with different content would then be recorded as success.
- Do not widen the retry count or delay to hide it. More retries only make the redundant write more likely, since each one follows a possibly successful attempt.

## Interaction with retries and messaging

There are usually two layers of retry in play, and they stack. The Azure SDK has its own retry policy for transient errors on a single call. On top of that, the sync job or the RabbitMQ consumer may retry the whole handler. The SDK-level retry is the one most likely to produce this error, because it repeats the same request after a timeout whose outcome is unknown.

When the SDK retries internally and the first request had actually been accepted, the caller sees the final exception from the last attempt, which is BlobAlreadyExists, and never sees the earlier successful attempt. That is why the stack trace looks like a plain conflict with no sign of a transient fault. Turn up the log level for the storage client if you need to see the intermediate attempts.

On the message side, RabbitMQ gives at-least-once delivery in our setup. A handler must therefore be safe to run twice for the same message. The archive step is one of the places where that has to be designed in, not assumed. The simplest shape is: verify, upload without overwrite, on BlobAlreadyExists compare content, record the result, acknowledge. If the process dies anywhere in the middle, the next delivery follows the same path and ends in the same state.

Dead-letter handling matters too. If BlobAlreadyExists is treated as a generic failure, the message will cycle through retries and finally land in a dead-letter queue even though the data is archived. Someone then has to clear it by hand. Handling the conflict explicitly avoids that pile-up.

## Audit trail implications

LabNotebook Sync exists to enforce audit trails, so what we record about this event matters as much as what we do about it.

If a retried upload finds an identical blob and we treat that as success, the audit record should still say what happened: an upload was attempted, the blob already existed, content matched, and the entry was marked archived on that basis. A silent success would hide a retry from the record. A compliance reviewer should be able to see from the trail that the archive was confirmed by verification rather than by a fresh write.

If the content does not match, the audit record must show a failure with the reason, and the blob must be left untouched. This is a reportable event. It suggests either a naming defect or a tampering attempt, and we should not decide which in code.

If overwrite is used on purpose, the audit record should link the new blob to the one it replaced, or at least note that a replacement took place and why. Without that, an overwrite looks like data loss to a reviewer.

One more point: timestamps. The blob's own last-modified time will be from the first, successful attempt, which can be earlier than the time the database says the archive happened. That gap is expected in this scenario. Reviewers who compare the two will see a difference and may ask. This note is the answer to that question.

## Testing this behaviour

This failure is easy to miss in tests because the happy path never produces it. A test that exercises it needs to simulate an upload that succeeds on the service but appears to fail to the caller, then run the retry.

Useful cases to cover:

- The retry hits an existing blob with identical content and the call reports success without writing again.
- The retry hits an existing blob with different content and the call fails loudly, leaving the blob as it was.
- The error code is checked, so that a different RequestFailedException, such as an authorization failure, is not mistaken for a conflict.
- A redelivered queue message for an already archived entry ends in an acknowledged message and a consistent database record.
- A call site that deliberately enables overwrite does replace the blob, and the audit record notes it.

A fake storage client that can be told to accept a write and then throw a timeout is more useful here than a mock that only returns canned errors. If the emulator is used locally, remember that it follows the same no-overwrite rule by default, so the problem reproduces there without needing a real storage account.

## Checklist for the next person

When you meet BlobAlreadyExists in blob-archive-client logs, go through this in order.

- Find the first failure for that upload and read what came just before the retry.
- Check that the exception is the conflict, by error code, and not another storage error.
- Compare the existing blob with the source: size, then content hash.
- If identical, fix the bookkeeping and the audit record; do not write again.
- If different, stop, leave the blob alone, and escalate; this is a naming or integrity problem.
- If the flow really should replace the blob, enable overwrite on that call only, after confirming retention and versioning with the owner of the policy.
- Make sure the handler is safe to run twice, so the next redelivery ends the same way.

The short version: the error is the client doing its job. The upload is not idempotent by default, retries make the conflict likely, and the fix is to verify and reconcile, not to switch the protection off.
