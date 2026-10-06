# Spec Delta

## Purpose
The HTTP API a `bilbo relay` serves under `/v1/`: the transport tree as create-only objects, the signatures that admit a request, how a scope and its manifest chain enter the relay, and the pairing mailbox.

## ADDED Requirements

### Requirement: The API root
The relay SHALL serve its API under `/v1/`, with the paths of the transport tree (`scopes/<scope_id>/manifest/<n>.json`, `scopes/<scope_id>/devices/<device_id>/<seq>.seg`, `pair/<nameplate>/<name>.msg`) below it. `GET /v1/` SHALL answer 200 with `{"relay":"bilbo","api":1}` and needs no signature. A method other than `GET` or `PUT` SHALL answer 405. A successful `PUT` SHALL answer with an empty body.

#### Scenario: The root answers
- **WHEN** a client sends `GET /v1/` with no signature headers
- **THEN** the relay answers 200 with `{"relay":"bilbo","api":1}`

#### Scenario: A delete is not allowed
- **WHEN** a device listed in the scope's manifest sends a signed `DELETE` of one of its own segments
- **THEN** the relay answers 405 and the segment is still there

#### Scenario: A create answers with no body
- **WHEN** a listed device creates a segment
- **THEN** the relay answers 201 with `Content-Length: 0`

### Requirement: Request framing
The relay SHALL answer 400 `bad-request`, before any signature check, to a request with any `Transfer-Encoding` header, more than one `Content-Length` or one that is not plain decimal digits, no `Host`, an `Expect` other than `100-continue`, or a target that is not origin-form starting with `/v1/` or whose path and query do not match the tree's grammar byte for byte. Percent-escapes, `.` and `..` segments SHALL NOT match. It SHALL route and verify on that same unmodified target.

#### Scenario: Smuggling shape
- **WHEN** a `PUT` carries both `Transfer-Encoding: chunked` and `Content-Length: 10`
- **THEN** the relay answers 400 `bad-request` and stores nothing

#### Scenario: Two lengths
- **WHEN** a request carries `Content-Length: 10` twice, or `Content-Length: +10`
- **THEN** the relay answers 400 `bad-request`

#### Scenario: A path outside the tree
- **WHEN** a client sends `GET /v2/scopes/`, `GET /`, `GET http://relay/v1/` or `GET /v1/scopes/%41/devices/`
- **THEN** the relay answers 400 `bad-request`

#### Scenario: A malformed id
- **WHEN** a signed request names `scopes/not-an-id/devices/`
- **THEN** the relay answers 400 `bad-request`

#### Scenario: Garbage on the socket
- **WHEN** a client sends bytes that are not an HTTP request
- **THEN** the relay answers 400 `bad-request` and closes the connection

### Requirement: Request bodies
A `PUT` SHALL carry `Content-Length`, or the relay SHALL answer 411. When it carries `Expect: 100-continue` and its headers pass every check that needs no body, the relay SHALL send `100 Continue` before reading the body. When the relay refuses a request before reading its body, it SHALL send the response, stop writing, read and discard what the client still sends for up to 2 seconds, then close.

#### Scenario: No length
- **WHEN** a `PUT` carries no `Content-Length`
- **THEN** the relay answers 411 and stores nothing

#### Scenario: A proxy that expects continue
- **WHEN** a `PUT` of a segment carries `Expect: 100-continue`
- **THEN** the relay sends `100 Continue`, reads the body and answers 201

#### Scenario: The refusal reaches a client still sending
- **WHEN** a client sends a 17 MiB segment and the relay refuses it before reading the body
- **THEN** the client reads the 413 `too-large` answer rather than a connection reset

### Requirement: Error responses
Every response SHALL carry `Bilbo-Time: <the relay's clock in Unix seconds>`. A response with a 4xx or 5xx status SHALL carry the body `{"error":"<reason>"}`, where the reason is one of `bad-request`, `signature`, `clock`, `replay`, `not-admitted`, `invalid`, `not-found`, `exists`, `not-next`, `manifest`, `too-large`, `quota`, `rate`, `busy` and `internal`, as the requirements below assign them. A 429 or 503 SHALL carry `Retry-After`.

