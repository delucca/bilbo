---
id: 01KEXY5GN4F1FECKTPWBN4H068
created: 2026-01-14T06:42-03:00
---

# standards-ingest fails on CSV files with a byte order mark

standards-ingest crashes on CSV files that were exported with a byte order mark (BOM). The error is `UnicodeDecodeError: 'utf-8' codec can't decode byte 0xff in position 0`. Opening the file with encoding `utf-8-sig` avoids it. The short name `stdimp` means the same thing as standards-ingest; you will see it in logs, task names and chat, so treat the two as one component.

## Symptom

A teacher or admin uploads a standards CSV, the ingest job starts, and it dies on the very first read of the file. Nothing is written to the index and no partial standards show up. The traceback ends with:

```
UnicodeDecodeError: 'utf-8' codec can't decode byte 0xff in position 0
```

Position 0 is the giveaway. The failure is at the first byte of the file, not somewhere in the middle of a row, so it is not a bad character in the data. It is the BOM marker at the start of the file.

## Cause

Some spreadsheet tools write a BOM at the start of a CSV when you pick a "CSV UTF-8" or similar export option. Plain `utf-8` decoding does not strip it. In the case that crashed, the first byte was 0xff, which the plain `utf-8` codec cannot decode, so standards-ingest stopped before it parsed a single header or row.

Files that look fine in a text editor can still have the BOM, because editors hide it. Do not trust "it opens fine" as proof that a file is clean.

## Fix

Open the CSV with encoding `utf-8-sig` instead of `utf-8`. That codec reads files with a BOM and files without one, so it is safe as the default for every upload. It also stops the marker leaking into the first column name, which would otherwise break header matching on the first field.

```python
with open(path, newline="", encoding="utf-8-sig") as f:
    reader = csv.DictReader(f)
```

Do this wherever standards-ingest opens an uploaded file, not only in the one code path that failed. If there is more than one reader (for example one for the Celery task and one for a management command), change them all.

## Notes for later

- Do not ask users to re-export their files. The ingest should accept what spreadsheet tools produce.
- When adding a new import path to standards-ingest, start from `utf-8-sig` and only go back to `utf-8` with a reason.
- If a similar decode error shows up with a different byte or a different position, it is probably a different encoding problem (for example a file saved in another code page), not this BOM issue. Check the position first: position 0 points to the BOM.
- Keep a small test fixture with a BOM so this does not regress.
