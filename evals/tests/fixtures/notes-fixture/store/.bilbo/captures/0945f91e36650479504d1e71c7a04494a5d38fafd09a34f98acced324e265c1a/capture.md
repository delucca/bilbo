# Demo busy timeout

Intro nav line.

## Busy timeout

A writer that meets a lock waits up to the timeout before it reports `SQLITE_BUSY`.

### Setting the timeout

The timeout is zero by default; set it in milliseconds right after opening the connection.
