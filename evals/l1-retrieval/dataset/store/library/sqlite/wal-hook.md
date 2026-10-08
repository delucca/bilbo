---
id: 01M4D07J7QNMJSG7319W01X8KF
fetched: 2026-10-08
origin: "url: https://www.sqlite.org/c3ref/wal_hook.html"
digest: sha256:0f5f6f16ab16917d985013c8c9a63b0e223a0a19d45f91cfc0be026b19b3bb83
kept: 30-58
---
# Write-Ahead Log Commit Hook

## Write-Ahead Log Commit Hook

> ```
> void *sqlite3_wal_hook(
>   sqlite3*,
>   int(*)(void *,sqlite3*,const char*,int),
>   void*
> );
> ```

The [sqlite3\_wal\_hook()](../c3ref/wal_hook.html) function is used to register a callback that is invoked each time data is committed to a database in wal mode.

The callback is invoked by SQLite after the commit has taken place and the associated write-lock on the database released, so the implementation may read, write or [checkpoint](../wal.html#ckpt) the database as required.

The first parameter passed to the callback function when it is invoked is a copy of the third parameter passed to sqlite3\_wal\_hook() when registering the callback. The second is a copy of the database handle. The third parameter is the name of the database that was written to - either "main" or the name of an [ATTACH](../lang_attach.html)-ed database. The fourth parameter is the number of pages currently in the write-ahead log file, including those that were just committed.

The callback function should normally return [SQLITE\_OK](../rescode.html#ok). If an error code is returned, that error will propagate back up through the SQLite code base to cause the statement that provoked the callback to report an error, though the commit will have still occurred. If the callback returns [SQLITE\_ROW](../rescode.html#row) or [SQLITE\_DONE](../rescode.html#done), or if it returns a value that does not correspond to any valid SQLite error code, the results are undefined.

A single database handle may have at most a single write-ahead log callback registered at one time. Calling [sqlite3\_wal\_hook()](../c3ref/wal_hook.html) replaces the default behavior or previously registered write-ahead log callback.

The return value is a copy of the third parameter from the previous call, if any, or 0.

The [sqlite3\_wal\_autocheckpoint()](../c3ref/wal_autocheckpoint.html) interface and the [wal\_autocheckpoint pragma](../pragma.html#pragma_wal_autocheckpoint) both invoke [sqlite3\_wal\_hook()](../c3ref/wal_hook.html) and will overwrite any prior [sqlite3\_wal\_hook()](../c3ref/wal_hook.html) settings.

If a write-ahead log callback is set using this function then [sqlite3\_wal\_checkpoint\_v2()](../c3ref/wal_checkpoint_v2.html) or [PRAGMA wal\_checkpoint](../pragma.html#pragma_wal_checkpoint) should be invoked periodically to keep the write-ahead log file from growing without bound.

Passing a NULL pointer for the callback disables automatic checkpointing entirely. To re-enable the default behavior, call sqlite3\_wal\_autocheckpoint(db,1000) or use [PRAGMA wal\_checkpoint](../pragma.html#pragma_wal_checkpoint). 

See also lists of [Objects](../c3ref/objlist.html), [Constants](../c3ref/constlist.html), and [Functions](../c3ref/funclist.html).
