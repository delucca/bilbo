---
id: 01KY0WTTYDW2D6HH43WE8P4RBW
created: 2026-07-20T20:12-03:00
---

# slot-solver: where things live

slot-solver is the part of ClinicSlotter that takes a request for an outpatient appointment and finds a time that fits the clinician's availability and the room constraints. This note says where the pieces sit so you can start looking in the right place. It does not record tuning values or rules; read the code for those.

## Where the code sits

The solver is plain Ruby inside the Rails app, not a separate service. Look first under the app's domain or service layer, in a directory named after scheduling or the solver. The core is a small set of objects: one that gathers candidate slots, one that filters them by constraint, and one that ranks what is left. Controllers and jobs call a single entry point and should not reach into the inner objects.

Models for clinicians, rooms, appointments and availability windows are ordinary ActiveRecord classes in the usual models directory. The solver reads them but should not own their validations. If a rule is about whether a record is valid at all, it belongs on the model. If it is about whether two records can coexist at a time, it belongs in the solver's constraint code.

Constraint checks are kept as separate small classes or modules, one per kind: clinician availability, room availability, and overlap with existing bookings. Adding a new kind of constraint usually means adding one more of these and registering it where the others are listed.

## Data and storage

Everything the solver needs comes from MySQL through ActiveRecord. Availability is stored as windows, and existing appointments are rows with a start and an end. Time zone handling is a known source of confusion: check how the clinic's zone is applied before comparing times, and look at the existing helpers rather than writing new conversions.

Queries that load windows and bookings for a date range are the hot spot. If the solver feels slow, look at those scopes and the indexes behind them before touching the ranking logic.

## Background work and integration

Long or bulk runs go through Sidekiq. Worker classes live in the app's jobs or workers directory and are thin: they load the records, call the solver entry point, and save the result. Retries matter here, so a job that books a slot should be safe to run twice. Check how the existing workers guard against double booking before adding another.

FHIR comes in at the edges. Appointment, Schedule and Slot resources are mapped to and from the internal models in the integration or serializer code, not inside the solver. The solver works with internal records only. If an external system sends a request, the mapping layer converts it first.

Heroku runs the web and worker processes. Dyno and queue settings live in the process definition file and in config vars, not in the solver code. Check there when behavior differs between local and production.

## Tests and where to start

Specs for the solver sit in the spec tree mirroring the code layout. Constraint classes have unit specs; the entry point has broader scenario specs using factories for clinicians, rooms and windows. When changing a constraint, start from its unit spec and then run the scenario specs.

For a bug report, first reproduce it with factories in a scenario spec, then narrow down to the constraint that wrongly allowed or rejected the slot. For a performance report, start with the queries. For a booking collision, start with the Sidekiq worker and the database-level protection against overlaps.

## Open questions

Not yet confirmed: how much of the ranking is configurable per clinic, and whether any constraint logic is duplicated in the front-desk UI. Check both before changing behavior.
