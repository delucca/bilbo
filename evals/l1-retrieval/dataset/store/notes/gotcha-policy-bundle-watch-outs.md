---
id: 01KPJF46R0K85HHER5W33X48HE
created: 2026-04-19T05:53-03:00
---

# policy-bundle-repo: things to watch out for when changing it

Notes for anyone touching policy-bundle-repo, written quickly after getting bitten a few times. Nothing here is a rule from the team. It is a list of places where a small change in this repo ends up somewhere you didn't expect. AuditMesh scans cloud infrastructure configs, and policy-bundle-repo holds the Open Policy Agent policies that decide what counts as a violation. Everything downstream (the Python scanners, the Lambda functions, the DynamoDB records, the Jira tickets) depends on what these policies say and on the shape of what they return.

The main trap is that a policy change looks local. You edit a rule, the unit tests pass, and the review is easy because the diff is small. The effect shows up later, in a ticket queue that a security team is reading.

## Where the change actually lands

The repo is not a leaf. Policies are built into bundles, the bundles are picked up by whatever evaluates them, and the evaluation results are turned into findings and then into tickets. Before changing anything, trace that path in your head at least once:

```text
policy-bundle-repo -> Open Policy Agent -> AWS Lambda -> DynamoDB -> Jira
```

A change at the left end can alter every stage to the right of it. Some examples of what that means in practice:

- A rule that now fires on more resources means more findings stored in DynamoDB and more tickets created in Jira. Nobody asked for those tickets, and cloud security teams notice a flood very quickly.
- A rule that now fires on fewer resources means findings silently stop. Nothing errors. Closed-loop logic that auto-resolves tickets when a finding disappears may then close tickets for problems that still exist.
- A change to the shape of the output (field names, nesting, whether something is a set or a list) can break the Python code that reads it, or worse, not break it and just produce empty values in tickets.

So when you review a diff here, ask what the consumers do with the result, not only whether the rule is logically correct.

## Output shape is a contract

The policies return structured results, and the Lambda code and the ticket templates read specific fields. That shape is an informal contract and it is easy to break because the Rego looks self-contained.

Things to check when you touch a rule's result:

- Renaming a field in the result object. Search the consuming Python for the old name before you rename. Also search the Jira templating, since field names often end up in ticket text.
- Changing a value from a single item to a collection or the other way around. Rego is permissive about this and Python usually is not.
- Adding a message or reason string. These often end up verbatim in tickets. Keep them readable by a person who has never seen the policy, and keep them free of internal details or anything resembling a secret.
- Removing a field that looks unused. It may be used only by the deduplication logic, which would then start creating duplicate tickets.

If you change the shape on purpose, the consumers and the policies have to move together. Don't merge the policy side alone and plan to fix the Python afterward. Bundles can be loaded by running Lambda functions before the code that understands them is deployed, or the other way around, so think about both orders and about what happens in the gap.

## Rule semantics that bite

Open Policy Agent has some behavior that is easy to forget when you are in a hurry. These are the ones that have caused wrong results here:

- **Undefined is not false.** If a rule depends on a field that is missing from the input, the rule is undefined, not false. A negated condition on a missing field can then behave opposite to what you meant. Cloud configs are full of optional fields, so decide explicitly what a missing value means for each rule: compliant, violating, or unknown.
- **Default values.** A default on a rule changes what callers see when nothing matches. Adding or removing one changes output for every resource that previously matched nothing, which is usually most of them.
- **Multiple rule bodies with the same name are ORed.** Adding a second body to an existing rule widens it. People expect it to refine it.
- **Iteration over empty collections.** A rule that iterates over a list of, say, rules or attachments simply does not fire when the list is empty. That can hide a violation, such as a resource with no protection configured at all.
- **Type mismatches.** A string compared against a number is just false, with no error. Input produced by different scanners may differ in whether values come as strings or numbers.
- **Partial evaluation and caching.** If anything evaluates policies in a precompiled or cached way, a change might not take effect where you expect until the cache or bundle is refreshed.

Write a test for the missing-field case and the empty-collection case whenever you add or alter a rule. Those are the two cases that tend to be skipped, and they are the two that matter most for a security tool.

## Testing and rollout

Policy unit tests prove the rule does what you wrote. They don't prove that you wrote the right thing, and they say nothing about volume. What helps:

- Run the changed policies against a realistic sample of inputs and compare the set of findings before and after. Look at the difference, not just whether the new rule works on its own fixture. A diff of findings is the closest thing to a preview of the ticket queue.
- Be suspicious of any change that moves the finding count a lot in either direction. A big drop is as suspect as a big rise.
- Keep fixtures representative. Fixtures written by hand tend to have every field present and every type clean. Real configs do not.
- Check that fixtures contain no real account identifiers, credentials, or customer data. Sanitize anything you copy from a real scan.
- If a rule is new and noisy-looking, consider whether it should first report without creating tickets. Check how the pipeline handles that before assuming it can. Don't invent a mode that the Lambda side doesn't support.

Rollout order matters too. If bundle publishing and the consuming code are separate deployments, think through which one lands first. A bundle that is ahead of the code can produce results the code mishandles. Code that is ahead of the bundle usually fails more gently, but check that too.

## Tickets, state, and long-lived findings

Findings are stored, and tickets are created from them and tracked over time. That means history exists, and a policy change interacts with it.

- If a rule's identity is part of how findings are keyed or deduplicated, renaming or splitting that rule makes old findings look unrelated to new ones. The old ones may never be resolved, and the new ones create fresh tickets for the same underlying problem.
- Merging two rules into one has the mirror problem: tickets for one of them may be closed as if fixed.
- Changing the severity or category of a rule affects ticket priority, routing, and sometimes deadlines that teams track. Don't treat severity as cosmetic text.
- Changing the wording of a message can change dedup behavior if the message is part of the key. Find out before editing wording.
- Removing a rule entirely leaves stored findings and open tickets behind. Decide what should happen to them, and say so in the change description, because the repo itself won't do it.

It helps to write down in the pull request what you expect to happen to existing findings and open tickets. If you can't say, that is a sign the change needs a closer look before it ships.

## Small habits that save time

- Keep each change to one policy concern. Mixing a refactor with a semantic change makes the findings diff impossible to read.
- When refactoring, a pure refactor should produce an identical findings diff, which is empty. Use that as the test.
- Comment on why a rule is exempted or loosened, not only what it does. Exemptions without reasons turn into permanent holes.
- Don't hard-code environment-specific values in policies. Pass them as data so the same bundle works everywhere.
- Don't copy rules from public examples without reading them. Their assumptions about input shape may not match what our scanners produce.
- Say in the commit message when a change is expected to alter the number of tickets. People on the Jira side will thank you.
- If you aren't sure how a consumer uses a field, read the consumer. It is quicker than guessing and being wrong in production.