#### Scenario: A refusal names its reason
- **WHEN** a request is refused because its signature does not verify
- **THEN** the status is 401, the body is `{"error":"signature"}`, and the response carries `Bilbo-Time`

### Requirement: Order of checks
The relay SHALL check a request in this order and answer the first failure: framing and grammar (400, 411), method (405), then for a signed request the time window (401 `clock`), the signature (401 `signature`) and the nonce (401 `replay`), then admission (403), then the object (404, 409, 413, 507). A nonce SHALL be recorded only after its signature verifies, and only for a key the relay knows: a device a held scope's latest manifest lists, or an admitted owner.

#### Scenario: A forged request does not burn a nonce
- **WHEN** a request with a bad signature uses a nonce, and a valid request then uses the same key and nonce
- **THEN** the first answer is 401 `signature` and the second is served

#### Scenario: A stranger learns nothing of a scope
- **WHEN** a key that is nobody signs a `GET` of a scope the relay does not hold
- **THEN** the relay answers 403 `not-admitted`, as for a scope it holds

### Requirement: Signed requests
Every signed request SHALL carry `Bilbo-Key` (the 64-hex Ed25519 public key), `Bilbo-Time` (Unix seconds), `Bilbo-Nonce` (32 hex) and `Bilbo-Signature` (128 hex), verifying under that key over these lines joined by `\n`: `bilbo-relay-1`, the method, the target, the time, the nonce, and the hex SHA-256 of the body (of no bytes for an empty body). Every request under `/v1/scopes/` SHALL be signed. A signature that is missing there or does not verify anywhere SHALL answer 401 `signature`.

#### Scenario: A valid signature
- **WHEN** a device listed in a scope's latest manifest signs `GET /v1/scopes/<scope_id>/devices/` as described
- **THEN** the relay answers 200

#### Scenario: A body changed in transit
- **WHEN** a request's body differs by one byte from the body its signature hashed
- **THEN** the relay answers 401 `signature` and stores nothing

#### Scenario: No signature
- **WHEN** a request under `/v1/scopes/` carries no `Bilbo-Signature`
- **THEN** the relay answers 401 `signature`

#### Scenario: A path the proxy kept
- **WHEN** the client's URL is `https://relay.example/bilbo`, a proxy strips `/bilbo`, and the client signed the target from `/v1/` on
- **THEN** the signature verifies

### Requirement: Time window and replays
The relay SHALL answer 401 `clock` to a signed request whose `Bilbo-Time` is more than 300 seconds from its own clock, and 401 `replay` to a signed request whose key and nonce it has accepted in the last 600 seconds. This SHALL hold for every signed request, the mailbox's included. A relay restart MAY forget nonces; nothing else SHALL.

#### Scenario: A clock too far off
- **WHEN** a request is signed with a time 301 seconds behind the relay's clock
- **THEN** the relay answers 401 `clock`, and `Bilbo-Time` in the response gives the relay's time

#### Scenario: A replayed request
- **WHEN** a signed `GET` is accepted and the same bytes are sent again 10 seconds later
- **THEN** the second answer is 401 `replay`

#### Scenario: A replayed nameplate opener
- **WHEN** a captured signed `PUT /v1/pair/42/a.msg` is sent again after its nameplate expired
- **THEN** the relay answers 401 `clock` or 401 `replay` and opens nothing

### Requirement: Who may use a scope
A request on a scope, other than a manifest create, SHALL be accepted only when its key is the `sign` key of a device the scope's latest manifest lists, or the scope's `owner` key for reading its manifests. A device SHALL create segments only under its own `devices/<device_id>/`. A manifest create SHALL be admitted by the two requirements below instead. Anything else SHALL answer 403 `not-admitted`, whether or not the relay holds the scope. A scope that failed the relay's start-up check SHALL answer 403 `invalid` instead, as the `relay-server` spec says.

