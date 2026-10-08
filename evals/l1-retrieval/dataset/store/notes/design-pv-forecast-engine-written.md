---
id: 01K9H3AKKFRZCRRG8WHPZDHFH1
created: 2025-11-08T03:44-03:00
sources:
  - "code: Project.toml"
---

# pv-forecast-engine design

pv-forecast-engine is the part of GridHaven that predicts how much electricity a household solar array will produce over the coming hours and days. The battery scheduler reads its output and decides when to charge against the time-of-use tariff. The engine is written in Julia 1.10 and fits one gradient-boosted model per site cluster using the package EvoTrees.jl. This note records why it is shaped that way, how data moves through it, and what to remember before changing it.

The short version: sites are grouped into clusters by similarity, each cluster gets its own boosted tree model, and every site in a cluster shares that model. Site-specific differences are handled through input features and a small per-site correction step, not through separate models. Most of the design follows from that choice.

## Why one model per site cluster

There were three obvious options: one global model for everything, one model per site, or something in between. We went in between.

A single global model was the first thing tried. It was simple to train and serve, but it smeared together very different installations. A steep roof facing one way behaves nothing like a flat roof with a different orientation, and a site with heavy shading from trees behaves nothing like an open field mount. The global model learned averages. Its errors were large exactly where installers cared most, on sites with unusual geometry or shading.

One model per site looked attractive for accuracy and was bad in practice. New customers have almost no history, so their models would be useless for weeks. Sites with noisy or gappy telemetry produce unstable fits. And the number of models to train, version, store and monitor grows with the customer base, which makes the nightly job slow and the failure surface wide. Installers add sites constantly, so the operational cost would keep rising.

Clusters fix both problems. A cluster is a set of sites that look alike on the things that drive output: panel orientation and tilt bucket, installed capacity band, inverter type family, rough climate zone, and a shading profile summary. A new site is assigned to the nearest existing cluster on day one using its declared configuration, so it gets a reasonable forecast immediately. As it accumulates history, its own residuals feed the per-site correction, and it can be moved to a different cluster if the assignment turns out to be wrong.

Gradient-boosted trees were chosen over neural sequence models for reasons that are mostly practical. The tabular inputs (weather features, solar geometry, recent output lags, site descriptors) suit trees well. Training is fast enough on commodity machines to retrain every cluster regularly. The models are small, the behavior is easy to inspect through feature importance, and the failure modes are boring: a tree model does not extrapolate wildly outside the range it saw, which matters when a battery schedule depends on the result.

EvoTrees.jl was picked because it is native Julia, so there is no foreign-language bridge in the training or serving path. It handles the loss functions we need, including quantile objectives for uncertainty bands, and it trains on the CPU without special setup. Staying inside one language kept packaging simpler for the installer-facing deployments, where the runtime footprint matters.

### What a cluster is not

A cluster is not a geographic region and is not an installer's customer list. Neighbouring houses can land in different clusters if their roofs differ, and one installer's customers can be spread over many clusters. Do not tie anything else, such as permissions or billing, to cluster identity. Cluster membership is an internal modelling detail and is allowed to change between training runs.

## Data flow and feature construction

Telemetry comes from the inverters and meters at each site. Devices publish over MQTT to a gateway, and the gateway forwards into Azure IoT Hub. From there a consumer writes the readings into InfluxDB, which is the store the engine reads history from. The engine does not talk to devices and does not subscribe to MQTT itself. Keeping it behind InfluxDB means the engine can be re-run on past data without touching live ingestion, and a broken device connection shows up as missing data rather than as a crash in the forecaster.

The engine reads three kinds of input.

- Site telemetry history from InfluxDB: measured generation, and where available the inverter status, which helps separate real zero output from a device that was offline.
- Weather inputs: forecast irradiance, cloud cover, temperature and wind for the forecast horizon, and the matching observed or reanalysis values for training periods.
- Static site descriptors: capacity, orientation, tilt, location, inverter family and the shading summary, held in the site registry that the installer-facing Svelte application edits.

