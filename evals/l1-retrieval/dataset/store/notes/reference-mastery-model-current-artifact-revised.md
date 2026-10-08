---
id: 01KDSK6DAGYH6BERHNPQYXYTQ7
created: 2025-12-31T03:58-03:00
---

# mastery-model artifact location

This note replaces the earlier note about "mastery model current artifact". The mastery-model artifact now lives at `/srv/classroom/models/mastery.joblib`. The old location, `/var/models/mastery/current.joblib`, is no longer the right one and should not be used by anything new.

The rest of this note is what someone touching mastery-model in ClassroomCompass needs to know about where the file is, who reads it and what to do when it is missing or stale.

## Current path

The artifact is a single joblib file at `/srv/classroom/models/mastery.joblib`. It holds the fitted scikit-learn estimator that mastery-model uses to estimate how well a student has mastered a curriculum standard. There is one file, with no per-version subfolder under the new location. The file name no longer has "current" in it, so do not look for a symlink or a pointer file named that way.

## Previous path

Before the move the artifact was read from `/var/models/mastery/current.joblib`. The old note, titled around "mastery model current artifact", gave that value. Treat that note as superseded. If you find the old path in a config file, a deploy script, a runbook or a comment, it is out of date and should be changed to `/srv/classroom/models/mastery.joblib`.

## Why it matters

Mastery-model feeds the part of the product that teachers see most: the per-student progress view against curriculum standards and the list of suggested next exercises. If a process loads the wrong path, one of two things happens. Either it fails to start the scoring code, or, worse, it loads a stale copy left behind at the old location and quietly gives suggestions from an older fit. The second case is the one to watch for, because nothing crashes.

## Who reads the file

The readers are the Django application and the Celery workers. The Django side loads the model when it needs to score a student on a request, or when a view asks for suggested exercises. The Celery side loads it for batch work, such as recomputing mastery for a whole class after new results are imported. Both should get the location from settings rather than hard-coding it in a task or a view. When changing the path, change the setting and check that no module has its own copy of the string.

## Where the path is configured

Keep the path in one Django setting, read from the environment with a default that points at the new location. Celery workers use the same Django settings module, so they pick it up without extra work. Do not scatter the path across task modules. If you find a second definition, remove it and make it use the setting.

## Deployment notes

The directory under `/srv/classroom` has to exist on every host that runs the web app or a worker, and the service accounts for both need read access to the file. Write access should be limited to whatever job publishes a new model. After deployment, make sure old hosts or images do not still carry a file at the old location that something might fall back to. Removing the old file is better than leaving it as a trap, but only once nothing reads it.

## Updating the model

When the model is retrained, the new fit replaces the file at `/srv/classroom/models/mastery.joblib`. Write it to a temporary name in the same directory first and then rename it into place, so a reader never sees a half-written file. Processes that already loaded the model keep the old one in memory until they restart or reload, so long-running Celery workers need a restart or an explicit reload step after a publish. Web workers behave the same way.

## Interaction with Elasticsearch

The artifact itself does not depend on Elasticsearch. Exercise and standard lookups for the suggestion step go through the search index, and mastery-model supplies the scores that rank the candidates. A move of the model file does not require reindexing. If suggestions look odd after a move, look at the model load first and the index second.

## Interaction with the Vue front end

The Vue.js front end never touches the file and does not know where it is. It only calls the Django API for progress and suggestions. A wrong or missing artifact therefore shows up in the UI as an API error or as empty or odd suggestions, not as anything about a path. Do not add the path to any client-side config.

## Failure symptoms

If the file is missing at the new location, the load step fails and the scoring endpoints return errors, while batch recomputation tasks fail and are retried by Celery. If the file exists but is from an old fit, the symptoms are softer: mastery values that do not move after new results, or suggestions teachers say are repeating. In both cases first confirm which path the process is actually using and that it is `/srv/classroom/models/mastery.joblib`.

## Compatibility of the file

A joblib file is tied to the scikit-learn version and to the Python environment that wrote it. When the library is upgraded, the artifact may need to be re-fitted and re-published at the same path. Keep the training environment and the serving environment on matching versions, or loading can fail or give different results. This is unrelated to the path move but often gets mistaken for it.

## Things to avoid

Do not point a worker at the old path as a quick fix. Do not copy the artifact into the repository. Do not keep two live copies at different locations, since they drift apart. Do not hard-code the path in tests; use the setting, or a small fixture model, so tests do not depend on the host layout.

## Open items

Check docs, runbooks and deploy templates for remaining mentions of `/var/models/mastery/current.joblib` and update them. Decide whether the publish job should also trigger a worker reload. Until then, restart workers by hand after each publish.
