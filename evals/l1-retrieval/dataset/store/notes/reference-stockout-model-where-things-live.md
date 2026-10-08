---
id: 01JRRMW3AD0J5BTFZB55CN0W8R
created: 2025-04-13T19:38-03:00
---

# stockout-model: where the pieces live

This is a map of the stockout-model component in ShelfSense, written quickly so the next person or agent does not have to rediscover where things sit. It points at areas, not at exact files, and it avoids values that change. When you need a precise location, search the repository for the names mentioned here. Check the code before trusting anything below, because the layout drifts.

stockout-model is the part of ShelfSense that estimates, per store and per product, how likely a shelf is to run empty within the planning horizon. The replenishment side then turns those estimates into suggested orders for merchandising analysts at grocery chains. The model itself is a Scala codebase running on Apache Spark. It reads and writes Delta Lake tables. Results are published to Snowflake, where the analyst-facing tools read them. Airflow schedules the runs. The scheduling side has its own note, [[shelfsense-dags-airflow-nightly-revised]]. Read that note for DAG layout and timing, and read this one for what the model code and data look like.

## Repository layout in general terms

The Scala code is split along the usual lines for a Spark project, and stockout-model follows the same shape. There is a build definition at the top that declares the modules and their dependencies. Under it there are a few logical areas.

The first area is the shared domain code. It holds the case classes for stores, products, calendar days, inventory snapshots, sales lines and the prediction record. If a column name or type looks wrong somewhere, check here first, because most of the job code builds Datasets from these classes or maps rows into them. Do not change a field here casually. The Delta tables and the Snowflake tables downstream depend on the same names, and a rename here ripples through all of them.

The second area is the feature code. It takes cleaned input tables and produces the feature table the model trains and scores on. It is organized by feature family: recent sales velocity, days of cover, promotion flags, calendar and holiday effects, supplier lead time signals, weather or local event signals when they are available, and store attributes. Each family tends to live in its own object or small group of objects with a function that takes a DataFrame and returns a DataFrame with extra columns. A top-level feature assembly step joins them. If you want to add a signal, add a new family and register it in the assembly step. Do not bolt columns onto an existing family.

The third area is training. It holds the code that builds the training set from historical features and labels, fits the model, evaluates it, and writes the model artifact and its metadata. The labels come from observed out-of-stock events, which is a messier thing than it sounds. See the section on labels below.

The fourth area is scoring. It loads the current model artifact, reads the current feature table, produces predictions, and writes them to the predictions table. Scoring is the part that runs every day. Training runs less often.

The fifth area is the publishing code. It copies the predictions from Delta into Snowflake and keeps the Snowflake side consistent with what Delta holds. It is deliberately thin. Business logic should not live here.

There is also a test area mirroring the main layout. Unit tests use small in-memory DataFrames built with a local Spark session. A few heavier tests exercise whole jobs against small fixture tables. Fixtures live next to the tests, not with the main code.

Configuration is kept apart from code. Environment-specific values such as storage locations, Snowflake connection targets, and table names per environment are read from configuration files and from the orchestrator, not hard-coded. If you find a literal location in Scala code, treat it as a bug or a leftover.

## Data inputs and where they come from

The model does not own its raw data. It consumes tables produced by upstream ingestion jobs, and those jobs belong to other parts of ShelfSense. What stockout-model sees is a set of Delta tables, usually described as the cleaned or curated layer.

The main inputs are these.

- Point-of-sale sales, at store, product and day grain, sometimes finer. Returns and voids are handled upstream, but check how, because the model assumes net sales.
- Inventory snapshots, at store and product grain, taken at a regular cadence. These are the on-hand counts the retailer reports. They are noisy. Counts can be stale, negative, or missing for stores that report late.
- Receipts and orders in transit, which say what is on the way and when.
- Product master data, including category, pack size, shelf life class and substitution groups.
- Store master data, including format, region, opening status and trading calendar.
- Promotion calendars, which say which products are on deal in which stores and when.
- External calendar data such as public holidays and local events.

Each of these has an owner outside stockout-model. When a number looks off, find out whether the problem is in the input or in the feature code before changing anything. Most of the time spent on this component goes to input quality, not to the model.

The curated tables are partitioned by date and often by chain or region. Reads in the feature code rely on partition pruning, so filters on the date column should be pushed down early and written in a form Spark can prune. A filter written through a function on the date column often kills pruning and turns a quick read into a full scan. This has bitten people more than once.

Schema evolution on the Delta inputs is allowed upstream. That means new columns can show up without notice. The feature code should select the columns it needs by name and not use star selects that pass unknown columns along. A star select into the feature table is how stray columns end up in training data.

## Features, labels and training data

Feature building is the heaviest step in the Spark sense. It joins several large tables at store, product and day grain and computes rolling windows. A few general observations about it:

