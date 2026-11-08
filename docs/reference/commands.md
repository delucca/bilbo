# Commands

## remember

```text
bilbo remember <file|folder> [--kind <kind>] [--project <project>]
```

Keeps files as entries in the store. An entry is a sequence of records, numbered from 1 per generation. Run it again and it keeps only what is new.

### Kinds

A kind picks the files under the target, splits each into records and proposes the entry's meta. `bilbo remember --help` lists the kinds.

| Kind | Claims | Records |
|---|---|---|
| `markdown` | `.md` files | one per line, newline included; a final line without a newline is one record |

A `markdown` file that is not UTF-8 is refused. A valid ULID in the frontmatter `id` becomes the entry's id at its first capture; a file with no usable `id` gets a fresh one. An id already held by an entry of another origin is refused for that file.

### Detection

Without `--kind`, every kind says which files it claims under the target. Exactly one kind claims files: it runs. None: there is nothing to keep (exit 1). Several: a usage error that names them and asks for `--kind`. With `--kind`, only that kind runs; an unknown kind is a usage error.

### What the walk skips

A folder is walked recursively. Hidden entries (names starting with `.`) and symbolic links that the walk finds are skipped, and the walk never follows a link, so it cannot loop.

A target named on the command line is taken as given, even when it is a symbolic link or a hidden name, as `find -H` and `git add` treat their arguments. Its origin is the resolved path, and a kind tests the resolved file, not the name given: a `note.md` link to a file without `.md` keeps nothing.

### Outcomes

An entry is found again by its origin (`file:<absolute path>`). For each file:

- **new**: no entry has this origin yet.
- **appended**: the earlier bytes are unchanged and the file gained records; only those are kept.
- **new generation**: an earlier record changed, or the file shrank. The older generation stays readable.
- **unchanged**: nothing to keep.
- **refused**: the file is named on stderr with its reason, and the run goes on.

`--project` labels new entries only. A file that left the folder keeps its entry.

### Output

Standard output, two lines:

```text
new <n>, appended <n>, new generation <n>, unchanged <n>, refused <n>
records <n>, raw bytes <n>, stored bytes <n>
```

Raw bytes are the input bytes kept; stored bytes are what the store grew by. On a terminal the same counts come as a marked message.

A second `remember` waits for the first one to finish writing the store, and says so on stderr.

### Exit codes

| Code | Meaning |
|---|---|
| 0 | success |
| 1 | a file was refused, nothing was claimed, or the target or the store could not be read |
| 2 | usage or config error |