#### Scenario: A revoked device
- **WHEN** manifest 3 dropped a device, and that device signs a `GET` of a segment in the scope
- **THEN** the relay answers 403 `not-admitted`

#### Scenario: Writing another device's folder
- **WHEN** a listed device signs a `PUT` of `scopes/<scope_id>/devices/<another device_id>/00000000000000000001.seg`
- **THEN** the relay answers 403 `not-admitted` and stores nothing

#### Scenario: The owner reads the manifests
- **WHEN** a request signed by the scope's owner key, which no device entry lists, reads `manifest/latest`
- **THEN** the relay serves it

#### Scenario: The owner cannot read segments
- **WHEN** a request signed by the scope's owner key lists device folders or reads a segment
- **THEN** the relay answers 403 `not-admitted`

#### Scenario: A key that is nobody
- **WHEN** a request is signed by a valid key that is neither a listed device nor the owner
- **THEN** the relay answers 403 `not-admitted`

### Requirement: Admitting a scope
A scope SHALL enter the relay only through `PUT /v1/scopes/<scope_id>/manifest/1.json` whose body is a manifest that: names `<scope_id>` in `scope`, has `n` 1 and `prev` null, carries an `owner` whose fingerprint is admitted, has a `chain` with one entry per epoch from 1 to its `epoch` minus 1, in order, verifies under that owner's signature, and lists the request's key as a device, or the request is signed by that owner. A manifest that fails a check SHALL answer 422 `manifest`, and an owner that is not admitted SHALL answer 403 `not-admitted`.

#### Scenario: A new scope
- **WHEN** a device sends its scope's first manifest, signed by the admitted owner and listing that device
- **THEN** the relay answers 201, and the device can then create its segments

#### Scenario: Another owner's scope
- **WHEN** a manifest 1 is signed by an owner key whose fingerprint the relay was not started with
- **THEN** the relay answers 403 `not-admitted` and keeps nothing of the scope

#### Scenario: A forged owner signature
- **WHEN** a manifest 1 names the admitted owner key but its `sig` does not verify
- **THEN** the relay answers 422 `manifest`

#### Scenario: The uploader is not listed
- **WHEN** a valid manifest 1 is sent by a device key it does not list, which is not the owner key
- **THEN** the relay answers 403 `not-admitted`

### Requirement: The manifest chain
A manifest create at an `n` already stored SHALL follow the create-only rule. For n above the latest, the relay SHALL accept it only when n is the latest plus one, its `prev` is the SHA-256 of manifest n-1's stored bytes, its `owner` equals n-1's, its `scope` and `n` match the path, its `chain` holds one entry per epoch from 1 to its `epoch` minus 1, in order, with every entry of manifest n-1 unchanged, its signature verifies, and it lists the request's key as a device or the request is signed by the owner. A larger n SHALL answer 409 `not-next`; any other failed check, 422 `manifest`.

#### Scenario: Enrolling a device
- **WHEN** a listed device sends manifest 4 after 3, with `prev` the hash of 3, signed by the same owner, adding a device
- **THEN** the relay answers 201, and the added device's requests are accepted from then on

#### Scenario: A skipped version
- **WHEN** the latest manifest is 3 and a device sends manifest 5
- **THEN** the relay answers 409 `not-next`

#### Scenario: Two devices race
- **WHEN** two devices each send a different manifest 4
- **THEN** one answer is 201 and the other is 409 `exists`, and manifest 4 holds the first one's bytes

#### Scenario: An older version resent
- **WHEN** the latest manifest is 4 and a device sends manifest 3 with other bytes than the stored 3
- **THEN** the relay answers 409 `exists`

#### Scenario: A changed owner
- **WHEN** manifest 4 is signed by a different owner key than manifest 3, even an admitted one
- **THEN** the relay answers 422 `manifest`

#### Scenario: A chain with an epoch missing
- **WHEN** manifest 4 is at epoch 3, correctly signed by the owner, and its `chain` holds an entry for epoch 1 but none for epoch 2
- **THEN** the relay answers 422 `manifest` and stores nothing

