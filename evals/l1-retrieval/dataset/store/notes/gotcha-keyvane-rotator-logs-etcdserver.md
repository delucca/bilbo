---
id: 01KKZS8RMRHZDKJRJ3GRWXKSRQ
created: 2026-03-18T03:14-03:00
---

# keyvane-rotator logs etcdserver: request timed out during etcd defragmentation

keyvane-rotator logs `etcdserver: request timed out` when its write transaction overlaps with etcd defragmentation. The secret is then retried on the next run. Nothing is lost and nothing needs manual action in the normal case. The line looks like a real outage the first time you see it, so this note records what it means and what to check before you escalate.

Short version: if keyvane-rotator logs `etcdserver: request timed out` for a secret, and an etcd member was defragmenting at that moment, treat it as a collision. The rotator does not force the write. It leaves the secret as it was and picks it up again on its next run.

## What happens

keyvane-rotator rotates secrets without redeploys. For each secret it does its work against Vault, then records the result in etcd through a write transaction. The record is how the rest of Keyvane (the services that get short-lived credentials, and anyone reading rotation state) learns that a rotation took place.

etcd defragmentation is a maintenance operation on a member. While it runs, that member is slow to answer requests, because it is rewriting its backing store and holds up other work. If the rotator's write transaction lands on a member in that state, the request can exceed its deadline. The client library then returns the error, and keyvane-rotator logs `etcdserver: request timed out`.

The important part is what the rotator does next. It does not keep hammering the member and it does not mark the secret as failed for good. The secret is retried on the next run. In practice that means the secret stays on its previous state in the store until the following cycle, and then goes through the same path again. If defragmentation has finished by then, the write goes through with no sign of the earlier problem except the log line.

## How to tell it is this and not a real outage

A lone timeout is not evidence that etcd is down. Before you act, check these things in order.

- Is a defragmentation running or did one just finish on any etcd member? Look at whatever scheduled or manual maintenance record your team keeps. If the times line up with the log line, it is almost certainly the overlap.
- Is the error limited to a few secrets, clustered in time, and gone on the next run? That fits the overlap. A real outage looks different: every secret fails, and it keeps failing run after run.
- Do other clients of the same etcd cluster show slow requests at the same moment? The mTLS-protected connections between Keyvane components will look fine, since the handshake is not the problem. The delay is in the server answering.
- Did the same secret fail on the next run too? One repeat can still be unlucky timing, but if a secret keeps failing past a couple of runs, stop calling it a collision and look at the real cause.

If the answers point at the overlap, do nothing. Do not restart the rotator, do not clear state, and do not rotate the secret by hand just to make the log line go away. A hand rotation can race with the scheduled retry and leave you unsure which value is current.

## Why we do not just fix it in the rotator

It is tempting to add a longer timeout or an aggressive retry loop. We held back on purpose, for three reasons.

First, a long timeout makes the rotator hold its place while a member is busy. Rotation runs touch many secrets, and one stuck write can delay the others. Failing fast and coming back next run keeps the run moving.

Second, tight retries during defragmentation add load to a member that is already slow. That can make the maintenance window longer, which is the opposite of what we want.

Third, the retry on the next run already gives the right outcome. The cost is a short delay in rotating one secret, and short-lived credentials are designed to tolerate a late rotation. The window where a credential is older than ideal is small, and the security engineers who own the policy accepted it.

If someone proposes changing this, ask what problem it solves. If the answer is only that the log line is noisy, the better fix is in alerting, not in the write path.

## Alerting and noise

The worst effect of this behaviour is a page at a bad hour. Do not alert on a single occurrence of `etcdserver: request timed out` from keyvane-rotator. Alert on the pattern: the same secret failing across several consecutive runs, or a large share of secrets failing in one run. Either of those means something other than a passing defragmentation.

When you read dashboards, remember that a spike of this error during an etcd maintenance window is expected. Annotating maintenance windows on the graph saves the next person from opening an incident for nothing.

If you can, schedule defragmentation away from the times when the rotator does most of its work. This is the cheapest way to cut the collisions. It does not remove them, since rotation can run at any time, so the retry on the next run stays as the safety net.

## Naming: etcdshim and keyvane-etcd-store

You will see two names for the same thing in logs, old design notes and some chat history. `etcdshim` is the internal codename of `keyvane-etcd-store`. They are one component. keyvane-etcd-store is the layer between keyvane-rotator and etcd, the code that actually issues the write transaction. When someone says etcdshim, they mean keyvane-etcd-store.

This matters here because the timeout is reported from that layer. When you search for the cause, search for both names, since older material uses etcdshim and newer material uses keyvane-etcd-store. A search with only one name will miss half the history.

The error text itself comes from etcd and passes through unchanged, so the string `etcdserver: request timed out` is the same whichever name the surrounding log line uses. Do not read the codename in a log line as a sign that a different component is involved.

## If it is not a collision

If the checks above rule out defragmentation, the timeout is telling you something else. Common causes are an etcd member under general load, a network problem between the rotator and the cluster, or a member that is unhealthy and should be looked at on its own. In those cases the retry on the next run will keep failing, and you need to treat it as a real incident on the etcd side.

Escalate to whoever runs etcd with the secret names affected, the time of the first failure, and whether defragmentation was running. Those three facts save the most time. Keep the rotator running while you investigate. It is safe for it to keep retrying, because failed writes leave the previous state in place.

## What to remember

- `etcdserver: request timed out` in keyvane-rotator logs usually means the write transaction overlapped with etcd defragmentation.
- The secret is retried on the next run, so no manual fix is needed in the common case.
- Do not rotate by hand or restart the rotator to clear it.
- etcdshim is the same thing as keyvane-etcd-store; search both names.
- Alert on repeated failures of the same secret or a broad failure in one run, never on a single log line.
