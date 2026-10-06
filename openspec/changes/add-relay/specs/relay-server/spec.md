# Spec Delta

## Purpose
`bilbo relay`, the server a user runs on a machine they control so their devices can sync through it: how it starts, where and how durably it keeps data, which owners it admits, its limits, what it logs, and the NixOS module that runs it as a service.

## ADDED Requirements

### Requirement: Start the relay
`bilbo relay --data <dir> --owner <fingerprint> [--owner <fingerprint>]... [--listen <address:port>] [--max-scopes <n>] [--max-scope-mb <n>] [--max-object-mb <n>]` SHALL serve the `relay-api` spec until it is stopped. Once listening, it SHALL print `relay listening on http://<address:port>` to stderr, with the port actually bound. `--owner` SHALL be read without regard to case and with or without its hyphens. A missing `--data` or `--owner`, a fingerprint that is not 24 base32 characters once hyphens are dropped, an invalid value, or any other argument SHALL be a usage error.

#### Scenario: A relay starts
- **WHEN** a user runs `bilbo relay --data /srv/relay --owner <their fingerprint>`
- **THEN** stderr says `bilbo: relay listening on http://127.0.0.1:8738` and the relay answers `GET /v1/`

#### Scenario: A free port for tests
- **WHEN** a test runs `bilbo relay --data <dir> --owner <fingerprint> --listen 127.0.0.1:0`
- **THEN** the startup line names the port the system picked

#### Scenario: No owner
- **WHEN** a user runs `bilbo relay --data /srv/relay`
- **THEN** stderr names `--owner` as missing with the usage message, the exit code is 2, and nothing is created

#### Scenario: A mistyped fingerprint
- **WHEN** a user passes `--owner` a value that is not an owner fingerprint
- **THEN** stderr names the value as not an owner fingerprint, and the exit code is 2

### Requirement: The listen address
The relay SHALL speak plain HTTP/1.1 and SHALL NOT terminate TLS. It SHALL listen on `127.0.0.1:8738` unless `--listen` says otherwise. When the address is not a loopback address, it SHALL print `relay serves plain HTTP on <address:port>; put a TLS proxy in front of it` to stderr and serve anyway. An address it cannot bind SHALL make it print `cannot listen on <address:port>: <reason>` and exit 1.

#### Scenario: Behind a proxy on another host
- **WHEN** a user runs the relay with `--listen 0.0.0.0:8738`
- **THEN** stderr carries the plain HTTP warning and the relay serves

#### Scenario: A port in use
- **WHEN** another process holds `127.0.0.1:8738`
- **THEN** stderr says `cannot listen on 127.0.0.1:8738`, the exit code is 1, and the data folder is not locked afterwards

### Requirement: The data folder
The relay SHALL create `--data` with mode 0700 when it is missing, and SHALL keep objects under it at the same paths as the transport tree, so the folder is also a valid `file://` transport. It SHALL hold `<data>/.relay.lock` for its lifetime. A second relay on the same folder SHALL print `another relay serves <data>` and exit 1. At start, it SHALL delete what an interrupted write left under `<data>/.tmp/`.

#### Scenario: The tree on disk
- **WHEN** a device creates segment 1 of a scope through the relay
- **THEN** `<data>/scopes/<scope_id>/devices/<device_id>/00000000000000000001.seg` holds the segment's bytes

#### Scenario: Two relays on one folder
- **WHEN** a relay serves `/srv/relay` and a user starts another with `--data /srv/relay`
- **THEN** the second prints `another relay serves /srv/relay` and exits 1, and the first keeps serving

#### Scenario: Data that is not a folder
- **WHEN** `--data` names a regular file
- **THEN** stderr names the path, the exit code is 1, and the file is unchanged

### Requirement: Durable writes
The relay SHALL answer 201 or 200 to a create only once the object's bytes and its name are on stable storage. An object SHALL never be visible under its final name with partial bytes, whether the relay is killed, the machine loses power or the disk fills during the write.

#### Scenario: Killed mid-write
- **WHEN** the relay is killed while receiving a segment's body, and started again
- **THEN** the segment does not exist, `.tmp` is empty, and the device's retry is answered 201

#### Scenario: Killed after the answer
- **WHEN** the relay answered 201 to a create and is then killed with SIGKILL
- **THEN** after a restart the object is served with the same bytes

#### Scenario: A full disk
- **WHEN** the disk fills while the relay writes or flushes an object
- **THEN** the relay answers 507 `quota`, the object does not exist, nothing is left under `.tmp/`, and the relay keeps serving reads

#### Scenario: Another write error
- **WHEN** any step of a create fails for a reason other than a full disk
- **THEN** the relay answers 500 `internal`, the object does not exist, and nothing is left under `.tmp/`

### Requirement: Admitted and valid scopes
At start the relay SHALL verify every scope on disk: each manifest's signature, `n`, `prev`, unchanged owner, device ids, and chain entries for every earlier epoch kept unchanged from the version before, contiguous segment seqs, and an owner passed with `--owner`. It SHALL keep, but not serve, a scope that fails: requests on it SHALL answer 403 `not-admitted` when its owner is not passed and 403 `invalid` otherwise, it SHALL be logged once, and it SHALL NOT count toward `--max-scopes`.

#### Scenario: Two owners on one relay
- **WHEN** the relay was started with two `--owner` flags
- **THEN** it admits scopes of both owners, and each owner's `GET /v1/scopes/` lists only its own

