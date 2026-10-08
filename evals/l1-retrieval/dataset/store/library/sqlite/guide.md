---
id: 01M4D07HVVG2SNJCGVSGD6MCAX
created: 2026-10-08T02:36-03:00
---

# sqlite

The SQLite documentation, in the public domain: the SQL dialect, the file and locking model, write-ahead logging, pragmas, extensions and the C interface.

## on-conflict

Describes SQLite's ON CONFLICT clause and its five resolution algorithms (ROLLBACK, ABORT, FAIL, IGNORE, REPLACE), including how each handles UNIQUE, NOT NULL, CHECK and foreign key violations. Consult it when deciding how INSERT or UPDATE statements should react to constraint violations, or when checking side effects of REPLACE on triggers, update hooks and change counts.

## sqlite3-limit

Documents the sqlite3_limit() C interface for querying and setting per-connection run-time limits on the size of SQLite constructs, including how negative values, hard compile-time upper bounds, and the returned prior value behave. Consult it when restricting databases from untrusted sources or when you need to read a current limit without changing it.

## window-functions

Covers SQLite window functions: OVER clauses, PARTITION BY, frame specifications, FILTER, built-in window functions, window chaining, and writing user-defined aggregate window functions in C. Consult it when writing or debugging window queries in SQLite or implementing xStep/xInverse/xValue/xFinal callbacks.

## sqlite3-open

Documents the C interfaces sqlite3_open(), sqlite3_open16() and sqlite3_open_v2(), including flag values, VFS selection, error handling, and URI filename syntax with its query parameters and examples. Consult it when opening SQLite database connections, choosing open flags such as read-only or shared cache, or constructing file: URI filenames.

## uri-filenames

Describes SQLite URI filenames: the URI format, path and query string rules, backwards compatibility, and the recognized query parameters such as mode, cache, vfs, nolock, psow, and immutable. Consult it when building a database connection string or deciding how to open a database with URI options.

## error-codes-messages

Documents the SQLite C interfaces sqlite3_errcode, sqlite3_extended_errcode, sqlite3_errmsg, sqlite3_errmsg16, sqlite3_errstr and sqlite3_error_offset. Consult it when retrieving error codes and messages after a failed call, including which functions preserve the error code, memory ownership of message strings, and thread-safety with serialized mode.

## sqlite-file-format

Describes the SQLite database file format: the header, pages, freelist, b-tree pages, record format, schema table storage, rollback journal, and write-ahead log format. Consult it when parsing or validating SQLite files at the byte level or when you need details of on-disk structures, journaling, or WAL checksums.

## sqlite3-step

Documents the sqlite3_step() C API, which evaluates a prepared SQL statement, including the meaning of the SQLITE_BUSY, SQLITE_DONE, SQLITE_ROW, SQLITE_ERROR and SQLITE_MISUSE return values. Consult it when writing or debugging statement-execution loops, handling errors and retries, or dealing with the differences between the legacy and v2/v3 prepare interfaces and the automatic-reset behavior.

## sqlite3-db-config

Documents the sqlite3_db_config() C interface, which changes configuration settings on a single database connection using an integer configuration verb followed by verb-specific arguments. Consult it when you need the function's signature, how it differs from sqlite3_config(), or its return value behavior.

## about-sqlite

Overview of SQLite as an embedded, serverless, self-contained SQL database engine, covering its single-file cross-platform format, small library size, testing and reliability practices, public-domain licensing, and long-term support intent through 2050. Consult it for a high-level description of what SQLite is and its design goals, not for API or SQL syntax details.

## replace

Describes the SQLite REPLACE command as an alias for INSERT OR REPLACE, provided for compatibility with other SQL database engines. Consult it when you see or need to write REPLACE statements in SQLite and want to know how they map to INSERT conflict handling.

## vacuum

Covers the SQLite VACUUM statement: its syntax, the INTO clause for writing a vacuumed copy to a new file, how VACUUM works internally, and its limitations. Consult it when rebuilding or compacting a database, making a consistent snapshot copy with VACUUM INTO, or weighing VACUUM against auto_vacuum.