#### Scenario: A rewritten chain entry
- **WHEN** manifest 4 is correctly signed and complete, but its `chain` entry for epoch 1 differs from manifest 3's
- **THEN** the relay answers 422 `manifest`

#### Scenario: A wrong prev
- **WHEN** manifest 4's `prev` is not the SHA-256 of the stored manifest 3
- **THEN** the relay answers 422 `manifest`

#### Scenario: Recovery from the phrase
- **WHEN** a device that manifest 3 does not list sends a valid manifest 4 that lists it, signing the request with the owner key
- **THEN** the relay answers 201

### Requirement: Create-only objects
A `PUT` SHALL first check whether the object exists: with the same bytes it SHALL answer 200 and change nothing; with other bytes, 409 `exists`. Only then SHALL it apply the path's own rules, and create the object with 201. A segment SHALL be accepted only as seq 1 or the next seq after the device's highest, and otherwise answer 409 `not-next`. The relay SHALL NOT read or check a segment's contents.

#### Scenario: The first segment
- **WHEN** a listed device creates `devices/<its id>/00000000000000000001.seg`
- **THEN** the relay answers 201 and serves those bytes from then on

#### Scenario: A retried create
- **WHEN** a device's highest seq is 7, its create of 7 succeeded but the answer was lost, and it sends the same bytes again
- **THEN** the relay answers 200

#### Scenario: An overwrite
- **WHEN** a device sends other bytes to a seq it already created
- **THEN** the relay answers 409 `exists` and the stored bytes do not change

#### Scenario: A gap in seq
- **WHEN** a device's highest seq is 7 and it sends seq 9
- **THEN** the relay answers 409 `not-next`

#### Scenario: Opaque segments
- **WHEN** a listed device creates a segment whose bytes are not JSON
- **THEN** the relay stores it like any other

### Requirement: Reading objects
`GET` of an object path SHALL answer 200 with the exact stored bytes, or 404 `not-found`. `GET /v1/scopes/<scope_id>/manifest/latest` SHALL answer 200 with the highest manifest's bytes and `Bilbo-Manifest: <n>`.

#### Scenario: The latest manifest
- **WHEN** a listed device reads `manifest/latest` of a scope holding manifests 1 to 4
- **THEN** the body is manifest 4's stored bytes and `Bilbo-Manifest` is `4`

#### Scenario: A missing segment
- **WHEN** a listed device reads a seq past the highest one
- **THEN** the relay answers 404 `not-found`

### Requirement: Listings
`GET /v1/scopes/<scope_id>/devices/` SHALL answer `{"devices":[{"id":"<device_id>","last":<highest seq>}]}`, sorted by id, for every device folder that holds a segment. `GET /v1/scopes/<scope_id>/devices/<device_id>/?after=<seq>` SHALL answer `{"seqs":[...],"more":<bool>}` with at most 1,000 seqs above `after`, ascending, and `more` true when more follow. A missing `after` SHALL mean 0.

#### Scenario: Device folders
- **WHEN** two devices have written seqs 1 to 3 and 1 to 12
- **THEN** the listing names both, with `last` 3 and 12

#### Scenario: A long folder
- **WHEN** a device folder holds seqs 1 to 2,500 and a device lists it after 0
- **THEN** the answer holds seqs 1 to 1,000 and `more` is true

#### Scenario: A bad cursor
- **WHEN** a device lists a folder with `after=-1` or `after=x`
- **THEN** the relay answers 400 `bad-request`

### Requirement: Listing an owner's scopes
`GET /v1/scopes/` signed by an admitted owner key SHALL answer `{"scopes":["<scope_id>",...]}`, sorted, holding every valid scope whose manifests that key signs. Signed by any other key, it SHALL answer 403 `not-admitted`.

#### Scenario: Recovery finds the scopes
- **WHEN** a device restored from the recovery phrase signs `GET /v1/scopes/` with the owner key
- **THEN** the answer lists every scope of that owner the relay holds, and no other owner's

