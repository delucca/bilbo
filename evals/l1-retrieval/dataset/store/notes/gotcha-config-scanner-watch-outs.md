---
id: 01K9E4RE3WCMADVP4BRRWXJ6W9
created: 2025-11-07T00:11-03:00
---

# config-scanner: general things to watch out for when changing it

These are the traps I keep hitting or expect to hit when touching `config-scanner`. It is the part of AuditMesh that reads cloud infrastructure configuration, runs it through Open Policy Agent policies, and hands the violations to the ticketing step. Most of the trouble comes from the seams between those parts, not from any single piece. None of this is a spec. It is a list of places where a small change has a larger effect than it looks like.

Read the section that matches what you are changing, then skim the last one before you call the work done.

## The scanner and the policies are two codebases that look like one

A change in the Python side of `config-scanner` can alter what the policies see, and a change in a policy can alter what the Python side must handle. Treat the input document as a contract between the two. If you rename a field, move a nesting level, or change how a list is represented, every policy that reads it breaks quietly. The policy will not fail loudly. It will simply stop matching, and the scan will report fewer violations than before.

Before changing the normalization code, grep the policies for every field you touch. Do the reverse too: if you add a new field to a policy, check that the scanner actually produces it for every resource type the policy applies to. A policy that reads a field the scanner never sets is the most common reason for a rule that looks right and never fires.

## Undefined is not false

Open Policy Agent treats a missing value as undefined, and undefined propagates. A rule body that references a missing field is just not true, and a negated check on it can behave differently from what you expect. Writing a rule that says a bucket must not be public is not the same as writing one that flags the bucket when a public setting is found. If the setting is absent, the first style can pass a resource that was never examined.

When you edit a rule, ask what happens when each referenced field is missing, null, empty, or the wrong type. Decide explicitly whether absence is a violation, a pass, or an unknown. Write that decision in a comment near the rule. Do not leave it to the evaluation semantics, because the next person will read the rule differently.

## Absent data versus a clean result

The scanner has to tell the difference between a resource that was scanned and found clean, and a resource that could not be read. Both can produce an empty violation list. If a change to the collection code swallows an exception and returns an empty structure, the downstream effect is that real violations get closed as resolved. Closing tickets on the basis of silence is the worst outcome this component can produce.

Keep a distinct status for resources that failed to load, and make sure anything that auto-resolves tickets checks that status. When you add a new collection path, add the failure path at the same time. Do not rely on a general handler at the top of the function to do the right thing.

## Lambda limits shape the design

The scanner runs inside AWS Lambda, so execution time, memory, and payload size are all bounded. A change that looks harmless, such as loading all resources of a type before evaluating, can push a large account over the limit while small test accounts pass. Test with the largest realistic input you can get, not with fixtures.

If a scan has to be split, remember that the split points become part of the behavior. Work that is divided across invocations needs a way to know it is complete, and a way to resume after a timeout without redoing everything. Do not add a loop that assumes it will finish in one invocation. Also keep an eye on the size of anything passed between invocations or stored as an event, since those limits are tighter than they seem.

## Cold starts and policy loading

Policies are loaded and compiled when the function starts, or on first use. Adding many policies, or making them heavier, raises the cost of every cold start. Hold the compiled state in module scope so warm invocations reuse it, but be careful that reuse does not leak data between scans. Anything that holds per-account or per-scan state must be created fresh each time, even though the policy engine itself is shared.

When policy bundles are packaged with the function, a forgotten rebuild means the deployed function runs old rules while the repository shows new ones. Check which version of the policies a deployed function is actually carrying before you debug why a new rule does nothing.

## DynamoDB is the memory of the scanner

State between scans lives in DynamoDB: what was seen, what was ticketed, what was suppressed. Any change to the item shape is a migration, even if no migration script exists. Old items stay in the table, and new code has to read them. Add new attributes as optional, and make the code tolerate their absence for as long as old items might exist.

Be careful with key design. A key built from fields that can change, such as a resource name that can be renamed, will make the same resource look new after the change, and that produces duplicate findings. Prefer stable identifiers from the cloud provider. If you change how keys are built, think through what happens to existing items before shipping.

## Consistency, conditional writes and retries

Lambda can invoke the same work twice, and retries after a timeout are normal. Writes that record a finding or a ticket must be safe to repeat. Use conditional writes where an item should only be created once, and handle the case where the condition fails as an expected outcome rather than an error to log loudly.

Reads right after writes can return stale data if you use the default consistency. If the scanner reads its own recent state to decide whether to open a ticket, this matters. Decide per read whether you need the stronger mode, and note that it costs more. Also watch for hot partitions when many findings share a key prefix, since a busy account can throttle itself.

## Pagination and throttling on the cloud side

Every listing call to the cloud provider can be paginated, and the default page is often smaller than the real set. A change that drops the pagination loop, or stops at the first page on an error, produces a partial picture that looks complete. When you add a new resource type, copy the pagination handling from an existing one and check it, rather than writing a single call.

Throttling from the provider is also routine. Retries with backoff are needed, and they must be bounded so that one slow account does not use up the whole invocation. Distinguish between a throttled call you retried and succeeded, and one you gave up on. The latter should mark the resource as not scanned, per the section above.

## Ticket creation in Jira

