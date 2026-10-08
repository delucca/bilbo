---
id: 01M0FJZF0NWGK5PDPD77MFSPYR
created: 2026-08-20T09:40-03:00
---

# stockout-model sets maxDepth=8

The stockout-model now sets `maxDepth=8`, replacing the earlier depth setting, because added promo-calendar features made deeper trees pay off on validation. This note replaces the earlier note about "stockout model sets maxdepth"; the new value is `maxDepth=8`.

## Decision

The tree depth limit for the stockout-model is `maxDepth=8`. The earlier depth setting is retired. Anyone reading only this note should take `maxDepth=8` as the current value and ignore whatever the older note said.

## Why the change

The reason is the promo-calendar features. Once they were added to the feature set, deeper trees started to pay off on validation. With the old, shallower limit the trees could not use the interactions between promo timing and store-level demand. With `maxDepth=8` they can, and validation results improved.

## What changed in the inputs

The promo-calendar features describe when promotions run and how they relate to the days around them. They sit next to the existing sales, inventory and calendar features that the stockout-model already used. The depth change only makes sense with these features present. If they are removed, the depth choice should be looked at again.

## What this setting controls

The depth limit caps how many levels each tree in the stockout-model can have. A higher cap lets a tree model more feature interactions. It also raises the risk of fitting noise and makes training slower and the model bigger. That tradeoff is why the setting was checked on validation and not picked by feel.

## Evidence

The support is validation performance after the promo-calendar features went in. Deeper trees did better than shallower ones on that validation data. No other justification is recorded here. The exact validation figures are not kept in this note, so look at the training run output if you need them.

## Where it applies

The setting belongs to the stockout-model training step, which runs in Spark on Scala code and reads feature tables from Delta Lake. Airflow schedules the training and scoring. Predictions and the replenishment orders built from them reach merchandising analysts through Snowflake. The depth value only affects training; downstream steps do not read it directly.

## Effects on replenishment

A change in the model can shift which stores are flagged for stockouts, and so which replenishment orders get generated. Analysts may see different flags than before for promo periods. That is expected and is the intent of the change.

## Risks

Deeper trees cost more time and memory during training. Overfitting is the main modelling risk, so keep watching validation against held-out data. If validation gets worse after later feature changes, the depth is a candidate to revisit.

## Things to check if results look off

First confirm the training job really picked up `maxDepth=8` and not a leftover value from the old setting. Second, confirm the promo-calendar features are populated for the period in question. Third, compare against the previous model on the same validation window.

## Alternatives not taken

Keeping the earlier depth setting was rejected because it gave up the gain from the promo-calendar features. No other depth values are recorded as candidates in this note.

## Supersedes

This note replaces the earlier note about "stockout model sets maxdepth". That note should no longer be followed. The current value is `maxDepth=8`.

## Open items

Decide whether the depth should be tuned again if more features are added. Decide whether to record the validation numbers in a separate note so the evidence is easy to find.
