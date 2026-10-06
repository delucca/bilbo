# Run a relay

A relay is the server version of the sync folder: a small program you run on a
machine you control, so your devices can sync without a shared folder. This page
shows how to run one, point a scope at it, move a folder scope onto it, and
repair it. [Security](../security.md#what-the-relay-sees) says what it can see.

The relay is `bilbo relay`. It speaks plain HTTP on `127.0.0.1:8738` for a TLS
proxy in front of it. It holds only the encrypted, signed files of
[Sync](sync.md), in the same tree a `file://` folder has.

```sh
bilbo relay --data /var/lib/bilbo-relay --owner yb4b-5aju-v6zb-x2nm-nc5x-ompf
```

`--owner` is the fingerprint `bilbo device` prints, in any case and with or
without the hyphens, and the flag repeats: the relay serves scopes of the owners
it is given and nobody else. There is no password or token to set; a device
proves it is yours by signing each request with the keys it already has. The
relay prints `relay listening on http://127.0.0.1:8738` once it is ready.

| Flag | Meaning |
| --- | --- |
| `--data <dir>` | Where the relay keeps its files, mode 0700, created when missing. One relay per folder. |
| `--owner <fingerprint>` | An owner the relay serves. At least one. |
| `--listen <address:port>` | Where to listen. `127.0.0.1:8738` by default; a non-loopback address prints a warning and serves anyway. |
| `--max-scopes <n>` | Scopes per owner. 16 by default. |
| `--max-scope-mb <n>` | MiB one scope may hold. 1024 by default. |
| `--max-object-mb <n>` | MiB one segment may be. 16 by default. |

A limit is a whole number from 1 to 1,048,576. A request over one is refused and
the device reports it on the scope's line in `bilbo sync`.

The relay allows 256 connections at once. It closes:

- a connection that has not sent its request line and headers 10 seconds after
  it was accepted,
- one whose body sends nothing for 30 seconds, and
- any that is not finished 300 seconds after it was accepted.

It logs one line per object created and per refusal of a device it knows, and a
count of the other refusals once a minute, never an address, a header, a body or
a pairing code.

## Run it on NixOS

The flake exports a module that runs the relay as the system service
`bilbo-relay.service`:

```nix
{ inputs, ... }:
{
  imports = [ inputs.bilbo.nixosModules.relay ];

  services.bilbo-relay = {
    enable = true;
    owners = [ "yb4b-5aju-v6zb-x2nm-nc5x-ompf" ];
    # listen = "127.0.0.1:8738";
    # maxScopes = 16; maxScopeMb = 1024; maxObjectMb = 16; # null keeps the default
  };
}
```

`owners` must not be empty. The service runs under a dynamic user with its data
in `/var/lib/bilbo-relay` and restarts on failure. `package` defaults to this
flake's `bilbo`.

## Run it with systemd

On any other Linux, install `bilbo` and write the same unit by hand, as
`/etc/systemd/system/bilbo-relay.service`:

```ini
[Unit]
Description=bilbo relay
After=network.target

[Service]
ExecStart=/usr/local/bin/bilbo relay --data /var/lib/bilbo-relay --owner yb4b-5aju-v6zb-x2nm-nc5x-ompf
DynamicUser=yes
StateDirectory=bilbo-relay
StateDirectoryMode=0700
UMask=0077
Restart=on-failure
RestartSec=10
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
NoNewPrivileges=yes
RestrictAddressFamilies=AF_INET AF_INET6
CapabilityBoundingSet=
SystemCallFilter=@system-service
SystemCallArchitectures=native
ProtectKernelTunables=yes
ProtectKernelModules=yes
ProtectControlGroups=yes
RestrictNamespaces=yes
RestrictRealtime=yes
LockPersonality=yes
MemoryDenyWriteExecute=yes
ProtectProc=invisible

[Install]
WantedBy=multi-user.target
```

Then `systemctl enable --now bilbo-relay`.

## TLS

Devices reach a relay over `https://` only, and trust the public web certificate
roots bilbo carries: a self-signed or private-CA certificate is refused. Put a
proxy in front of the relay.

### With Tailscale

On a machine in your tailnet, `tailscale serve` is the shortest way:

```sh
tailscale cert <host>.<tailnet>.ts.net
tailscale serve --bg 8738
```

HTTPS certificates must be on for the tailnet once, in the admin console. The
relay is then at `https://<host>.<tailnet>.ts.net`, reachable from the tailnet
only.

Run `tailscale cert` once first. Tailscale otherwise issues the certificate on
the first request, which can take longer than bilbo's 10 second connect timeout,
and the first sync reports the relay as unreachable. The next one succeeds.
`tailscale serve` keeps no access log.

### With Caddy

For devices outside a tailnet, Caddy gets and renews a certificate by itself:

```
relay.example.org {
	reverse_proxy 127.0.0.1:8738
}
```

Add its server `timeouts` and `max_header_size` too, so Caddy sheds slow clients
before they reach the relay:

```
{
	servers {
		max_header_size 16KB
		timeouts {
			read_header 10s
			read_body 5m
		}
	}
}
```

Caddy logs requests, addresses included, only when the site has a `log`
directive; leave it out to keep them off the disk.

A proxy that strips a path prefix works, since bilbo signs the path the relay
receives. Set the scope's URL to the prefix, as in [Sync
URLs](sync.md#sync-urls).

### Public exposure

A public relay is exposed to connection exhaustion: enough slow clients can hold
its 256 connections open for 300 seconds at a time. A relay for one person
accepts that. What strangers can and cannot do is in
[Security](../security.md#what-bilbo-protects).

## Use a relay

Run `bilbo device` on a device and pass its fingerprint to the relay. For a new
scope, set the URL on each device, as for a folder:

```
scope.personal.sync = https://relay.example.ts.net
```

A scope that already syncs through a folder [moves by
copy](#move-a-folder-scope-to-a-relay).

`bilbo setup` takes the URL at its `Folder or relay URL to sync through` prompt,
and checks that a relay answers there. `bilbo watch` publishes the scope's
manifest on its first cycle. A device whose clock is off by more than 5 minutes
is told so on the scope's line; fix the clock.

A relay that is down only pauses sync: every device keeps working locally, and
`bilbo watch` retries.

## Move a folder scope to a relay

A scope you already sync through a folder moves by copy, not by changing the URL
alone. A relay started empty refuses a device's later versions of a scope it
holds nothing of, and the watcher does not seed it. Copying also carries every
segment, so a device paired later still reads the whole history.

1. Stop `bilbo watch` on every device.
2. With the relay stopped, copy the synced folder's `scopes/` into the relay's
   data folder, which is `/var/lib/private/bilbo-relay` under the NixOS module.
   Under the module, systemd gives the copied files to the service's user when
   it starts the relay. With the plain unit, `chown -R` them to the user it runs
   as. The relay checks the copied tree when it starts, as it checks any other.
3. Start the relay with the owner's fingerprint.
4. Set `scope.<name>.sync` to the relay's URL on every device.
5. Run `bilbo device init` in a terminal on each device. It sees that the
   manifest pins the folder and the config says the relay, and writes a new
   version that pins the relay. Devices that do this at once race for the same
   version: the first wins on the relay, and each other moves its own copy
   aside, takes the relay's and reports it.
6. Start `bilbo watch` again.

If a device's owner is not among the relay's `--owner` flags yet, `bilbo device
init` says `scope <name> failed: relay <url> does not admit this owner; start it
with --owner <fingerprint>`. Restart the relay with the fingerprint and run
`init` again.

A relay that holds no copy of the scope tells the device so: `relay <url> holds
no scope <scope id> of this owner; copy the folder it synced through into the
relay's data folder`. A relay started empty gets only the versions and segments
written after the move, so a device paired later would miss the older notes.
Copy the folder first.

## Drop or repair a scope

The relay's data folder is a `file://` tree you can read without the relay. Stop
the relay before touching it, and start it again after.

- To drop a scope, remove `<data>/scopes/<scope id>`, and only once no device
  syncs it any more: set `scope.<name>.sync = off` on each, or run `bilbo device
  init` after changing its URL. The scope's id is in `bilbo device`. A device
  that still syncs a dropped scope cannot refill it, and reports `relay <url>
  does not admit this device`.
- To repair a damaged segment, copy `<root>/.bilbo/scopes/<scope
  id>/out/<seq>.seg` from the device that wrote it over `<data>/scopes/<scope
  id>/devices/<device id>/<seq>.seg` (the sequence number in 20 digits, as in
  the file's name there). Readers stop at a damaged segment and pick the fixed
  one up at their next poll.
- A scope that fails the check at start is kept and not served: the relay logs
  it once, naming the scope. A scope whose owner is not among the `--owner`
  flags answers `not-admitted`, so give the relay that owner again.

### Back up the data folder

Back up the data folder. Devices never re-upload what a relay loses: a client
only appends, so a relay that comes back without a scope or some segments stays
that way. A device whose next segment is far past what the relay holds reports
`relay <url> is missing this device's earlier segments; restore its data
folder`. Restore the folder from the backup.

Without a backup, the devices keep their notes, and a device paired later reads
only what the relay holds; the old segments are gone.

The relay never edits or deletes a stored object. The data folder grows until a
scope reaches `--max-scope-mb`; then devices report `relay <url> is full` for
the scope.

## Pair and recover through a relay

[Pairing](devices.md#pair-a-device) works through a relay, with its URL where
the folder was. `bilbo pair` on the enrolled device shows a code, and on the new
one `bilbo pair <code> --via https://relay.example.ts.net` pairs it.

The relay keeps the mailbox for 30 minutes at most and sees only sealed
messages. What a stranger can do to a pairing is in
[Security](../security.md#pairing).

When every device is lost, [the phrase](devices.md#add-a-device-with-the-phrase)
still recovers everything the relay holds. On the new machine, set
`scope.<name>.sync` to the relay URL and run `bilbo device recover`, or run
`bilbo setup` and its wizard with the URL. The relay needs only be started with
your fingerprint.

Revoking works the same through a relay; see [Revoke a
device](devices.md#revoke-a-device). A revoked device can still write to the
relay until you give it a new owner: restart the relay with the new fingerprint
at the end of the [Replace the owner](devices.md#replace-the-owner) steps.
[Security](../security.md#what-revoking-guarantees) has the guarantee.

## See also

- [Sync](sync.md)
- [Devices and the owner](devices.md)
- [Security](../security.md)
