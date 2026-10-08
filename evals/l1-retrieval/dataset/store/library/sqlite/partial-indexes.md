---
id: 01M4D07J49BGYBQ9Q2YVZZTYJA
fetched: 2026-10-08
origin: "url: https://www.sqlite.org/partialindex.html"
digest: sha256:90a0db8255738c90f7027eca831a99c85136958bccc55f4dbd5c0d3efc25aa3c
kept: 43-200
---
# 2\. Creating Partial Indexes


A partial index is an index over a subset of the rows of a table. 

In ordinary indexes, there is exactly one entry in the index for every row in the table. In partial indexes, only some subset of the rows in the table have corresponding index entries. For example, a partial index might omit entries for which the column being indexed is NULL. When used judiciously, partial indexes can result in smaller database files and improvements in both query and write performance. 

## 2\. Creating Partial Indexes

Create a partial index by adding a WHERE clause to the end of an ordinary [CREATE INDEX](lang_createindex.html) statement. 

**[create-index-stmt:](syntax/create-index-stmt.html)** hide

**[expr:](syntax/expr.html)** show

**[filter-clause:](syntax/filter-clause.html)** show

**[function-arguments:](syntax/function-arguments.html)** show

**[ordering-term:](syntax/ordering-term.html)** show

**[literal-value:](syntax/literal-value.html)** show

**[over-clause:](syntax/over-clause.html)** show

**[frame-spec:](syntax/frame-spec.html)** show

**[ordering-term:](syntax/ordering-term.html)** show

**[raise-function:](syntax/raise-function.html)** show

**[select-stmt:](syntax/select-stmt.html)** show

**[common-table-expression:](syntax/common-table-expression.html)** show

**[compound-operator:](syntax/compound-operator.html)** show

**[join-clause:](syntax/join-clause.html)** show

**[join-constraint:](syntax/join-constraint.html)** show

**[join-operator:](syntax/join-operator.html)** show

**[ordering-term:](syntax/ordering-term.html)** show

**[result-column:](syntax/result-column.html)** show

**[table-or-subquery:](syntax/table-or-subquery.html)** show

**[window-defn:](syntax/window-defn.html)** show

**[frame-spec:](syntax/frame-spec.html)** show

**[type-name:](syntax/type-name.html)** show

**[signed-number:](syntax/signed-number.html)** show

**[indexed-column:](syntax/indexed-column.html)** show

Any index that includes the WHERE clause at the end is considered to be a partial index. Indexes that omit the WHERE clause (or indexes that are created by UNIQUE or PRIMARY KEY constraints inside of CREATE TABLE statements) are ordinary full indexes. 