#### Scenario: An owner dropped from the flags
- **WHEN** the relay restarts without the `--owner` flag of a scope it holds
- **THEN** requests on that scope answer 403 `not-admitted`, and its folder is unchanged

#### Scenario: A tampered tree on disk
- **WHEN** an operator hand-edits `<data>/scopes/<scope_id>/manifest/2.json` and restarts the relay
- **THEN** stderr names the scope id as invalid once, every request on it answers 403 `invalid`, its devices cannot open a pairing nameplate, and the folder is unchanged

#### Scenario: A copied file:// folder
- **WHEN** an operator copies a valid `file://` transport folder of an admitted owner into an empty `--data` and starts the relay
- **THEN** the relay serves its scopes as if they had been created through it

### Requirement: Limits
The relay SHALL refuse, with 507 `quota`, a manifest 1 beyond `--max-scopes` valid scopes of its owner (default 16 per owner), and a create that would take a scope past `--max-scope-mb` MiB (default 1024), counting stored objects and bodies still being received. It SHALL refuse with 413 `too-large` a segment above `--max-object-mb` MiB (default 16) and a manifest above 1 MiB, before reading the body. Each limit SHALL be a whole number from 1 to 1,048,576, or a usage error.

#### Scenario: A full scope
- **WHEN** a scope holds 1,023.5 MiB under the default and a device sends a 1 MiB segment
- **THEN** the relay answers 507 `quota` and stores nothing, and reads of the scope still work

#### Scenario: An oversized segment
- **WHEN** a device sends a segment with `Content-Length` of 17 MiB under the default
- **THEN** the relay answers 413 `too-large` without reading the body

#### Scenario: Uploads in flight count
- **WHEN** a scope holds 1,000 MiB and two devices each start sending a 16 MiB segment at once
- **THEN** one is answered 201 and the other 507 `quota`

#### Scenario: Too many scopes
- **WHEN** the relay holds 16 scopes of one owner and that owner's 17th manifest 1 arrives
- **THEN** the relay answers 507 `quota`

#### Scenario: A bad limit
- **WHEN** a user passes `--max-scope-mb 0`
- **THEN** stderr names `--max-scope-mb`, and the exit code is 2

### Requirement: Slow connections
The relay SHALL serve one request per connection and at most 256 connections at once. It SHALL answer 431 to request headers above 16 KiB, close a connection whose request line and headers have not arrived 10 seconds after it was accepted, close one whose body sends nothing for 30 seconds, and close one that has not finished its request and response 300 seconds after it was accepted. At 256 open connections it SHALL answer a new one at once with 503 `busy` and close it.

#### Scenario: A stalled client
- **WHEN** a client opens a connection and sends half a request line, then nothing
- **THEN** the relay closes the connection 10 seconds after accepting it

#### Scenario: A body that drips
- **WHEN** a client sends a `PUT` body one byte every 20 seconds
- **THEN** the relay closes the connection 300 seconds after accepting it, stores nothing and keeps no temporary file

#### Scenario: Stalled clients below the cap
- **WHEN** 200 clients each hold a connection with half a request line
- **THEN** a listed device's request on a new connection is served

#### Scenario: At the cap
- **WHEN** 256 connections are open and another arrives
- **THEN** it is answered 503 `busy` with `Retry-After` at once, and served once a connection closes

### Requirement: What the relay logs
The relay SHALL print to stderr one line per created object: `manifest` or `segment` with the scope id, device id, n or seq and size, or `mailbox` with only the size. It SHALL print one line per 4xx refusal, other than 404, of a request whose signature verified under a key it knows (a device a held scope's latest manifest lists, an admitted owner, or the nameplate's opener), one line per 500 or 507 it answers, naming the status, reason, method and kind of object, and one line a minute counting all other 4xx refusals. No line after the startup line SHALL hold a peer address; no line SHALL hold object bytes, headers or a nameplate.

#### Scenario: A create is logged
- **WHEN** a device creates segment 12 of a scope
- **THEN** stderr holds one line naming `segment`, the scope id, the device id, `12` and the byte count

#### Scenario: Nothing personal in the log
- **WHEN** a pairing runs through nameplate `42`, and a request from the same peer is refused
- **THEN** no stderr line after the startup line holds `42` as a nameplate, the peer's address or any message bytes

#### Scenario: A flood stays short in the log
- **WHEN** a stranger sends 10,000 unsigned requests under `/v1/scopes/` within a minute
- **THEN** stderr gains one line that counts them, not 10,000

#### Scenario: Reads stay quiet
- **WHEN** a device reads 500 segments
- **THEN** the relay prints no line for them

### Requirement: NixOS module
The flake SHALL export `nixosModules.relay` with `services.bilbo-relay.enable`, `package`, `owners` (a non-empty list of fingerprints), `listen` (default `127.0.0.1:8738`) and `maxScopes`, `maxScopeMb` and `maxObjectMb` (null keeps the relay's default). When enabled, it SHALL run `bilbo relay --data /var/lib/bilbo-relay` with those flags as the system service `bilbo-relay.service`, under a dynamic user, with that folder at mode 0700, restarting on failure.

#### Scenario: The service on a NixOS host
- **WHEN** a NixOS configuration sets `services.bilbo-relay = { enable = true; owners = [ "<fingerprint>" ]; }`
- **THEN** `bilbo-relay.service` runs `bilbo relay --data /var/lib/bilbo-relay --owner <fingerprint> --listen 127.0.0.1:8738`

#### Scenario: No owners
- **WHEN** a configuration enables the module with `owners = [ ]`
- **THEN** evaluation fails with a message naming `services.bilbo-relay.owners`
