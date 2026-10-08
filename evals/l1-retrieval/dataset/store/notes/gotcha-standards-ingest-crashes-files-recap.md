---
id: 01KMD4ZBDBTWFK4DH94K28V8VP
created: 2026-03-23T07:48-03:00
---

# standards-ingest crashes on some files

Notes from a second look at why standards-ingest falls over on certain curriculum files. Written quickly, not exhaustive. Some of this probably overlaps with what is already known about the component, but I did not go back to check.

## What happens

The standards-ingest job picks up a curriculum standards file, parses it, and loads the standards into the database and the search index. On some files it dies partway through. The Celery task is marked failed, nothing useful shows up in the teacher-facing UI, and the standards list for that subject stays at whatever it was before. Sometimes it is half updated, which is worse than a clean failure.

## Files that trigger it

The pattern I keep seeing is that the files come from exam boards or ministries and were exported by hand from a word processor or spreadsheet. The crashes cluster around:

- files with odd character encodings, usually from a spreadsheet export that did not say what it used
- files with a byte order mark at the start that the parser treats as part of the first column name
- very large files, where the worker runs out of memory before it finishes
- files with merged cells or blank spacer rows, so a row has fewer fields than the header
- standards codes that repeat within one file, which trips a uniqueness constraint late in the run

None of these are rare. A new school year usually brings a fresh batch of files and a fresh round of failures.

## Encoding and header trouble

The parser assumes one encoding and does not sniff. If the file is in something else, accented characters in subject names come out wrong or the decode raises. The header row is matched by exact text, so a trailing space or a changed capitalisation makes the column go missing, and then every row fails on a lookup. It looks like a data problem but it is really a header mismatch.

## Size and memory

The whole file gets read into memory and then turned into a list of objects before anything is saved. For the biggest national frameworks that is more than the worker is allowed to use, and the process gets killed by the OS rather than raising a Python error. That is why the log just stops with no traceback. Check the worker's memory limit and the system log before assuming a parser bug.

## Duplicates and partial writes

Duplicate standards codes inside one file are only caught when the database rejects the insert, which happens deep in the run. Because the load is not wrapped as one unit, rows before the failure stay in. Re-running the ingest then collides with its own earlier rows. Cleaning up by hand means finding which batch was partially loaded.

## Search index side

After the database step, the job pushes documents to Elasticsearch. If the database step fails, the index is not touched, so the two can differ. If the index step fails, the database already has the new rows and the search page shows old data. The exercise suggester relies on the index for matching, so teachers see suggestions that lag behind the real standards.

## How to reproduce

Take a copy of a failing file, put it somewhere outside the repo, and run the ingest against a development database, not a shared one. Trim the file by halves until the crash stops; that finds the offending row quickly. Compare the header row byte for byte with a file that works. Run it with the worker memory limit lowered to see whether it is the size problem.

## Things to do about it

- validate the file before any writes: encoding, header names, field counts per row, duplicate codes
- strip the byte order mark and normalise header text
- stream the file rather than reading it whole, and save in batches inside a transaction
- make the ingest safe to re-run, so a repeat does not collide with earlier rows
- report the failing row and reason back to whoever uploaded the file, in plain words
- keep the index and database in step, or at least mark the index as stale when they are not

## Open questions

I am not sure whether the configured memory limit is the same in every environment. I also do not know if any of the exam board files come with an official schema we could check against. Worth asking whoever supplies them.