- Rolling windows are computed with window functions over partitions by store and product. These are expensive, and skew is real: a handful of very large stores or very fast-moving products can make a few tasks run far longer than the rest. If a feature job is slow, look at the task time spread in the Spark UI before you add more executors.
- Features are computed as of a cutoff date and must use only information available at that cutoff. Leakage is the most common modelling mistake here. When adding a feature, ask whether the value would have been known at the time the prediction would have been made. Inventory snapshots and receipts are the usual trouble, because they are often corrected after the fact. Use the version of the data that was current at the time, not the corrected one, when building training sets.
- Missing values are meaningful. A product with no sales in the window is different from a product with no record. The feature code makes this distinction in specific places, and the distinction matters to the model. Do not blanket-fill nulls with zero.
- The feature table is written to Delta, partitioned by date, so scoring can read the latest slice and training can read a range.

Labels deserve their own paragraph. A stockout is not recorded directly in most retailer data. It is inferred from patterns: on-hand at zero, sales dropping to zero when history says they should not, or an explicit out-of-stock flag where a chain provides one. The label code encodes these rules, and different chains get different treatment because their data differs. The label rules sit in the training area and they are the thing most worth reading before you trust any evaluation number. If the label definition changes, old models and new models are not comparable, and the evaluation history needs to be treated as broken at that point.

Training data assembly joins the feature table at the cutoff with the label looking forward over the horizon. There is a split strategy based on time, not random, because random splits leak across days for the same store and product. The validation window is later than the training window. The test window, if used, is later still. Keep it that way.

The model family is a gradient boosted tree approach running through Spark. The specific library and the specific hyperparameters are in the training configuration, and they change over time, so I am not recording them here. What matters for orientation is that the fitted pipeline includes the feature transformation stages, so the artifact is self-contained and scoring does not have to redo the encoding by hand. If you change the feature set, the artifact has to be retrained. An old artifact with a new feature table will fail or, worse, silently score with the wrong columns, depending on how the schema check is set up. Look at how the scoring code validates the input schema against the artifact metadata before relying on it.

Evaluation produces metrics at several levels: overall, by category, by chain, and by store format. The numbers are written next to the artifact as metadata and, in some setups, into a metrics table. Analysts care most about whether the highest-risk items are really the ones that run out, so ranking-style metrics matter more than plain accuracy. Because stockouts are rare relative to non-stockouts, plain accuracy tells you nothing.

## Scoring, outputs and Snowflake

Scoring runs on a schedule, usually overnight, so analysts have fresh predictions at the start of the working day. The scoring job does roughly this: figure out which date to score, read the feature slice for it, load the current artifact, produce a probability and a few supporting fields per store and product, and write the result to the predictions Delta table. The supporting fields include the expected days until stockout and a short list of the main drivers, which the analyst interface displays so the number is not a black box.

Predictions are written in a way that is safe to rerun. A rerun for the same date replaces that date's slice and leaves other dates alone. This is done with a partition overwrite or a merge, depending on the table. Check which before you rerun anything, because a wrong overwrite mode on a Delta table can wipe more than you meant. Delta's time travel will usually let you recover, but only within the retention window, and nobody should count on that.

The replenishment order generator is downstream of the predictions. It reads predictions and applies business rules: minimum order quantities, pack sizes, supplier schedules, and budget or capacity limits. Those rules live in the replenishment area, not in stockout-model. If an analyst says the orders look wrong, first establish whether the prediction was wrong or the ordering rule was wrong. They are separate code and have separate owners.

Publishing to Snowflake takes the latest predictions and loads them into tables the analyst tools query. A few things to know:

- The Snowflake tables mirror the Delta prediction schema closely. Type differences exist, especially around timestamps and decimals, and the publishing code handles the conversion. A type change in Delta needs a matching change on the Snowflake side or the load fails or truncates.
- The load is staged and then swapped or merged in, so analysts do not see half-loaded data. If a load fails midway, the previous good state should still be what they see. Verify this rather than assuming it when you touch the publishing path.
- Credentials for Snowflake come from the secrets mechanism used by the orchestrator and the cluster, not from the repository. Never paste connection details into code, notes or tickets.
- Views on top of the published tables are maintained separately from the load. The analyst tools read the views. If a column is added to the table, it will not show up in the tool until the view is updated.

A word on consistency. Delta is the source of truth for predictions. Snowflake is a serving copy. When the two disagree, trust Delta and republish. Do not hand-edit Snowflake rows.

## Orchestration, environments and operations

Airflow runs the pipeline. The DAG definitions are not inside the stockout-model code; they live in the orchestration repository or area, and the related note describes how the nightly flow is arranged. From the model's side, the contract is simple: each job is a Spark application with a main class, taking a small set of arguments such as the run date, the environment and the configuration location. Airflow submits the job to the cluster and watches it. The jobs themselves should not know about Airflow.

