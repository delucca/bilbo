---
id: 01K69HJTKTTW3FVR40B1GT6HPX
created: 2025-09-28T22:32-03:00
sources:
  - "code: progress/views.py"
---

# progress-api reference

progress-api is the Django service in ClassroomCompass that answers one question: how far along is a given student against the curriculum standards. Teachers see its output in the Vue.js dashboard, and the exercise suggester reads from it too. This note is the quick reference for what it exposes and how it fits with the rest of the stack.

## Main endpoint

The endpoint most callers need is `GET /api/v2/students/{id}/progress`. It returns the per-standard mastery of one student. The `{id}` in the path is the student's identifier in ClassroomCompass, not a school-issued number. The response is a list with one entry per curriculum standard the student has been assessed on, each with a mastery value and enough context to label it in the UI.

## What the response means

Mastery is a per-standard figure, not a grade. A teacher reading it should treat it as an estimate of how secure the student is on that standard right now. It moves as new exercise results arrive, so two calls a day apart can differ without anything being wrong.

## Standards that are missing

If a standard does not appear for a student, it usually means the student has not been assessed on it yet. It does not mean zero mastery. The dashboard should show these as "not started", and any client code should keep the two cases apart.

## Who calls it

The Vue.js dashboard calls it when a teacher opens a student page. The exercise suggestion code calls it to decide which standards are weakest. Reports and exports that list many students loop over the students and call it per student, so keep an eye on that pattern when adding bulk features.

## Where the data comes from

Mastery values are not computed inside the request. They are produced by background work run through Celery, which takes recorded exercise results and updates the stored mastery per student and standard. The endpoint reads the stored result. If a value looks stale, look at the Celery side first, not the view.

## Role of scikit-learn

The models that turn raw results into a mastery estimate use scikit-learn. They run in the Celery workers, not in progress-api itself. The API process does no model fitting and should not import the training code on the request path.

## Role of Elasticsearch

Elasticsearch holds the searchable curriculum standards and is used to label and look up standards. progress-api uses it for standard descriptions and grouping, not for storing mastery. If standard labels are blank in a response, check the index before suspecting the mastery data.

## Permissions

Access is limited to teachers who teach the student in question, plus school administrators. A request for a student outside the caller's classes should be refused rather than return an empty list. When adding a new caller, make sure it authenticates as a teacher or an administrator, not as a shared service user.

## Versioning

The v2 in the path is the current version. Older clients may still talk to v1 in places. Do not change the shape of a v2 response in place; add fields, and bump the version for anything that removes or renames them. The Vue.js front end assumes the fields it already reads are stable.

## Performance notes

A single call is cheap because it reads stored values. The cost comes from callers that fan out over a whole class. If you need class-wide numbers, prefer a purpose-built aggregate over many single-student calls, and cache on the client side for the length of a page view.

## Debugging checklist

When someone reports wrong numbers, go in this order. First confirm the student identifier is the right one. Then check whether the standard was assessed at all. Then look at when the Celery job last ran for that student. Only after that look at the view code. Most reports have turned out to be stale or missing background results.

## Open points

Nothing here is a known bug. The things worth watching are the fan-out pattern in exports, the handling of unassessed standards in new client code, and keeping Elasticsearch labels in step with the standards that mastery is stored against.
