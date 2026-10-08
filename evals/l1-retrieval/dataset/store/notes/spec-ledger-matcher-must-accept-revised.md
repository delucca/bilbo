---
id: 01KZEXRMZ92S5YTES99XJH6TNP
created: 2026-08-07T17:13-03:00
---

# ledger-matcher rounding tolerance for three-decimal currencies

This note replaces the earlier note "ledger matcher must accept". New value: ledger-matcher must accept a rounding difference of up to 2 minor units for currencies with three decimals. The earlier tolerance no longer applies.

## Purpose

Settlement files from card processors and our internal ledger entries do not always agree to the last digit. For currencies that use three decimals, the processor and our side may round at different steps, so a small gap shows up even when the money is the same. ledger-matcher should not flag those gaps for review. Finance operations teams were spending time on mismatches that were only rounding noise.

## The rule

When ledger-matcher compares a settlement line with a ledger entry in a three-decimal currency, the amounts count as matching if they differ by no more than 2 minor units. A difference larger than that is a mismatch and goes to review as before.

The tolerance is a ceiling, not a target. A difference of exactly 2 minor units matches. A difference above it does not.

## Scope of the rule

The rule applies only to currencies with three decimals. Currencies with zero or two decimals keep whatever tolerance they had before; this note does not change them. If a currency's decimal count is not known to the matcher, treat it as not covered and do not apply the new tolerance.

The comparison uses the absolute difference, so it does not matter which side is higher. Sign errors (a credit against a debit) are not rounding differences and must still be flagged.

## What it replaces

The earlier tolerance for these currencies was different and is superseded. Do not keep both. Anyone who finds the old value in code, config, tests or docs should change it to the value above and remove the old one rather than leave a fallback.

## Implementation notes

Keep the tolerance as a named setting in the Go matching code, not as a literal buried in a comparison. Do the comparison on integer minor units, never on floats, so the boundary case is exact. The amounts read from PostgreSQL and from the settlement files should be converted to minor units before the check.

## Tests to keep

Cover these cases for a three-decimal currency: equal amounts, a gap just under the limit, a gap exactly at the limit, a gap just over it, and a gap in each direction. Add one case showing that a two-decimal currency is unaffected. Add one case with a sign mismatch that must still be flagged.

## Review queue effects

Fewer rounding-only items should reach the review queue after this change. If the queue volume for three-decimal currencies does not drop, check whether the currency was recognised as three-decimal at all. Events published to Kafka for flagged mismatches keep their current shape; a matched pair within tolerance is simply not flagged.

## Open points

It is not settled whether matches accepted under the tolerance should be marked in the output so reviewers can see that a small difference was absorbed. That would help audits but adds a field to the gRPC messages. Decide before changing the schema.

Also unsettled: whether the tolerance should be configurable per processor. For now it is one value for all three-decimal currencies.