Training is scheduled separately from scoring and can also be triggered by hand when something has changed, such as a new feature family or a label fix. A manual training run should write its artifact under a distinct name or tag, not over the current one. Promotion of a new artifact to current is a deliberate step, and it is the point at which someone looks at the evaluation output. Keep that gate. Automatic promotion without a human look has produced bad scoring runs elsewhere, and the cost shows up as bad orders at stores.

Environments follow the normal pattern: a development one, a staging or test one, and production. Table names and storage locations differ per environment and come from configuration. Staging data is a sample, not a full copy, so volume problems only show up in production. Keep that in mind when a job passes in staging and then runs long or runs out of memory later.

Cluster sizing and Spark settings are in the job submission configuration. Typical trouble spots are shuffle partition counts for the large joins, memory for the window computations, and small-file buildup in the Delta tables written every day. Compaction and cleanup of old files are handled by maintenance jobs, which are separate from the model jobs. If reads of a table get slower over weeks without any code change, suspect file layout before suspecting the code.

Logging goes through the standard Spark logging into the cluster log storage, and Airflow keeps its own task logs. When a run fails, read the Airflow task log first to see which step failed, then the Spark driver log for that application for the actual exception. Executor logs matter mostly for out-of-memory problems and for serialization errors.

Monitoring has a few layers. Airflow alerts on task failure and on late completion. There are data checks after feature building and after scoring, such as non-empty output, expected store coverage, and a sanity range on the predicted probabilities. There is also a drift view that compares the distribution of scores and of key features against recent history. A sudden shift in the share of items flagged as high risk is usually an input problem, such as a late inventory feed or a missing chain, not a real change in stockouts. Check coverage by chain before anyone starts retuning.

## Gotchas and habits worth keeping

These are the things that cost time in the past. They are stated generally on purpose.

- Time zones. Stores sit in different time zones, and sales days are local days. The pipeline runs in a single zone. Day boundaries must be handled in the feature code with the store's zone, and mixing the two quietly shifts late-evening sales into the wrong day. When a store looks off by a day, check this.
- Late-arriving data. Some chains deliver sales and inventory late or in corrections. The scoring run uses what is there at run time. Reruns after late data lands will give different predictions for the same date. That is expected, so do not treat it as nondeterminism.
- Product identity. Products get re-coded, merged and split by chains. The product master carries mapping information, and the feature code relies on it for continuity of history. A product that looks new may have a long history under an older code. Ignoring the mapping makes new-looking items score as unknown.
- New stores and new products. History is short or missing. The model handles them through fallback features based on category and format averages. Predictions for them are less reliable, and the interface is meant to say so. Do not tune the main model to fix these cases. Handle them in the fallback logic.
- Closed and seasonal items. Items out of range, discontinued, or seasonal and not currently stocked should not be predicted as stockouts. The filtering for this happens before scoring, based on the range data. If analysts report predictions for items a store does not carry, look at the range filter and at the freshness of the range data.
- Promotions. Promotion calendars are often entered late or changed late. A prediction made before a promotion was entered will understate demand. This is a known limit and not a bug in the model code.
- Delta schema changes. Adding a column is easy. Changing a type or dropping a column breaks readers. Coordinate with the downstream owners and the publishing code before doing either.
- Local Spark tests are not production. Behaviour around partitioning, skew and memory differs a lot. Passing tests says the logic is right, not that the job will finish in time.
- Artifact and feature drift. The artifact and the feature code must match. Any time the feature code changes, ask whether the artifact needs retraining and whether scoring will notice a mismatch.
- Reproducibility. Training runs should record the data range, the code revision and the configuration used, in the artifact metadata. When a past model needs to be explained, that metadata is the only trustworthy record. Keep it filled in.

## Finding your way around quickly

If you are new to the component, or an agent starting cold, this order works.

Start from the build definition to see the modules. Open the domain classes to learn the vocabulary. Then read the top-level job entry points, which are the main classes that Airflow launches. They are short and show the shape of each stage: what is read, what is written, which configuration keys are used. From there, follow into the feature assembly step, then the label code, then the training and scoring code. Read the publishing code last, since it is the simplest.

For data questions, look for the table definitions or the schema documentation maintained next to the ingestion jobs, and compare them to what the feature code selects. For scheduling questions, go to the DAG note mentioned above and then to the orchestration code itself. For questions on what analysts see, go to the Snowflake views and the analyst tool configuration, which are outside this component.

When you change something, a short checklist helps: does it alter the feature set, the label rules, or the output schema? If the feature set or label rules change, plan a retrain and a re-evaluation, and plan a deliberate promotion. If the output schema changes, update the Delta write, the Snowflake load and the views together, and tell the owners of the order generator. If none of those change, a code-only fix can usually go straight through the normal review and release path.

Things I did not check while writing this: the exact current module names, the current model library choices, and the present state of the maintenance jobs. Verify those in the code before relying on this map for them. Update this note when the layout changes, and keep it general. Specific values belong in configuration, not here.