#### Scenario: A device key cannot list
- **WHEN** a listed device signs `GET /v1/scopes/` with its device key
- **THEN** the relay answers 403 `not-admitted`

### Requirement: The pairing mailbox
`pair/<nameplate>/<name>.msg`, nameplate and name each of `a-z`, `0-9` and `-` (1 to 64 and 1 to 16 characters), SHALL be create-only. A nameplate's first message SHALL be signed by a device that the latest manifest of a valid, admitted scope lists. Later messages SHALL be signed by that same key, except one unsigned message per nameplate. `GET` SHALL need no signature. Any other message SHALL answer 403 `not-admitted`.

#### Scenario: An enrolled device opens a nameplate
- **WHEN** a listed device sends a signed `PUT /v1/pair/42/a.msg`
- **THEN** the relay answers 201

#### Scenario: The new device answers
- **WHEN** a device with no manifest entry sends an unsigned `PUT /v1/pair/42/b.msg` to that open nameplate, and polls `GET /v1/pair/42/c.msg`
- **THEN** the `PUT` answers 201 and the `GET` answers 404 until `c.msg` exists

#### Scenario: A stranger cannot open a nameplate
- **WHEN** an unsigned `PUT /v1/pair/91/a.msg` arrives and nameplate `91` holds no message
- **THEN** the relay answers 403 `not-admitted` and stores nothing

#### Scenario: An unsigned second message
- **WHEN** `b.msg` was created unsigned, and an unsigned `PUT /v1/pair/42/c.msg` arrives
- **THEN** the relay answers 403 `not-admitted`, and the opener's signed `c.msg` is still accepted afterwards

#### Scenario: Another enrolled device cannot reply
- **WHEN** a device listed in a held scope, but not the opener, sends a signed `PUT /v1/pair/42/c.msg`
- **THEN** the relay answers 403 `not-admitted`

#### Scenario: No room for junk slots
- **WHEN** a stranger has created `b.msg` unsigned and then sends unsigned `x.msg`, `y.msg` and `z.msg`
- **THEN** each answers 403 `not-admitted`, and the opener can still create its 8 messages

#### Scenario: A second answer to the same slot
- **WHEN** `b.msg` exists and another client sends other bytes to `b.msg`
- **THEN** the relay answers 409 `exists`

### Requirement: Mailbox limits
A message SHALL be at most 4 KiB (413 `too-large`). A nameplate SHALL hold at most 8 messages and at most 32 SHALL be open; beyond, 507 `quota`. A nameplate SHALL be deleted 30 minutes after its first message. Per peer address, mailbox requests that no key the relay knows signed SHALL be limited to 60 a minute and to 4 distinct nameplates in 10 minutes, beyond which they get 429 `rate`. The relay SHALL keep peer addresses in memory only.

#### Scenario: An expired nameplate
- **WHEN** a nameplate was opened 31 minutes ago
- **THEN** `GET` of its messages answers 404 and its folder is gone from the data folder

#### Scenario: An oversized message
- **WHEN** a client sends a 5,000-byte message
- **THEN** the relay answers 413 `too-large`

#### Scenario: A flood from one address
- **WHEN** one peer address sends its 61st unsigned mailbox request within a minute
- **THEN** the relay answers 429 `rate` with `Retry-After`, and another peer address is still served

#### Scenario: Probing for open nameplates
- **WHEN** one peer address sends unsigned `GET`s to nameplates 1, 2, 3, 4 and then 5 within 10 minutes
- **THEN** the request to nameplate 5 answers 429 `rate`, while that address's polls of nameplates 1 to 4 are still served

#### Scenario: The opener is not rate-limited as a stranger
- **WHEN** the opener polls `b.msg` with signed `GET`s every 2 seconds for 10 minutes
- **THEN** none of its requests is answered 429

#### Scenario: A stranger's signed polls count
- **WHEN** one peer address sends signed `GET`s for 5 distinct nameplates, signed by a key no held scope lists and no admitted owner holds
- **THEN** the fifth is answered 429 `rate`
