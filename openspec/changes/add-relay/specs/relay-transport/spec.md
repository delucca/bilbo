# Spec Delta

## Purpose
The `https://` transport: how a device syncs a scope and pairs through a `bilbo relay`, which addresses and certificates it accepts, how it signs its requests, and how a relay's refusal reaches the user.

## ADDED Requirements

### Requirement: Relay URLs
A scope whose `scope.<name>.sync` is `https://<host>[:port][/prefix]`, or `http://` to `localhost`, `127.0.0.1` or `::1`, SHALL sync through the relay at that URL, with the API at `<url>/v1/`. The transport SHALL send no request to an `http://` URL whose host is not a loopback host.

#### Scenario: A relay behind tailscale serve
- **WHEN** `scope.personal.sync = https://bree.tail1234.ts.net` and `bilbo watch` runs
- **THEN** the device's segments for `personal` are created under `https://bree.tail1234.ts.net/v1/scopes/<scope_id>/`

#### Scenario: A relay under a path
- **WHEN** the URL is `https://example.org/bilbo`
- **THEN** requests go to `https://example.org/bilbo/v1/...`

#### Scenario: Plain HTTP to another host
- **WHEN** the URL is `http://bree:8738`
- **THEN** bilbo sends no request to `bree`, and the config error names `scope.personal.sync`

### Requirement: Certificates
The transport SHALL verify the relay's certificate and host name against the web PKI roots built into bilbo, and SHALL NOT fall back to an unverified connection. A failed verification SHALL be reported as `relay <url>: certificate not trusted: <reason>`.

#### Scenario: A self-signed certificate
- **WHEN** the relay's proxy presents a self-signed certificate
- **THEN** sync reports `certificate not trusted` for that URL, sends no request body, and changes nothing locally

### Requirement: No redirects
The transport SHALL NOT follow a redirect. A 3xx answer SHALL be reported as `relay <url> redirects; set the scope's URL to the address it redirects to`.

#### Scenario: A proxy that redirects
- **WHEN** the relay URL answers 301 to `https://other.example/`
- **THEN** sync reports the redirect and sends nothing to `other.example`

### Requirement: Signed requests from the device
The transport SHALL sign every request under `/v1/scopes/` as the `relay-api` spec describes, with a fresh random nonce per request: with the owner key, when the device holds it, to list the owner's scopes and to read a scope's manifests, and with the device's Ed25519 key for every other request, manifest creates included. On the device that shows a pairing code, it SHALL sign every request to that nameplate with the device key, the first message included; on the device that answers the code, it SHALL sign no mailbox request.

#### Scenario: Every segment request is signed
- **WHEN** the transport creates, lists and reads segments
- **THEN** each request carries `Bilbo-Key` with the device's public key, `Bilbo-Time`, `Bilbo-Nonce` and `Bilbo-Signature`

#### Scenario: Two identical reads
- **WHEN** the transport reads the same segment twice
- **THEN** the two requests carry different nonces, and neither is refused as a replay

#### Scenario: Manifests are read as the owner
- **WHEN** the watcher of an enrolled device lists the owner's scopes on a relay and reads their manifests, among them a scope whose latest manifest does not list the device
- **THEN** each of those requests carries the owner's public key in `Bilbo-Key`, and none is refused 403

#### Scenario: The new device answers unsigned
- **WHEN** a device answers a pairing code through a relay
- **THEN** its `b.msg` and its polls of the nameplate carry no `Bilbo-Signature`, while every request of the device that showed the code to that nameplate carries one

### Requirement: Clock skew
When the relay answers 401 `clock`, the transport SHALL retry the request once with a fresh nonce, signed with the relay's time from the answer's `Bilbo-Time`, and SHALL keep that offset for its later requests to the relay in the same process. When the retry is refused too, it SHALL report `this device's clock is <n> s off the relay's; fix the clock`.

#### Scenario: A laptop clock 10 minutes off
- **WHEN** the device's clock is 600 seconds behind the relay's
- **THEN** the retried request succeeds, sync goes on, and the next requests need no retry

#### Scenario: A relay that keeps refusing the time
- **WHEN** the retry is answered 401 `clock` again
- **THEN** sync reports the clock message with the measured offset

### Requirement: Refusals reach the user
The transport SHALL turn every failed request into one message naming the relay URL. The watch log SHALL print it after `sync <name>: `, which names the scope, and `bilbo sync` SHALL show it for the scope. A 200 to a create of an existing identical object SHALL count as created. A 502, 503 or 504 without `Bilbo-Time` SHALL be reported as `relay <url> unreachable: the proxy answered <status>`, and any other response without `Bilbo-Time` as `<url> is not a bilbo relay`. A failure SHALL leave the local store unchanged.

#### Scenario: The relay does not admit the owner
- **WHEN** a device creates manifest 1 of `personal` and the relay answers 403 `not-admitted`
- **THEN** sync reports `relay <url> does not admit this owner; start it with --owner <fingerprint>`, with the owner's fingerprint

#### Scenario: A revoked device
- **WHEN** a device's requests on `personal` are answered 403 `not-admitted` after a manifest dropped it
- **THEN** the watch log prints `sync personal: relay <url> does not admit this device`

#### Scenario: A scope the relay found invalid
- **WHEN** the relay answers 403 `invalid` on `personal`
- **THEN** the watch log prints `sync personal: relay <url> holds an invalid copy of this scope; repair the relay's data folder`

#### Scenario: A full scope
- **WHEN** the relay answers 507 `quota` to a segment of `personal`
- **THEN** the watch log prints `sync personal: relay <url> is full`, and the push is retried later

#### Scenario: The relay is down behind its proxy
- **WHEN** `tailscale serve` answers 502 because the relay process is stopped
- **THEN** sync reports `relay <url> unreachable: the proxy answered 502`

#### Scenario: No room for a pairing
- **WHEN** the relay answers 507 `quota` to a mailbox message
- **THEN** pairing reports `relay <url> has no room for a pairing now; try again later`

#### Scenario: Not a relay
- **WHEN** the URL points at a web server that answers 404 without `Bilbo-Time`
- **THEN** sync reports `<url> is not a bilbo relay`

#### Scenario: A lost answer
- **WHEN** a segment create times out after the relay stored it, and the retry is answered 200
- **THEN** the transport counts the segment as created and does not report an error

### Requirement: Timeouts
The transport SHALL give up on a connection after 10 seconds and on a request after 300 seconds, reporting `relay <url> unreachable: <reason>`.

#### Scenario: The relay is down
- **WHEN** nothing listens at the relay URL
- **THEN** sync reports `relay <url> unreachable` within 10 seconds and keeps working on local notes