## sqlite-with-clause

Documents SQLite's WITH clause, covering ordinary and recursive common table expressions, with examples for hierarchical queries, graph traversal, depth-first versus breadth-first ordering, and the MATERIALIZED and NOT MATERIALIZED hints. Consult it when writing or debugging CTEs in SQLite, or when checking version requirements and limitations such as the restriction on use in triggers.

## loadable-extensions

Explains SQLite run-time loadable extensions: loading them with load_extension() or the shell .load command, compiling them as shared libraries, writing entry points with the sqlite3ext.h macros, persistent extensions, and static linking with SQLITE_CORE. Consult it when building, loading, or debugging an extension, or choosing entry point names and load-time settings.

## threadsafe

Explains SQLite's three threading modes (single-thread, multi-thread, serialized) and how each is selected at compile time with SQLITE_THREADSAFE, at start time with sqlite3_config(), and per connection with sqlite3_open_v2() flags. Consult it when using SQLite from multiple threads, choosing mutex behavior, or interpreting sqlite3_threadsafe().

## strict-tables

Explains SQLite STRICT tables: the CREATE TABLE STRICT option, the allowed column datatypes and rigid type enforcement, the ANY datatype, and other table options such as WITHOUT ROWID. Consult it when defining tables that need enforced column types, or when checking compatibility of STRICT tables with SQLite versions before 3.37.0.

## sqlite-json

Documents SQLite's built-in JSON functions and operators, including json_extract, the -> and ->> operators, json_set/insert/replace, json_patch, aggregate functions, json_each and json_tree, plus the JSONB binary format, path and value argument rules, JSON5 extensions, and known quirks. Consult it when writing or debugging SQL that creates, queries, modifies, or validates JSON text or JSONB in SQLite.

## partial-indexes

Covers SQLite partial indexes: creating them with a WHERE clause on CREATE INDEX, unique partial indexes, the rules the query planner uses to decide when a partial index can serve a query, and which SQLite versions support them. Consult it when designing indexes over a subset of rows or when working out why a query does or doesn't use a partial index.

## busy-handler

Documents sqlite3_busy_handler(), which registers a callback that SQLite invokes when a database table is locked by another thread or process. Consult it for the callback's arguments and return value, deadlock cases where the handler is skipped, how it interacts with sqlite3_busy_timeout(), and restrictions on what the handler may do.

## update-hook

Documents the SQLite C interface sqlite3_update_hook(), which registers a callback invoked when rows in rowid tables are inserted, updated or deleted. Consult it for the callback arguments, the cases where the hook is not invoked (WITHOUT ROWID tables, truncate optimization, ON CONFLICT REPLACE), and the restriction against modifying the connection inside the hook.

## file-locking

Describes how SQLite version 3 file locking and concurrency work in rollback-journal mode, covering lock states, the rollback journal, the steps for writing to a database file, and ways databases can be corrupted. Consult it when you need the exact locking and commit behavior, or how BEGIN, COMMIT and ROLLBACK relate to locks; WAL mode is not covered.

## sqlite-limits

Describes the size and quantity limits in SQLite, such as maximum BLOB or string length, number of columns, SQL statement length, page count, database size, and number of tables. Consult it when checking whether a design will hit a hard limit or how to raise or lower one with sqlite3_limit() or compile-time options.

## wal-checkpoint

Documents the sqlite3_wal_checkpoint() C function, which is equivalent to sqlite3_wal_checkpoint_v2() with the PASSIVE mode and transfers write-ahead log content into the database file. Consult it when you need to trigger a basic manual WAL checkpoint or decide whether to use the _v2 interface instead.

## sqlite-datatypes

Describes SQLite's dynamic type system: storage classes, type affinity rules for columns and expressions, comparison and type conversion behavior, sort order, and collating sequences. Consult it when determining how SQLite stores, converts, compares, sorts or groups values of different types.

