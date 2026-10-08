---
id: 01KC12AVTPV8F1XZ7PK00WYK59
created: 2025-12-09T05:06-03:00
---

# mastery-model: stale artifact after scikit-learn upgrade

Loading a mastery-model artifact after a scikit-learn upgrade emits `InconsistentVersionWarning: Trying to unpickle estimator`. When you see that warning, the predictions from that mastery-model artifact should not be trusted until the model is retrained. The artifact was pickled under the older scikit-learn, and the new version reads it anyway, so nothing crashes. That is the trap: the service keeps answering, and the answers can be wrong.

## What you see

The warning shows up in the logs at load time, in whichever process loads the artifact. That can be a Django web worker or a Celery worker. The exact text is:

```
InconsistentVersionWarning: Trying to unpickle estimator
```

The rest of the line names the estimator class and the two scikit-learn versions involved. No exception follows. Requests still return mastery estimates and next-exercise suggestions, and they look normal.

## Why it matters

Teachers use the mastery estimates to decide which exercises a student gets next. A silently wrong estimate sends a student to work that is too easy or too hard, and nobody notices because the output has the usual shape. Internal estimator attributes can change between scikit-learn releases. An unpickled object may then have missing or reinterpreted fields, and scikit-learn gives no guarantee it behaves correctly.

## What to do

- Treat the warning as a hard signal, not noise. Do not filter it out in logging config.
- Do not serve or rely on predictions from the loaded artifact.
- Retrain the mastery-model with the new scikit-learn version and write out a fresh artifact.
- Redeploy the web and Celery workers so they all load the new artifact, not a cached old one.
- Confirm the warning is gone from the logs after the reload.

## Prevention

Bump scikit-learn and retrain in the same change. Do not upgrade the library alone. Record the scikit-learn version next to each artifact so a mismatch can be seen before load. A startup check that fails loudly when the warning fires would be better than relying on someone reading logs.

## Open points

Nothing yet stops a deploy that pairs a new scikit-learn with an old artifact. Elasticsearch-backed suggestion lookups are not affected by this warning; the problem is only in the pickled estimator. If a retrain is not possible right away, the safer fallback is to pin scikit-learn back to the version the artifact was built with, rather than serve from the mismatched one.
