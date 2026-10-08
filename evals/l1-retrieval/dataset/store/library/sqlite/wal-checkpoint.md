---
id: 01M4D07J6NJDMW6WMMQ3PXBT7N
fetched: 2026-10-08
origin: "url: https://www.sqlite.org/c3ref/wal_checkpoint.html"
digest: sha256:c5581f00d713164a2b0362ce7a9ee25001036e8c954123d7b38d47fab7db6b51
kept: 30-42
---
# Checkpoint a database

## Checkpoint a database

> ```
> int sqlite3_wal_checkpoint(sqlite3 *db, const char *zDb);
> ```

The sqlite3\_wal\_checkpoint(D,X) is equivalent to [sqlite3\_wal\_checkpoint\_v2](../c3ref/wal_checkpoint_v2.html)(D,X,[SQLITE\_CHECKPOINT\_PASSIVE](../c3ref/c_checkpoint_full.html),0,0).

In brief, sqlite3\_wal\_checkpoint(D,X) causes the content in the [write-ahead log](../wal.html) for database X on [database connection](../c3ref/sqlite3.html) D to be transferred into the database file and for the write-ahead log to be reset. See the [checkpointing](../wal.html#ckpt) documentation for addition information.

This interface used to be the only way to cause a checkpoint to occur. But then the newer and more powerful [sqlite3\_wal\_checkpoint\_v2()](../c3ref/wal_checkpoint_v2.html) interface was added. This interface is retained for backwards compatibility and as a convenience for applications that need to manually start a callback but which do not need the full power (and corresponding complication) of [sqlite3\_wal\_checkpoint\_v2()](../c3ref/wal_checkpoint_v2.html). 

See also lists of [Objects](../c3ref/objlist.html), [Constants](../c3ref/constlist.html), and [Functions](../c3ref/funclist.html).
