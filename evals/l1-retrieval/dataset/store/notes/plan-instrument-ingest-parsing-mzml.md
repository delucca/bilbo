---
id: 01K9QY736TM3VD90P81BDR38YA
created: 2025-11-10T19:29-03:00
---

# Plan: mzML parsing in instrument-ingest-worker before 2.6.0

The plan is to add parsing of mzML output files to instrument-ingest-worker before release 2.6.0. Today the worker handles the instrument output formats we already support and has nothing for mass spectrometry files in the mzML format. Scientists who run mass spec instruments currently attach those files by hand to notebook entries, which leaves a gap in the audit trail: the file is there, but nothing records that the entry was built from it automatically. The goal is that an mzML file dropped by an instrument is picked up, parsed, linked to the right notebook entry and audited the same way as every other instrument output.

This note is the plan, not a record of finished work. Nothing here has been built yet. Update it as steps land and remove steps that turn out wrong.

## Goal and deadline

The hard constraint is the release: mzML parsing has to be in instrument-ingest-worker before 2.6.0 ships. If it cannot be finished and reviewed in time, the fallback is to ship the file handling in a reduced form (store the raw file, record metadata only) and leave full spectrum parsing for the next release. That fallback is a decision for the release owner, not for whoever is coding. Raise it early rather than late.

What "done" means for this work:

- An mzML file arriving through the normal ingest path is recognised by format, not only by file extension.
- The parser extracts the run-level metadata that compliance cares about (instrument identity, acquisition time, software that produced the file, sample references) and a summary of the spectra.
- The raw file is kept unchanged in blob storage and the parsed result is stored in SQL Server next to the notebook entry link.
- Every step writes an audit record, including failures.
- Malformed files are rejected cleanly and reported, never half-ingested.

## Where it fits in the worker

instrument-ingest-worker consumes messages from RabbitMQ that announce new instrument output, fetches the file from Azure Blob Storage, runs a format-specific parser, and writes results to SQL Server. The mzML work should follow that shape and not invent a second path. In practice:

- Register a new parser behind the same interface the other format parsers use, and add mzML to whatever table or registry the worker uses to pick a parser.
- Keep format detection in one place. Look at the file content (the root element and namespace) as well as the name, since instruments and operators name files inconsistently.
- Do not change message contracts on the queue unless unavoidable. If the message needs a new field, it must be optional so older publishers keep working during rollout.

Check the existing parsers first and copy their conventions for logging, cancellation, retries and error types. The new code should read like the code around it.

## Parsing approach

mzML is XML and the files can be large. Do not load the whole document into memory. Use a streaming reader and process the file in a forward-only pass. Points to settle while building:

- Read the header sections first (file description, software list, instrument configuration, data processing) and fail fast if they are missing or inconsistent.
- Walk the spectrum and chromatogram lists lazily. For the first version, store summary values per run and not every peak. Full peak storage is a separate decision because of volume.
- Binary data arrays are encoded and may be compressed. Decide whether v1 decodes them at all. Recommendation: do not decode peak arrays in this release; record that they exist and how they are encoded, and keep the raw file for anyone who needs the peaks.
- Handle both plain mzML and the indexed variant, which wraps the same content with an index. The index must not be trusted for correctness; if it disagrees with the content, log it and continue from the content.
- Be strict about encoding and well-formedness. Reject files that are truncated or do not parse, and say why in the audit record.

Prefer a small amount of our own code on top of the framework XML reader over pulling in a large third-party mass spec library, unless a review shows the library is already approved for use in this product. A new dependency in a regulated product needs sign-off, which takes time we may not have before the release.

## Storage and data model

Raw file: write to Azure Blob Storage exactly as received, under the same naming and container rules the worker uses for other outputs. Never rewrite or normalise the raw file. Compliance needs the original bytes, and a content hash should be recorded at the time of receipt so later checks can prove nothing changed.

Parsed result: add tables in SQL Server for the run-level metadata and the spectrum summary, keyed to the existing ingest record so the link to the notebook entry comes for free. The migration must be additive. No existing columns change, so the worker and the rest of the system can be deployed in either order.

Open question: whether run-level metadata belongs in a generic key-value structure or in typed columns. Typed columns are easier to query for compliance reports, key-value is easier to extend. Lean to typed columns for the fields we know we report on and one flexible column for the rest.

## Audit trail requirements

This product exists to enforce audit trails, so this is the part that gets the most scrutiny from reviewers and from compliance officers. For each mzML file the audit log should show, in order: received, format detected, parsing started, parsing finished or failed, stored, linked to entry. Each record carries who or what acted, the time, the file identity, and the content hash.

Failures are audited as failures. A rejected file leaves a record with a reason a person can read, and the raw file is still kept so the evidence is not lost. Retries must not create misleading duplicates: a redelivered message for a file already ingested should be recognised and recorded as a repeat, not parsed and linked twice.

Do not log file contents or sample identifiers in free-text log lines. Put them in the audit store, where access is controlled.

## Testing

- Unit tests for the parser with small hand-made mzML samples: valid, indexed, missing header section, truncated, wrong namespace, empty run.
- At least one realistic sample from a real instrument, anonymised, kept with the other test fixtures. Ask a scientist on the team for one; do not fabricate a sample and call it realistic.
- A memory check with a large file to confirm the streaming reader stays flat. This is the failure most likely to show up only in production.
- Integration test through RabbitMQ and blob storage in the existing test environment: publish a message, confirm the parsed rows, the audit records and the entry link.
- A redelivery test for idempotence.

## Rollout and risks

Ship behind a configuration switch so the mzML path can be turned off in a deployment without a rollback. Roll it out to a pilot lab first and watch the audit records and worker memory before enabling it everywhere.

Risks to watch:

- Large files causing memory pressure or long processing that outlasts the message visibility or acknowledgement window. Make sure acknowledgement happens after durable storage, and that long parses do not cause duplicate delivery.
- Vendor variation. Different instrument vendors write mzML with different optional sections. Expect surprises and make the parser tolerant of optional parts while strict on required ones.
- Validation obligations. If the product's validation documentation has to cover new parsers, the paperwork can take longer than the code. Start that conversation with compliance now.
- Scope creep into peak storage, visualisation or search. All out of scope for this release.

## Next steps

- Read the existing parsers in instrument-ingest-worker and note the interface and conventions to follow.
- Get sample mzML files, including an indexed one, from the scientists.
- Decide with the release owner on the fallback to metadata-only if time runs short.
- Confirm the dependency policy for XML or mass spec libraries.
- Draft the additive SQL migration and have it reviewed by whoever owns the schema.
- Implement detection and the streaming parser, then the storage and audit steps, then tests, then the configuration switch.
- Talk to compliance about validation documentation for the new format.