Feature construction is a pure step: given a site, a time window and the inputs above, it returns a table. It computes solar geometry for the site location, converts forecast irradiance into an estimate of plane-of-array irradiance for that site's tilt and orientation, adds time-of-day and season encodings, adds recent lagged output where it exists, and attaches the site descriptors. Sites without enough history simply have missing lag features, and the trees handle that without special treatment.

One rule we hold to: features available at training time must be available at forecast time with the same meaning. The easiest way to break this is lagged output. At training time you can look up exactly what the site produced an hour earlier. At forecast time, for a horizon several hours out, that value does not exist yet. So lag features are built relative to the forecast issue time, not relative to each target timestamp, and the training table is constructed by simulating issue times. Early versions leaked future information through this and looked excellent offline and poor live. If offline accuracy suddenly looks too good after a feature change, suspect leakage first.

### Handling bad telemetry

Real sites produce junk. Meters drop out, inverters report stale values, clocks drift, and curtailment clips output when the grid operator limits export. The engine treats these differently.

Gaps are left as gaps. We do not interpolate across long outages, because a filled-in curve teaches the model to expect output that never happened. Short gaps within a single reading cadence may be bridged for lag features only, never for the training target.

Stale repeats, where a device reports the same value over and over, are detected and dropped from training targets. Clock drift is corrected where the device reports its own time and flagged otherwise.

Curtailment is the awkward case. When output is clipped, measured generation understates what the panels could have produced. If those hours go into the target unchanged, the model learns that clear sunny conditions sometimes give low output. We mark suspected curtailed periods and exclude them from training for the affected sites. They stay in the evaluation set but are reported separately, since the scheduler cares about what actually reaches the house or battery.

## Training, serving and the per-site correction

Training runs as a scheduled job. For each cluster it assembles the training table from the member sites, fits the EvoTrees.jl model, evaluates it on a held-out recent period, and writes the artifact along with its metadata. Splits are by time, never random, because random splits let neighbouring hours of the same day land on both sides and inflate the scores. The held-out period is always the most recent stretch, so the score reflects how the model would have done had it been deployed.

A new model does not replace the serving one automatically. It is promoted only if it is not worse than the current model on the held-out period by a margin we consider noise, and only if no member site got markedly worse. This second check matters because a cluster-level average can improve while one site in the cluster degrades. When a candidate fails, the job keeps the old model, records the reason, and raises a warning for a human to look at. We prefer a slightly stale model to a surprising one, since the battery schedule is derived from the forecast and a bad forecast turns into a bad charging decision.

Hyperparameters are set per cluster family rather than tuned from scratch for every cluster. Fully automatic tuning per cluster was tried and gave unstable results on small clusters, where the validation set is thin and the chosen settings jumped around between runs. The current approach uses a shared default and a short list of overrides for clusters with unusual size or noise. Treat any change to the defaults as a change affecting every cluster, and compare across all of them before merging.

### Uncertainty

The scheduler needs more than a single number. Charging the battery overnight from the grid is a bet that the next day will not be sunny enough to fill it from the panels, and that bet should depend on how sure the forecast is. The engine therefore produces a central estimate and a lower and upper band. The bands come from quantile objectives in the boosted models rather than from a Gaussian assumption, because solar error is heavily skewed: cloud can only remove output from the clear-sky ceiling, so the downside tail is long and the upside is bounded.

Bands are checked for calibration as part of the evaluation. A nominal band that covers noticeably fewer actual outcomes than it claims is a bug to fix before shipping, even if the central error looks fine. The scheduler consumes the lower band for cautious decisions, so a band that is too narrow quietly makes the system overconfident.

### Per-site correction

Because sites share a cluster model, each site keeps a small correction applied after the model output. It captures persistent bias that the cluster cannot see, such as a partly blocked panel string, a slightly different real tilt than declared, or a soiling pattern. It is estimated from the site's own recent residuals, bounded so it cannot grow large, and shrunk toward zero when the site has little history. The intent is a nudge, not a second model.

The correction is deliberately simple. Earlier we tried fitting a second boosted model per site on the residuals, which brought back the per-site problems described above: too many artifacts, fragile for new sites, and prone to chasing noise. A slowly updated bias term gets most of the benefit with almost none of the cost.

