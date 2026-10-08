---
id: 01M4D07JG2BJE103EC9C4G42Z5
fetched: 2026-10-08
origin: "url: https://www.sqlite.org/lang_reindex.html"
digest: sha256:a7d814d991c02ff2e86a030bc64686ecbbb00f8807f7e0edd7c7c79a18133e71
kept: 28-42
---
# Reindex

REINDEX 

**[reindex-stmt:](syntax/reindex-stmt.html)** hide

The REINDEX command is used to delete and recreate indexes from scratch. This is useful when the definition of a collation sequence has changed, or when there are [indexes on expressions](expridx.html) involving a function whose definition has changed. 

If the REINDEX keyword is not followed by a collation-sequence or database object identifier or the keyword "EXPRESSIONS", then all indexes in all attached databases are rebuilt. 

If the REINDEX keyword is followed by a collation-sequence name, then all indexes in all attached databases that use the named collation sequences are recreated. 

Or, if the argument attached to the REINDEX identifies a specific database table, then all indexes attached to the database table are rebuilt. If it identifies a specific database index, then just that index is recreated. 

If the argument is "EXPRESSIONS", then all [indexes on expressions](expridx.html) are rebuilt. The ability to rebuild expression indexes in a single command like this is new as of SQLite version 3.53.0 (2026-04-09). 

For a command of the form "REINDEX *name*", a match against collation-name takes precedence over a match against index-name or table-name. This ambiguity in the syntax may be avoided by always specifying a schema-name when reindexing a specific table or index. If *name* is EXPRESSIONS, then in addition to rebuilding all expression indexes, the command also rebuilds any indexes that use a collating sequence named "expressions" or the indexes of a table named "expressions" or an index named "expressions", following the precedence rules above. 
