---
id: 01KEZ3BX8PH694ZRNGCHXB2ERV
created: 2026-01-14T17:32-03:00
sources:
  - "doc: Matching rules v3"
---

# ledger-matcher spec: matching rules and rounding tolerance

This note specifies how ledger-matcher compares card-processor settlement records with internal ledger entries, and what it does when they do not agree. The rule most people ask about is the rounding tolerance: ledger-matcher must accept a rounding difference of up to 1 minor unit between a settlement amount and a ledger amount. Anything larger is a mismatch and goes to review. Details below are kept general on purpose; fill in specifics from the code when you need them.

## Purpose

Finance operations teams at online marketplaces receive settlement files from card processors. Each line says the processor paid out or charged some amount. The internal ledger holds its own entries for the same movements. ledger-matcher pairs the two sides and decides whether each pair agrees. Pairs that do not agree are flagged so a person can look at them. It does not fix anything itself.

## Scope

In scope: pairing settlement lines to ledger entries, comparing amounts and currencies, producing a verdict per pair, and emitting unmatched or mismatched items for review. Out of scope: parsing raw processor file formats (that happens upstream, before ledger-matcher sees the data), posting corrections to the ledger, and the review UI.

## Inputs and outputs

Inputs are normalized settlement records and ledger entries. Settlement records arrive through the event stream on Kafka; ledger entries are read from PostgreSQL. Results are written back to PostgreSQL and a review event is published for every item that needs attention. Other services can ask for match status over gRPC.

Each verdict carries the settlement reference, the ledger reference if one was found, the amounts on both sides, the difference, and a reason code. Keep the reason codes stable; downstream dashboards group by them.

## Matching rules

Pairing goes by processor reference first, then by a fallback key built from merchant, date and amount when the reference is missing. A settlement line pairs with at most one ledger entry and the reverse. If two candidates tie, do not guess: flag the item as ambiguous.

Currency must be identical on both sides. A currency difference is always a mismatch, no tolerance applies.

Amounts are compared as integers in minor units. Never compare floats. Conversion from the processor's representation to minor units happens before comparison, and the exponent of the currency decides what a minor unit is.

## Rounding tolerance

ledger-matcher must accept a rounding difference of up to 1 minor unit between a settlement amount and a ledger amount. In other words, if the absolute difference between the two integer amounts is at most 1 minor unit, the pair is treated as matched. The reason for the tolerance is that processors and the ledger round fees and splits at different steps, so a one-unit drift is normal and not worth a human's time.

Rules around the tolerance:

- It applies per pair, not per batch. Many one-unit differences in one file are each accepted; they are not summed and then judged.
- A tolerated pair is still recorded with its actual difference, so totals can be audited later.
- A difference above 1 minor unit is a mismatch, even if it is only just above.
- The tolerance does not apply to currency differences or to sign differences. A refund against a charge is a mismatch regardless of size.

## Mismatch handling

A mismatch produces a review item with both sides attached. Items are never dropped silently. An unmatched settlement line (no ledger entry) and an unmatched ledger entry (no settlement line) are separate categories, since the usual cause differs: late settlement versus a missing booking. Unmatched ledger entries should wait for a grace period before being flagged, because settlement files lag.

## Idempotency and replays

Kafka delivers at least once, so the same settlement record can show up twice. Matching must be idempotent: reprocessing a record yields the same verdict and does not create a second review item. Key writes on the settlement reference. When a late ledger entry arrives for a previously unmatched settlement line, the verdict is updated and the old review item is closed.

## Open questions

Whether the tolerance should become configurable per currency or per merchant is undecided. For now it is a single fixed rule. Also undecided: how long the grace period for unmatched ledger entries should be in production. Check with finance operations before changing either, and update this note when it is settled.