If a site's correction stays large for a long time, that is a signal rather than something to absorb. It usually means the site is in the wrong cluster, its declared configuration is wrong, or the hardware changed. The engine reports such sites so an installer can check them, and a reassignment to another cluster is the normal fix.

## Serving, interfaces and operating notes

At forecast time the engine loads the current model for each requested site's cluster, builds features for the site at the issue time, predicts, applies the site correction, and returns the series with bands. Results are written back to InfluxDB so that the scheduler, the installer dashboards built in Svelte, and any later analysis read the same stored values. We store forecasts rather than recomputing them on demand, because comparing what was predicted with what happened later requires the original prediction, not a re-run with newer information.

Each stored forecast carries the model version and the issue time. This is not optional. When someone asks why the battery charged at a strange hour, the answer starts with which forecast the scheduler saw, and that needs the version and the issue time recorded with it.

The scheduler is a separate component and the boundary is narrow: it reads forecast series and bands and never calls into the model code. Anything the scheduler wants that the engine does not provide, such as a different horizon or a different quantile, is a request to the engine rather than a reason to reimplement forecasting elsewhere.

### Failure behaviour

The engine is allowed to fail visibly, but it should not leave the scheduler with nothing. The fallbacks, in order of preference:

- If a site's own features are partly missing, predict with what exists and let the trees handle the missing values.
- If the weather input for the horizon is unavailable, fall back to a climatological profile for the site's cluster and mark the forecast as degraded, with wider bands.
- If the cluster model cannot be loaded, use the previous version of that model if one exists.
- If nothing is available for a site, return an explicit no-forecast result. Never return zeros, because zero looks like a real prediction of an overcast day and would push the scheduler toward grid charging.

The degraded flag has to survive all the way to the scheduler and the dashboards. A forecast that quietly substitutes a fallback is worse than one that says so.

### Things to remember when changing it

Keep the language and package pairing in mind. The engine is Julia 1.10, and the model code depends on EvoTrees.jl. Upgrading either can change numerical results slightly, and saved model artifacts are tied to the package that wrote them. Do not assume an old artifact loads after a package upgrade. Treat an upgrade as a retrain-everything event and compare outputs before and after on a fixed set of sites.

First-run latency in Julia is a real cost for short-lived processes, because compilation happens on the first call. The serving path avoids paying it per request by staying in a long-running process, and the training job is long enough that it does not matter. If someone wraps the engine in a short-lived command for convenience, expect it to look slow for reasons unrelated to the model.

Be careful with time. All internal timestamps are in UTC, and conversion to local time happens only at the edges, for the dashboards and for matching the tariff schedule. Daylight saving changes have bitten the tariff side, not the engine, but a local-time feature sneaking into the model would reintroduce the problem. Solar geometry is computed from UTC and location, never from local clock time.

Cluster reassignment changes which model serves a site and therefore changes its forecast abruptly. When moving sites, do it in the training job's own pass rather than by hand, so the per-site correction is reset or re-estimated against the new cluster instead of carrying over a bias that belonged to the old one.

When evaluating a change, look at more than the average error. Report error by horizon, by cluster, by season and by sky condition, and look at the worst sites. A change that helps clear days and hurts partly cloudy days can improve the average and still make the battery schedule worse, since partly cloudy days are where the charging decision is hardest and where forecast errors cost the most money.

### Open questions

- Whether clusters should be learned automatically from behaviour instead of being defined from declared configuration. Declared configuration is easy to explain to installers but can be wrong; behaviour-based grouping fixes that but makes cluster identity less stable.
- Whether a shared model across clusters with the cluster as a feature would match the per-cluster fits while cutting the number of artifacts. It was not tried seriously because per-cluster fits are easy to reason about and isolate failures.
- Whether the per-site correction should be allowed to vary by time of day. Morning and afternoon shading differ, and a single bias term cannot capture that, but a richer correction risks drifting back toward per-site models.
- How to treat sites with batteries and export limits in the training target, so curtailment exclusion stays accurate as more installations have controllable export.

None of these is blocking. The current design is good enough that the main work is keeping telemetry clean, keeping the cluster assignments honest, and not letting the per-site correction grow into a hidden second model.
