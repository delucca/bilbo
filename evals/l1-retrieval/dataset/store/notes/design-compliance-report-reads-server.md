---
id: 01M1FKKHYAPQ47XVMKWEFQ2T7W
created: 2026-09-01T20:06-03:00
---

# compliance-report-api design

compliance-report-api is the read-only reporting service in LabNotebook Sync. Compliance officers and research scientists use it to pull audit trail reports for notebook entries and instrument output. It answers questions like who changed an entry, when, what the instrument reported at that time, and whether the sync pipeline saw gaps. This note records how the service is shaped, where its data comes from, and the rules that keep it safe to run next to the audit pipeline. It is written fast, so treat it as working notes rather than a spec.

The central rule: compliance-report-api reads from a SQL Server read replica using the connection string name `ReportReadOnly`, and it never writes to audit tables. Everything else in the design follows from that.

## Purpose and scope

The service exists so that reporting load never touches the primary database that the sync workers and the audit writer use. Audit tables are the legal record. Anything that could slow them down, lock them, or alter them is out of bounds for a reporting path.

What it does:
- Serves report queries over HTTP for compliance officers and scientists.
- Builds report documents (tabular and exportable) from audit rows, notebook entry metadata and instrument run metadata.
- Stores generated export files in Azure Blob Storage so large reports can be downloaded later instead of streamed.

What it does not do:
- It does not write, update or delete anything in audit tables.
- It does not create audit events itself. Even "report was generated" bookkeeping, if we need it, goes through the normal audit writer path, not through this service's database access.
- It does not consume instrument data directly. Instrument output arrives through the sync pipeline and is already in the database by the time a report is built.

## Data access

All database reads go through the connection string named `ReportReadOnly`. That name resolves to the read replica. There should be no second connection string in this service's configuration. If someone adds one, that is a review flag.

Practical consequences:
- The database login behind `ReportReadOnly` should have read permission only. Even if code in the service tried to write, the login should refuse. Do not rely on the code alone to keep the guarantee.
- Replica lag is real. A report run right after an edit may not show that edit yet. The API should return the time the data is current as of, so an officer can tell a stale report from a missing event. Do not hide this lag.
- Queries should be written so they do not hold long locks or run unbounded scans. Page results and require a date range for broad queries.
- Use read-only intent on the connection so a mistaken failover to a writable node is still treated as read use.
- Prefer plain parameterised queries or a thin data layer. Avoid any ORM feature that tracks changes and tries to save them. In the .NET code, use no-tracking reads by default and do not expose a save call on the reporting context.

## Service layout

The service is a C# ASP.NET Core application on .NET. It keeps the layers small:
- Controllers or minimal endpoints that validate input, check the caller's role, and hand off to a query service.
- Query services that build the SQL for each report type and shape the rows into response models.
- A report builder that turns result sets into export files and writes them to Blob Storage.
- A small background piece that cleans up expired export files.

Keep report types as separate classes with one query each. Compliance people ask for oddly specific views and it is easier to audit one query per report than a general query builder.

## Reports and exports

Small reports are returned in the response body. Large ones are produced as an export: the API accepts the request, builds the file, puts it in Azure Blob Storage, and returns a reference the caller can poll or follow. Export files hold regulated data, so storage access is private and downloads go through short-lived access granted by the API after an authorisation check.

Export content must be reproducible. Each export carries the parameters used, the replica data-as-of time, the service version, and who asked. That way an inspector can see how a given file was made and rerun it.

Retention for export files should be set by the compliance team, not by developers. Make it configuration, and make the cleanup job log what it removes. Removing an export file is not removing audit data; the audit tables are untouched.

## Messaging

Report requests that need long runs can be queued over RabbitMQ so the HTTP call returns quickly. The worker that consumes them is part of this service and uses the same `ReportReadOnly` access. Rules for the queue side:
- Messages carry the request parameters and the requester identity, not result data.
- Handlers are idempotent: running the same request twice produces the same export, or reuses the first one.
- Failed messages go to a dead-letter queue and are visible to operators. Do not retry forever against a replica that is down.
- The service publishes nothing that other components treat as an audit event.

## Security and access

- Callers authenticate through the platform's normal identity setup. Role checks decide who may run which report. Compliance officers see cross-project reports; scientists see their own projects.
- Every report request is logged by the API in its own application log with caller, report type and parameters. This is operational logging, separate from the audit tables, which this service cannot write.
- No credentials in source. The `ReportReadOnly` connection string comes from configuration or a secret store in each environment.
- Sensitive values in responses follow the same masking rules as the rest of LabNotebook Sync.

## Testing and checks

- Integration tests run against a test database and use a login that has read access only. A test should confirm that an attempted write through the service's connection fails.
- Add a test or a startup check that the only configured connection string for reporting is `ReportReadOnly`.
- Test replica lag handling by faking the data-as-of time and checking it appears in the response and in export metadata.
- Load test the heaviest report types against a replica-sized dataset, watching that queries stay bounded.

## Gotchas and open items

- Replica lag can make a report look like it is missing an audit event. Check the data-as-of time before concluding anything is lost.
- If the replica is unavailable, the service should fail the request with a clear error. It must not fall back to the primary. That fallback would break the whole point of the design.
- Query timeouts need tuning per report. A single default will either cut off legitimate large reports or let a bad query run too long.
- Open: agree with compliance on export retention and on what metadata inspectors want embedded in each file.
- Open: decide whether report-generation events should be recorded in the audit trail, and if so, which component writes them.
