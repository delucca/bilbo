---
id: 01KZHC1022M3ZH9H7MXGRQNE05
created: 2026-08-08T16:01-03:00
---

# stockout-model tree depth: maxDepth=6

The stockout-model caps its trees with `maxDepth=6`. The reason is overfitting on sparse, slow-moving SKUs: deeper trees memorize the few sales and stockout events those items have, and the predictions on new weeks get worse. This note records the decision, the reasoning behind it, what it costs, and what would have to be true before anyone changes it. If you are about to raise the depth because a metric looks a little better on a fast mover, read the whole thing first.

## Decision

The tree-based stockout-model uses `maxDepth=6` for every tree it trains. It is a single global setting, not a per-category or per-SKU one. Analysts at the grocery chains see one behaviour from the model, and the training job in Spark reads one value. We chose a shallow ceiling on purpose and accepted that some fast-moving items could fit slightly better with more depth.

## Why this value

Slow-moving SKUs have long runs of zero sales, a handful of positive days, and very few real stockout labels. A deep tree can carve out a leaf for almost every one of those rare rows. Training error looks excellent, but the leaf is just a memory of a single store, a single week, and a single odd event such as a delivery delay or a local promotion. When the same pattern does not repeat, the forecast for that SKU is confidently wrong. Capping the depth at `maxDepth=6` forces the trees to stay with broad splits: store cluster, recent velocity, days of cover, promo flag, season. Those generalize.

## What overfitting looked like

The symptoms we saw while comparing depths were consistent. Validation scores on the long tail of slow SKUs dropped as depth grew, while scores on the head of the catalogue barely moved or rose a little. Feature importance shifted toward identifiers and calendar fields that act as proxies for individual rows. Predicted stockout probability for slow items became spiky, flipping between very high and very low from one week to the next with no change in real inventory conditions. Analysts do not trust a model that behaves that way, and it generates noisy replenishment orders.

## Why the long tail matters more than the head

A grocery chain carries a large number of items that sell rarely at any single store. Each of them is individually unimportant, but together they make up a big share of the assortment and a big share of the orders the system proposes. A model that is a bit sharper on the best sellers and unreliable on everything else produces a lot of bad suggestions in total. Fast movers also have plenty of data, so even a shallow tree captures most of their signal. The trade favours the shallow setting.

## Alternatives considered

We looked at several other ways to deal with sparse items instead of a single global cap.

- Deeper trees with a stronger minimum leaf size. This helps, but it ties behaviour to a second parameter whose right value depends on the data volume of each chain, and that makes tuning fragile.
- Separate models for fast and slow items. This could work, but it doubles the surface to maintain and monitor, and the boundary between the groups drifts over time.
- Heavier regularization through sampling of rows and columns. We keep modest sampling, but alone it did not stop the memorization of rare rows.
- Pruning after training. It adds a step and a parameter and gave results close to simply stopping earlier.

A fixed shallow ceiling was the simplest option that fixed the problem, so we took it.

## Trade-offs we accepted

Shallow trees cannot represent high-order interactions. If a real effect depends on a combination of many conditions, such as a specific store type, a specific category, a specific promo mechanic and a weather event together, the model will approximate it only roughly. We accept lower peak accuracy on some fast-moving items. In return we get stable behaviour, predictable training time, and smaller models that are easier to explain to merchandising analysts when they ask why an item was flagged.

## How it is applied in the pipeline

The parameter is set where the stockout-model is configured for the Spark training job, written in Scala. The same value is used whether the job is run by the scheduled Airflow workflow or by hand during experiments, so that offline comparisons match production. Training reads its features from Delta Lake tables, and the scored output that feeds replenishment is published to Snowflake for the analysts. Nothing downstream needs to know the depth; it is a property of the trained artifact only.

## Keeping experiments honest

When someone experiments with other depths, they should compare on slices, not on one overall number. Report results for slow SKUs separately from fast ones, and prefer a time-based split over a random one, because random splits leak the same store and item history into both sides and make deep trees look better than they are. A depth that wins on a leaky split is not evidence against this decision.

## What would change our mind

We would revisit the cap if one of these became true. The volume of history per slow SKU grew enough that rare events are no longer rare. A new feature set carried genuine high-order signal that shallow trees clearly cannot use, shown on a clean time-based split and not only on the fast movers. Or the model were split by velocity class for other reasons, in which case a different depth for the fast class would be reasonable while the slow class keeps `maxDepth=6`. Without one of those, leave it alone.

## Risks

The main risk is silent drift of the value. Someone raises the depth during a tuning session, the result looks fine on aggregate, and it gets merged. The tail then degrades slowly and nobody notices until analysts complain about odd orders. A second risk is the opposite: treating the number as sacred and ignoring a real change in the data. This note is meant to prevent the first without causing the second, which is why the reasoning is written out and not only the value.

## Monitoring

Watch the stability of predictions for slow items from week to week, not only the headline accuracy. Large swings in predicted stockout probability with flat inventory conditions are the early sign that the model is memorizing. Also watch calibration on the slow group: predicted probabilities should match observed stockout frequency. If the shallow model becomes underconfident or loses ranking power on the head, that is the signal to look at the interaction question, not to simply deepen everything.

## How to explain it to analysts

Merchandising analysts do not need the tree details. The short version is that the model deliberately keeps its rules simple so that it does not overreact to a few unusual weeks on items that rarely sell. When an analyst asks why a slow item got a mild, steady risk score instead of a dramatic one, that is the intended behaviour, not a bug. If they think a specific fast item is under-served, check it on its own slice before blaming the depth.

## Relationship to other settings

The depth cap works together with the other training settings but does not depend on exact values of them. Sampling, the number of trees, and the learning rate were tuned with this ceiling in place. If you change the ceiling, expect those to need another look, because a deeper ensemble usually wants a smaller learning rate or fewer trees. Do not change several at once; change the depth alone and compare on the slices above.

## Reproducing the evidence

To redo the comparison, train the stockout-model at several depths on the same time-based split, using the same feature tables in Delta Lake, and score the held-out weeks. Break out the metrics by velocity group. Plot the gap between training and validation performance against depth for the slow group. The gap widens as depth grows, and the validation curve on the slow group flattens or turns down before it does on the fast group. That is the pattern behind choosing `maxDepth=6`.

## Open questions

Is a velocity-dependent depth worth the added complexity once there is more history per item? Would monotonic constraints on a few features give the smoothness we want without a hard cap? Does the stockout label itself, which is noisy because recorded zero stock is not always a true stockout, cause part of the overfitting, so that cleaning labels would help more than any depth change? None of these are decided. They are the places to look if the current setting stops being good enough.

## Summary for a later session

Keep `maxDepth=6` in the stockout-model. It exists because deeper trees overfit on sparse slow-moving SKUs, and the long tail drives a large part of the proposed replenishment orders. Test any change on a time-based split with the slow SKUs reported separately, change one parameter at a time, and write down new evidence here before altering the value.