## wal-hook

Documents the SQLite C function sqlite3_wal_hook(), which registers a callback invoked after each commit to a database in WAL mode. Consult it for the callback's parameters and return-value rules, how it interacts with sqlite3_wal_autocheckpoint and the wal_autocheckpoint pragma, and how to disable or restore automatic checkpointing.

## sqlite3-changes

Documents the sqlite3_changes() C interface, which returns the number of rows modified, inserted or deleted by the most recently completed INSERT, UPDATE or DELETE statement on a connection. Consult it for what is and isn't counted (views, INSTEAD OF triggers, trigger programs, foreign key actions) and for thread-safety caveats.

## compile-options

Documents SQLite's compile-time options (SQLITE_* preprocessor macros), including recommended options, platform configuration, default parameter values, size limits, and options that enable, disable, or omit features. Consult it when building SQLite from source and deciding which macros to define, or when checking what a specific SQLITE_ macro does.

## create-function

Documents the SQLite C API functions sqlite3_create_function, sqlite3_create_function_v2 and sqlite3_create_window_function for registering application-defined scalar, aggregate and window SQL functions. Consult it when implementing or overriding custom SQL functions, choosing text encoding and flags such as SQLITE_DETERMINISTIC and SQLITE_DIRECTONLY, or handling callbacks and destructors.

## isolation

Explains SQLite's isolation semantics: transactions are SERIALIZABLE, changes are invisible to other connections until commit, and behavior is undefined when a query's table is modified on the same connection mid-query. Consult it when reasoning about visibility of changes between connections, concurrency, shared-cache read_uncommitted, or deleting and updating rows while stepping through a SELECT.

## sqlite-blob-open

Documents the sqlite3_blob_open() C interface for opening a BLOB or TEXT value for incremental I/O, including its parameters, flags, error conditions, and handle expiration behavior. Consult it when reading or writing large values in pieces with sqlite3_blob_read() and sqlite3_blob_write(), or when debugging why a blob open fails or a handle returns SQLITE_ABORT.

## backup-api

Documents the SQLite Online Backup API: sqlite3_backup_init, step, finish, remaining and pagecount, including error handling, concurrency and threading rules for source and destination connections. Consult it when copying or backing up a live SQLite database from C, or when choosing between this API, VACUUM INTO and sqlite3_rsync.

## sqlite-expressions

Covers SQLite's SQL expression syntax: operators and precedence, literals, bound parameters, LIKE/GLOB/REGEXP/MATCH, BETWEEN, CASE, IN, EXISTS, subqueries, CAST, boolean expressions, and function invocation forms. Consult it when writing or debugging SQLite expressions or checking operator behavior and type-conversion rules.

## generated-columns

Covers SQLite generated columns: the GENERATED ALWAYS AS syntax, VIRTUAL versus STORED columns, their capabilities and limitations, and version compatibility (added in 3.31.0). Consult it when defining or altering tables with computed columns, or when checking restrictions such as ALTER TABLE ADD COLUMN and PRAGMA table_xinfo behavior.

## explain-query-plan

Describes SQLite's EXPLAIN QUERY PLAN command and how to read its output, including table and index scans, temporary b-trees for sorting, subqueries, and compound queries. Consult it when interpreting a query plan or checking whether a query uses an index, covering index, or temporary sort.

## sqlite-cli

Documents the sqlite3 command-line shell: starting it, dot-commands, output formats, schema queries, importing and exporting CSV, redirecting I/O, SQLite Archive, database recovery, SQL parameters, command-line options, and building the shell from source. Consult it when you need the syntax or behavior of a specific dot-command or shell option, or when scripting sqlite3.

## virtual-tables

Describes the SQLite virtual table mechanism: how to create and register virtual table modules, and the full sqlite3_module method interface (xCreate, xConnect, xBestIndex, xFilter, xColumn, xUpdate, transaction methods, xShadowName, xIntegrity, and others). Consult it when implementing or debugging a custom virtual table or when you need the exact semantics of a module callback.