The expression following the WHERE clause may contain operators, literal values, and names of columns in the table being indexed. The WHERE clause may *not* contain subqueries, references to other tables, [non-deterministic functions](deterministic.html), or [bound parameters](lang_expr.html#varparam).

Only rows of the table for which the WHERE clause evaluates to true are included in the index. If the WHERE clause expression evaluates to NULL or to false for some rows of the table, then those rows are omitted from the index. 

The columns referenced in the WHERE clause of a partial index can be any of the columns in the table, not just columns that happen to be indexed. However, it is very common for the WHERE clause expression of a partial index to be a simple expression on the column being indexed. The following is a typical example:

```
CREATE INDEX po_parent ON purchaseorder(parent_po) WHERE parent_po IS NOT NULL;
```

In the example above, if most purchase orders do not have a "parent" purchase order, then most parent\_po values will be NULL. That means only a small subset of the rows in the purchaseorder table will be indexed. Hence the index will take up much less space. And changes to the original purchaseorder table will run faster since the po\_parent index only needs to be updated for those exceptional rows where parent\_po is not NULL. But the index is still useful for querying. In particular, if one wants to know all "children" of a particular purchase order "?1", the query would be: 

```
SELECT po_num FROM purchaseorder WHERE parent_po=?1;
```

The query above will use the po\_parent index to help find the answer, since the po\_parent index contains entries for all rows of interest. Note that since po\_parent is smaller than a full index, the query will likely run faster too.

### 2.1. Unique Partial Indexes

A partial index definition may include the UNIQUE keyword. If it does, then SQLite requires every entry *in the index* to be unique. This provides a mechanism for enforcing uniqueness across some subset of the rows in a table.

For example, suppose you have a database of the members of a large organization where each person is assigned to a particular "team". Each team has a "leader" who is also a member of that team. The table might look something like this:

```
CREATE TABLE person(
  person_id       INTEGER PRIMARY KEY,
  team_id         INTEGER REFERENCES team,
  is_team_leader  BOOLEAN,
  -- other fields elided
);
```

The team\_id field cannot be unique because there are usually multiple people on the same team. One cannot make the combination of team\_id and is\_team\_leader unique since there are usually multiple non-leaders on each team. The solution to enforcing one leader per team is to create a unique index on team\_id but restricted to those entries for which is\_team\_leader is true:

```
CREATE UNIQUE INDEX team_leader ON person(team_id) WHERE is_team_leader;
```

Coincidentally, that same index is useful for locating the team leader of a particular team:

```
SELECT person_id FROM person WHERE is_team_leader AND team_id=?1;
```

## 3\. Queries Using Partial Indexes

Let X be the expression in the WHERE clause of a partial index, and let W be the WHERE clause of a query that uses the table that is indexed. Then, the query is permitted to use the partial index if W⇒X, where the ⇒ operator (usually pronounced "implies") is the logic operator equivalent to "X or not W". Hence, determining whether or not a partial index is usable in a particular query reduces to proving a theorem in first-order logic.

SQLite does not have a sophisticated theorem prover with which to determine W⇒X. Instead, SQLite uses two simple rules to find the common cases where W⇒X is true, and it assumes all the other cases are false. The rules used by SQLite are these: 

1. If W is AND-connected terms and X is OR-connected terms and if any term of W appears as a term of X, then the partial index is usable.

   For example, let the index be

   ```
   CREATE INDEX ex1 ON tab1(a,b) WHERE a=5 OR b=6;
   ```

   And let the query be:

   ```
   SELECT * FROM tab1 WHERE b=6 AND a=7; -- uses partial index
   ```

   Then the index is usable by the query because the "b=6" term appears in both the index definition and in the query. Remember: terms in the index should be OR-connected and terms in the query should be AND-connected.

   The terms in W and X must match exactly. SQLite does not do algebra to try to get them to look the same. The term "b=6" does not match "b=3+3" or "b-6=0" or "b BETWEEN 6 AND 6". "b=6" will match to "6=b" as long as "b=6" is on the index and "6=b" is in the query. If a term of the form "6=b" appears in the index, it will never match anything.

2. If a term in X is of the form "z IS NOT NULL" and if a term in W is a comparison operator on "z" other than "IS", then those terms match.

   Example: Let the index be

   ```
   CREATE INDEX ex2 ON tab2(b,c) WHERE c IS NOT NULL;
   ```

   Then any query that uses operators =, <, >, <=, >=, <>, IN, LIKE, or GLOB on column "c" would be usable with the partial index because those comparison operators are only true if "c" is not NULL. So the following query could use the partial index:

   ```
   SELECT * FROM tab2 WHERE b=456 AND c<>0;  -- uses partial index
   ```

   But the next query can not use the partial index:

   ```
   SELECT * FROM tab2 WHERE b=456;  -- cannot use partial index
   ```

   The latter query can not use the partial index because there might be rows in the table with b=456 and where c is NULL. But those rows would not be in the partial index.

These two rules describe how the query planner for SQLite works as of this writing (2013-08-01). And the rules above will always be honored. However, future versions of SQLite might incorporate a better theorem prover that can find other cases where W⇒X is true and thus may find more instances where partial indexes are useful.

## 4\. Supported Versions

Partial indexes have been supported in SQLite since [version 3.8.0](releaselog/3_8_0.html) (2013-08-26). 

Database files that contain partial indices are not readable or writable by versions of SQLite prior to 3.8.0. However, a database file created by SQLite 3.8.0 is still readable and writable by prior versions as long as its schema contains no partial indexes. A database that is unreadable by legacy versions of SQLite can be made readable simply by running [DROP INDEX](lang_dropindex.html) on the partial indexes.