The step that turns violations into Jira tickets has the highest cost of mistakes, because humans see the output. Duplicate tickets, tickets with empty descriptions, and tickets opened against the wrong project all erode trust in the whole tool. Any change to how a violation is turned into a ticket should be checked against how the existing tickets are matched, since the matching is what prevents duplicates.

The link between a finding and its ticket must be stored reliably, and it must be stored in a way that survives a retry. If the ticket is created and then the write that records it fails, the next run will create another one. Think about the order of operations and about what a recovery looks like. A search of Jira for an existing ticket before creating is a useful second check, though it is not a replacement for good state.

## Jira fields, workflows and limits

Jira projects differ in required fields, allowed values, and workflow transitions. Code that works against one project can fail against another because a field is required there or a status name differs. Do not hardcode names that a project administrator can change. Read them from configuration, and make the failure clear when a configured value does not exist.

Jira also limits request rates and the size of text fields. A long violation description may be cut off or rejected. Keep descriptions structured and short, and put the long evidence somewhere linked. Transitions to closed states can be blocked by workflow rules, so a resolve step must handle a refusal without losing track of the ticket.

## Severity, grouping and ticket volume

How findings are grouped decides how many tickets a team gets. Grouping by resource gives many small tickets, grouping by rule gives few large ones, and each choice changes how teams work the queue. A tweak to grouping can flood a project the first time it runs, because existing findings regroup into new tickets. Before changing grouping or severity mapping, think about what happens to the open backlog, not only to new findings.

Severity is read by people and sometimes by automation that sets priority or due times. Changing a mapping silently changes those downstream behaviors. Tell the consumers, and keep the old and new mapping from being mixed within one scan.

## Exceptions, suppressions and expiry

Teams will ask to suppress findings, and the suppression logic is a place where policy and state meet. A suppression keyed too broadly hides real problems; one keyed too narrowly gets ignored. Keep the matching rule simple and documented. Suppressions that never expire become permanent blind spots, so if there is an expiry, make sure the scanner actually checks it on each run and does not just read the flag.

When you change resource identity or normalization, check what happens to existing suppressions. A suppression that stops matching will cause old accepted findings to come back as new tickets, which annoys exactly the people who already made a decision.

## Testing policies properly

Policy tests should cover three cases for each rule: a clear violation, a clear pass, and a resource with the relevant data missing. The third case is the one that gets skipped, and it is where the undefined behavior bites. When you fix a bug in a rule, add the test that would have caught it before you fix the rule.

Fixtures drift from reality. Refresh at least some of them from real configuration exports with sensitive values removed, since hand-written fixtures tend to match the author's idea of the format. Also run the whole policy set against a realistic sample after any change to normalization, and compare the count of findings by rule before and after. A big shift in one rule that you did not mean to touch is a strong signal.

## Permissions and credentials

The function needs read access to many accounts, plus write access to its own state and to Jira. Keep the cloud side read-only. A new resource type usually needs a new permission, and the failure shows up as access denied for that type only, which can look like an empty result if errors are swallowed. When you add collection for something new, update the permission definitions in the same change and test in an account with the real role, not with an administrator's credentials.

Jira credentials and any tokens should come from a secret store, never from the code or the environment values in plain config. Rotation should not require a code change. Check that a failed authentication is reported as an outage of the ticketing step and not as zero findings.

## Logging and what must not be logged

The configurations being scanned can contain secrets: connection strings, keys in environment settings, tokens in user data. Logging a whole resource document during debugging is an easy way to copy those into a log store with a much wider audience. Log identifiers and rule names, not full documents. If you must dump input for a debug session, do it locally with scrubbed data and remove the logging before merging.

Tickets are another place where this leaks. Evidence copied into a Jira description is visible to everyone with access to the project. Include the offending setting name and location, not the secret value.

## Python packaging and dependencies

The deployment package has size limits and a specific runtime. A new dependency may pull in compiled parts that do not match the Lambda environment, so it can import fine locally and fail on deploy. Keep dependencies few, and check that anything with native code is built for the target platform. Pin versions so that a rebuild does not change behavior on its own.

The policy engine integration can be an embedded library, a bundled binary, or a call out to a separate service. Whichever it is, upgrading it can change evaluation behavior in small ways, including how built-in functions treat edge cases. Run the full policy tests after any upgrade and read its release notes for semantic changes before assuming a pass means safe.

## Rolling out a change

The scanner runs on a schedule and on demand, against many accounts, so a bad release hits everyone at once. Where possible, run a new version in a mode that evaluates and records what it would do without opening or closing tickets, and compare with the current version. This is the only reliable way to see ticket volume changes before people do.

Keep changes small and separable: policy changes, state shape changes, and ticket formatting changes should not ship in the same release, since you will not be able to tell which one caused a surprise. Have a way to roll back that does not depend on state written by the new version being readable by the old one. If the new version writes items in a new shape, the old version must still cope with them.

## Before calling a change to config-scanner done

Check that missing data is handled, not just present data. Check that a failed read cannot be mistaken for a clean result. Check that repeating the same invocation gives the same state and no extra tickets. Check that existing state, suppressions and open tickets still match after your change. Check that no secrets reach logs or ticket text. Check that the deployed package carries the policies and dependencies you think it does.

If any of those cannot be checked locally, say so in the change description, so the next person knows what was never exercised.