## attach-database

Covers the SQLite ATTACH DATABASE statement: its syntax, how the filename and schema name are interpreted, VFS selection, table name resolution across attached databases, atomicity of multi-database transactions, and the limit on attached databases. Consult it when attaching additional database files to a connection or when reasoning about cross-database queries and commits.

## sqlite3-config

Documents sqlite3_config(), the C interface for making global SQLite configuration changes, including its non-threadsafe nature, when it may be called relative to sqlite3_initialize() and sqlite3_shutdown(), and its return values. Consult it when setting global configuration options or debugging SQLITE_MISUSE returned from a sqlite3_config() call.

## result-codes

Documents SQLite's primary and extended result codes, including their numeric values, definitions, and the meaning of each code. Consult it when interpreting an error returned by the SQLite C API or deciding how to handle a specific code such as SQLITE_BUSY or SQLITE_IOERR_*.

## sqlite3-exec

Documents the sqlite3_exec() C interface, a convenience wrapper that runs one or more semicolon-separated SQL statements, including its callback arguments, error message handling, and return behavior. Consult it when writing or reviewing C code that calls sqlite3_exec(), especially for freeing error strings with sqlite3_free(), callback abort semantics, and the usage restrictions.

## reindex

Describes the SQLite REINDEX statement, which deletes and recreates indexes, including its forms for collation names, tables, indexes, and the EXPRESSIONS keyword added in 3.53.0. Consult it when rebuilding indexes after a collation or expression-function definition changes, or to resolve name precedence ambiguity in REINDEX arguments.

## rtree

Documents the SQLite R*Tree module: creating, populating, and querying rtree virtual tables, auxiliary columns, integer-valued R-trees, custom query callbacks, shadow tables, and the rtreecheck() integrity function. Consult it when indexing multi-dimensional range data in SQLite or implementing custom R-Tree geometry queries.

## aggregate-functions

Covers SQLite's built-in aggregate functions, including their syntax, the list of functions, and descriptions of each such as sum, total, count, avg, min, max, group_concat and their handling of NULLs, DISTINCT, ORDER BY and overflow. Consult it when writing or debugging aggregate queries in SQLite or checking the exact semantics of an aggregate function.

## sqlite-delete

Covers the SQLite DELETE statement: syntax, WHERE and RETURNING behavior, restrictions inside CREATE TRIGGER, the optional LIMIT, OFFSET and ORDER BY clauses, and the truncate optimization. Consult it when writing or debugging DELETE statements, especially limited deletes or deletes inside triggers.

## query-optimizer

Overview of the SQLite query optimizer and the transformations it applies, including WHERE clause analysis, index use, BETWEEN, OR, LIKE and skip-scan optimizations, join ordering, subquery flattening, co-routines, automatic indexes, and push-down. Consult it when working out why SQLite picks a particular query plan or how to write queries and indexes so the optimizer can use them.

## sqlite-autoincrement

Explains how SQLite assigns ROWIDs by default and how the AUTOINCREMENT keyword changes that, including the sqlite_sequence table, the guarantees it gives, and its restrictions. Consult it when deciding whether to use INTEGER PRIMARY KEY AUTOINCREMENT or when debugging ROWID reuse, gaps, or SQLITE_FULL errors.

## speed-comparison

A historical benchmark comparing SQLite 2.7.6 with MySQL and PostgreSQL across sixteen tests, including inserts, selects, transactions, indexes and DROP TABLE, with timings and explanations of the differences. The page itself says it is very old, so consult it only for historical context or the reasoning behind those results, not for current SQLite performance.

## sqlite-wal

Documents SQLite's Write-Ahead Logging mode: how the WAL, checkpointing, and concurrency work, how to enable and configure it, and its performance tradeoffs. Consult it when setting up WAL, handling large WAL files, read-only databases, shared-memory limits, SQLITE_BUSY in WAL mode, backwards compatibility, or the WAL-reset bug.

## sqlite-testing

Describes how SQLite is tested, including its test harnesses, anomaly and fuzz testing, regression tests, 100% branch and MC/DC coverage, dynamic and static analysis, and release checklists. Consult it when you need to know SQLite's testing methodology, what guarantees its test coverage gives, or how to reason about its reliability.

## sqlite-select

Covers the syntax and processing rules of SQLite's SELECT statement, including FROM and JOIN handling, WHERE, GROUP BY and aggregates, DISTINCT, compound selects, ORDER BY, LIMIT, VALUES, and WITH. Consult it when writing or debugging SELECT queries in SQLite or checking how SQLite deviates from standard SQL in join syntax and precedence.

## sqlite-upsert

Covers SQLite's UPSERT syntax (INSERT ... ON CONFLICT DO NOTHING / DO UPDATE), including conflict targets, the excluded table, examples, parsing ambiguity with SELECT, limitations, and version history. Consult it when writing or debugging insert-or-update statements in SQLite or checking which SQLite version supports a given UPSERT feature.

## sqlite-update

Covers the SQLite UPDATE statement: syntax, the SET clause and conflict handling, restrictions inside CREATE TRIGGER, the UPDATE FROM extension, and the optional ORDER BY and LIMIT clauses available with a compile-time option. Consult it when writing or debugging UPDATE queries in SQLite, especially joins with UPDATE FROM or limiting the rows updated.

## prepared-statement

Describes the sqlite3_stmt prepared statement object in the SQLite C interface and lists the functions that operate on it, such as sqlite3_step, sqlite3_reset, sqlite3_bind_*, and sqlite3_column_*. Consult it to find which API routines work with a compiled SQL statement handle.

## date-time-functions

Documents SQLite's date and time functions (date, time, datetime, julianday, unixepoch, strftime, timediff), including accepted time-value formats, modifiers, and worked examples. Consult it when writing or debugging SQLite date arithmetic, formatting, timezone/localtime conversion, or when checking supported date ranges and known caveats.

## savepoint

Documents SQLite's SAVEPOINT, RELEASE, and ROLLBACK TO statements, including their syntax, how named savepoints form a transaction stack, and how they interact with BEGIN, COMMIT, and ROLLBACK. Consult it when writing nested transactions or partial rollbacks in SQLite, or when you need the exact rules for what RELEASE and ROLLBACK TO do.

## sqlite3-prepare

Reference for the sqlite3_prepare() family (prepare, prepare_v2, prepare_v3 and the UTF-16 variants) that compiles SQL text into a prepared statement. Consult it for the parameters (nByte, pzTail, ppStmt), return values, statement ownership and finalization, and how the v2/v3 interfaces differ from the legacy ones.

## sqlite3-trace-v2

Documents the sqlite3_trace_v2() C interface, which registers a per-connection trace callback with an event mask and context pointer, including how to disable tracing and how the callback is invoked. Consult it when adding SQL tracing or profiling hooks to a SQLite connection or when migrating off the deprecated sqlite3_trace() and sqlite3_profile().

## sqlite-pragma

Reference for SQLite's PRAGMA statement: its syntax, pragma functions, and a list of every pragma with its arguments, behavior, and return values. Consult it when you need to configure or query SQLite internals such as journal_mode, foreign_keys, synchronous, table_info, or wal_checkpoint.

## without-rowid

Explains SQLite WITHOUT ROWID tables: syntax, compatibility, quirks, differences from ordinary rowid tables, and the space and speed benefits of clustered primary keys. Consult it when deciding whether to declare a table WITHOUT ROWID or when checking whether an existing table is one.

## last-insert-rowid

Documents the sqlite3_last_insert_rowid() C interface, including what rowid it returns, how triggers, virtual tables, failed or rolled-back INSERTs and INSERT OR REPLACE affect it, and its thread-safety caveat. Consult it when relying on the last inserted rowid from the C API or the equivalent SQL function.
