# Design

## Context

- **`add-device-keys`** (archived) gives every enrolled device the owner signing seed and its own device key in `<state>/bilbo/keys/` (`owner.key`, `device.key`). No device holds the owner X25519 box secret, which only the phrase derives; `owner.key` keeps the owner's box public key beside the seed, and every manifest version names it as `owner_box`.
  - It writes per-scope manifests under `<root>/.bilbo/scopes/<scope_id>/`, numbered from 1 with no gap and verified as a chain.
  - Without pairing, a device joins a scope only through `bilbo device recover`, which needs the 12-word phrase. `recover` also runs on an enrolled device, to add it to local manifests that do not list it yet.
  - A device opens only its own `sealed` entry. An enrolled device that a scope does not list therefore cannot read that scope, and cannot add itself either: a valid version's `chain` needs an entry for the current epoch, which members check against the key they hold (Manifest validity, Chain check by a member).
  - Manifests pin only the scheme `file://` for a folder, so each device keeps its own path to the folder in its config.
  - Its library modules live in the `identity` domain: `identity/keys.rs` (every crypto primitive: `random` over `getrandom::fill`, `encrypt` and `decrypt` (XChaCha20-Poly1305 with a random nonce), `SignKey::sign` and `verify` (Ed25519), `seal_epoch` and `open_epoch` (HPKE), the key files), `identity/phrase.rs` (the embedded BIP39 list, `word`, and `lookup` of a word or its first 4 letters), `identity/manifest.rs` (format, verification, sealing, writing versions: `add_device`, `adopt`, `verify_scope`, `open`, `survey`) and `identity/ceremony.rs` (the phrase screens). SHA-256 is `shared::hash::sha256`.
  - It writes an identity whole, under `<state>/bilbo/keys.lock`: `keys::write_identity` puts both key files into `<state>/bilbo/keys.new/`, removing a leftover first, and `host::swap::rename_new` moves that folder to `keys`. A `keys` folder holding one file is a damaged identity.
  - Its verb, `identity/device.rs`, takes the terminal test as a parameter, `device::run(args, env, terminal, prompter)`, so its tests need no terminal and no hook. It turns the terminal off when `CLAUDECODE` or `CODEX_THREAD_ID` is set and not empty.
- **`add-sync`** (archived) adds the `sync` domain. `sync/transport.rs` holds the `Transport` trait (create-only objects, get, the listings of scopes, devices and seqs, `highest_manifest`, `remove_mailbox`) and its `file://` folder; `transport::open(url, device)` picks it by scheme and refuses any other.
  - Its `sync-transport` spec owns the mailbox layout: `pair/<nameplate>/<name>.msg`, with nameplates of 1 to 64 and names of 1 to 16 characters from `[a-z0-9-]` (`transport::message_path`).
  - It also owns `remove_mailbox(nameplate)`, the one deletion the `file://` transport makes, only under `pair/`. It carries `expect(dead_code)` until pairing calls it.
  - `sync/scopes.rs::chain` fetches a scope's versions from a transport and verifies their chain; `bilbo device recover` uses it.
  - `bilbo watch` rereads the config, keys and manifests every sync cycle, publishes the local manifest versions the transport lacks, and confirms its own pending ones on reading them back.
  - Its advice to a device outside a scope (`sync/manifests.rs`'s not-in-the-scope line) and `recover`'s `unsealed` hint name only `bilbo device recover` so far.
- **`add-relay`** (not yet applied) adds the `https://` client and a relay that serves `pair/<nameplate>/<name>.msg`:
  - the first message of a nameplate must be signed by a listed device;
  - every later one must be signed by the same key, except one unsigned message per nameplate;
  - each message is at most 4 KiB, a nameplate holds at most 8 and at most 32 are open;
  - a nameplate is deleted 30 minutes after its first message;
  - unsigned mailbox requests are limited to 60 a minute and 4 distinct nameplates in 10 minutes per address.
- **I/O today:** `src/main.rs` is the only writer of stdout and stderr. `setup` and `watch` get a line printer as a callback and `digest` and `cite` get stdin as a reader. `pair` needs both: it prints the code, waits, then asks a question.
- **The config file** is written only by `setup`, through `write_config` in `src/setup/apply.rs`: a temporary file, then a rename, with the old file kept as `<name>.bak`. `shared/config.rs` parses it.
- **The layout rules** (AGENTS.md, checked by `tests/layout.rs`): a verb lives in the domain it serves and only `main` uses it; a verb that outgrows one file is a folder whose files reach each other through `super::`; domain library code forms no cycle, and of `identity` and `note` only verbs use `sync`; each crate stays in the files `PLACEMENT` lists.
- **A new device** has neither of the places where scope names, ids and URLs live: the config (`scope.<name>.sync`) and the manifests.

## Goals / Non-Goals

**Goals:**
- A typed code of about 25 characters that gives an online attacker one guess with odds of 1 in 2^33, and that a person can read aloud.
- No secret leaves A before A knows the code was right and the user has confirmed.
- One flow for both transports, built only on create, get, list and the mailbox removal, so the folder transport and the relay need nothing pairing-specific beyond a mailbox path and its limits.
- The whole exchange testable offline, in one process, with no hook in the shipped binary.

**Non-Goals:**
- Making scope selection a cryptographic boundary. See "What A sends".
- Long polling or push. Both sides poll.

## Decisions

### SPAKE2 through `spake2` 0.5.0-pre.0, in `src/identity/pake.rs` only

- **Why a PAKE:** the code has 33 bits of entropy. Anything that turns it into a key directly (a KDF over the code, then a box) gives whoever reads the mailbox an offline search over 2^33 codes, which takes minutes. SPAKE2 gives an active attacker one guess per run and a passive one nothing. The crate's README says the same: "An active attacker (man-in-the-middle) gets exactly one guess at the password".
- **Why this crate and version:** measured on 2026-10-03 in a scratch crate next to the crates `add-device-keys` adds (`ed25519-dalek` 3.0.0, `x25519-dalek` 3.0.0, `hpke` 0.14.1, `chacha20poly1305` 0.11.0, `hkdf` 0.13.0, `sha2` 0.11.0):
  - **0.5.0-pre.0** (January 2026) depends on `curve25519-dalek` 5, `hkdf` 0.13, `sha2` 0.11 and `rand_core` 0.10. Its requirements resolve to the stable releases `add-device-keys` pins. Re-measured on 2026-10-05 against release 0.13.0: adding the line locks one package, `spake2` itself; `curve25519-dalek` 5.0.0 gains its `rand_core` dependency (`rand_core` 0.10.1 is already in the lock); `cargo tree -d -e normal` still lists only `getrandom` 0.2 and 0.4 and `syn` 2 and 3; 0.5.0-pre.0 is still the newest release. `cargo tree -d` shows it adds no duplicate crate. MSRV 1.85.
  - **0.4.0**, the last stable release (July 2023), depends on `curve25519-dalek` 4, `hkdf` 0.12, `sha2` 0.10 and `rand_core` 0.6. Beside `add-device-keys`' crates it duplicates nine crates, eleven counting two that depend on the target: among them a second `curve25519-dalek`. Two copies of the curve code in a security binary is the cost that decides it.
  - **The protocol is the same in both.** The review diffed the two tarballs' `src/`: the 110 changed lines are the `rand_core` 0.10 trait rename, visibility and doc changes. The protocol code is the 2023 code.
- **How it is used:** default features off.
  - The asymmetric mode stops two A sides from pairing with each other.
  - The password is `bilbo-pair-1:` followed by the canonical code. The identities are `bilbo-pair-1 a` and `bilbo-pair-1 b`.
- **The random source:** `start_a_with_rng` and `start_b_with_rng` want `rand_core` 0.10's `CryptoRng`, which is `TryCryptoRng<Error = Infallible>`, and `keys::random` is a fill function that returns `Result<[u8; N], String>`, not an RNG object.
  - `src/identity/pake.rs` holds `KeysRng`, an adapter that implements `rand_core::TryRng` with `Error = Infallible` and `TryCryptoRng` over `keys::random`, filling in 32-byte chunks. `rand_core`'s blanket impls then make it `Rng + CryptoRng`. A failure of the system random source cannot be returned through `Infallible`, so the adapter treats it as fatal, as `rand_core`'s own `UnwrapErr` does.
  - So `getrandom` keeps its one user, `src/identity/keys.rs`. `rand_core` gets a direct user, `src/identity/pake.rs`, through the `spake2::rand_core` re-export, with no `Cargo.toml` entry.
  - A scratch crate built this adapter shape on 2026-10-03, and a scratch copy of release 0.13.0 built it inside `identity/pake.rs` on 2026-10-05, with clippy and `tests/layout.rs` passing.
- **Pinning:** `spake2 = "0.5.0-pre.0"` in `Cargo.toml` (the caret idiom), held by `Cargo.lock`. `cargo update` would move it to 0.5.0 when that is released. Two unit tests guard the protocol:
  - **The crate's own vector:** `test_asymmetric`, password `password`, identities `idA` and `idB`, key `712295de7219c675ddd31942184aa26e0a957cf216bc230d165b215047b520c1`. Through the public API, a fixed-bytes `TryRng` feeds each side its scalar's 32 little-endian bytes, then 32 zero bytes. `Scalar::random` reduces 64 bytes modulo the group order, so the scalar comes out unchanged. The scratch crate reproduced the key this way, and so did the 0.13.0 scratch copy (2026-10-05), with the crate's `msg1` `416fc960…1f2af9` as well.
  - **bilbo's golden key:** two fixed seeds through `KeysRng`'s shape, with bilbo's password and identities.
  - A crate update that changes the protocol fails the first test, on a vector nobody in bilbo chose.
- **The rest of the crypto:** HKDF-SHA256, XChaCha20-Poly1305 and Ed25519 come through `identity/keys.rs`'s helpers, and SHA-256 through `shared::hash`, so each crate keeps its files, as AGENTS.md asks. `keys.rs` gains `hkdf(salt, input, info)`, HKDF-SHA256 to 32 bytes; today its only HKDF is inside `Owner::derive`.

Alternatives considered:
- **`spake2` 0.4.0.** Stable, but the duplicates above, for the same protocol.
- **CPace.** The only crate, `pake-cpace`, was last released in December 2023 (planning notebook, `tools-survey.md`).
- **`magic-wormhole` 0.8.** A whole working flow, but EUPL-1.2, a copyleft that sits badly in an Apache-2.0 binary, and it expects the wormhole project's mailbox server.
- **SPAKE2 written by hand on `curve25519-dalek`.** About 200 lines of the kind of code that should not be written by hand.
- **Neither crate is audited.** The README warns of it in both versions, so the pre-release label adds no gap the stable one lacks. The surface bilbo uses is small (start, finish), and swapping the crate later only changes `src/identity/pake.rs`.

### The code: a nameplate and three BIP39 words

- **Format:** `<nameplate>-<w1>-<w2>-<w3>`, for example `42-orbit-tunnel-velvet`. The nameplate is a random number from 1 to 999. The words are random from the BIP39 English list that `add-device-keys` embeds in `src/identity/phrase.rs`.
- **Entropy:** the nameplate is not secret, since anyone who can read the transport can list mailboxes. The words carry 3 × 11 = 33 bits.
- **One attempt per code:** `b.msg` is create-only, and A uses the code up after the first answer, whatever it holds. So an attacker gets one guess, at odds of 1 in 8.6 billion, and a wrong guess shows on A as a wrong code. The fingerprint check below stands behind that.
- **Typing:**
  - Case does not matter, spaces or hyphens separate the parts, and a word can be given by its first four or more letters, which extends the rule `bilbo device recover` uses (`phrase::lookup` takes a word or exactly its first four letters); `phrase.rs` gains the lookup by four or more, since the list's four-letter prefixes are unique.
  - A word not in the list is refused before the mailbox is read, so a typo that is not a word does not use the code up. Only a valid wrong word does.
- **Nameplates:**
  - A picks a random number and claims it by creating `a.msg`. When the object exists, it tries another, up to 20 times.
  - Then it stops with `no free pairing number at <url>; try again later`. That needs a folder holding hundreds of mailboxes, which in practice means someone is squatting it (see Risks).

Alternatives considered:
- **Two words (22 bits), or magic-wormhole's 16 bits.** Its prize is one file, while here it is the owner key. A third word costs a second of typing.
- **Four words.** Little gain over three plus the fingerprint, for a longer code.
- **Digits only (10 digits for 33 bits).** Harder to read aloud and to type without error, and the word list is already in the binary.
- **A nameplate allocated by a server.** There is no server on `file://`.

### Three messages, key confirmation before any secret

```
A: creates a.msg   {format, spake}                          (signed by A on the relay)
B: reads a.msg, finishes SPAKE2, prints the fingerprint, its name and id
B: creates b.msg   {format, spake, nonce, box(B's identity)} (the one unsigned message)
A: reads b.msg, finishes SPAKE2, opens the box               (fails: wrong code)
A: checks B's name, prints B's name, id, scopes and the fingerprint; the user types y
A: writes the new manifests, then creates c.msg {format, result, nonce, box(payload)}  (signed by A on the relay)
B: reads c.msg, fetches and checks the manifests, writes manifests, config, then keys
```

- **Keys:** SPAKE2 gives K. The transcript hash T is SHA-256 over `bilbo-pair-1`, the nameplate and both SPAKE2 messages. HKDF-SHA256 with salt T and input K gives three outputs, with info `b`, `c` and `fingerprint`. Each box is XChaCha20-Poly1305 with a random nonce. Its associated data is `bilbo-pair-1 <role>`, a newline, then T, where the role is `b` for `b.msg`'s box and `c <result>` for `c.msg`'s, so the plain `result` that picks the reader is under the AEAD.
- **`b.msg`'s box:** B's name, its Ed25519 and X25519 public keys, B's signature over T, which proves B holds the signing key it names, and, when B is enrolled, its owner's signing public key.
- **Wrong code:** A's key differs from B's, and B's box does not open. A then creates `c.msg` with the plain result `wrong-code`, and no secret.
- **`c.msg`:** its plain `result` is one of `enrolled`, `wrong-code`, `declined`, `expired`, `name-taken` or `other-owner`. Only `enrolled` and `other-owner` carry a box: `other-owner`'s holds A's owner signing public key, which is no secret, so B can print A's owner fingerprint while the folder does not learn which owner the mailbox belongs to.
  - On the relay, only A's key can create `c.msg`.
  - On a folder, whoever can write the folder could forge a plain result. That can only end a pairing, never change what B stores.
- **Why three messages, not two:** with two (B first, then A answers with the payload), A would send the owner key before learning whether the code was right. SPAKE2 keeps that ciphertext safe from an offline search, but the user would confirm on A without knowing whether B typed the code correctly, and nobody would claim the nameplate. A third message costs one more poll.
- **Encoding:** each message is JSON with its `format` first; the SPAKE2 messages in hex, the nonces and boxes in base64 (`base64`, already a dependency through segments), since hex would double a 12-scope `c.msg` to the 4 KiB limit.
- **Sizes:**
  - `a.msg` is about 100 bytes and `b.msg` about 400.
  - `c.msg` carries no manifest, so it stays near 200 bytes per scope, and `add-relay`'s 4 KiB limit holds 12 scopes with room to spare.
  - A refuses a 13th scope before creating a mailbox, naming `--scope`, so the limit is a rule the spec states, not a size check that fails late.

### The fingerprint, the confirmation, and the terminal rule

- **The fingerprint:** the first 8 bytes of the `fingerprint` output, read as a number modulo 10^12, are printed as twelve digits in groups of four, for example `4829 1307 5521`. The digits keep it from looking like `add-device-keys`' owner fingerprint (`xxxx-xxxx-…` in base32).
- **What it catches:** a transport operator who saw the code can run SPAKE2 with A and with B separately. That gives two keys, and so two fingerprints. The operator could grind its own messages until the two match, but at about 2^40 scalar multiplications per success inside the window, that takes a large machine. The check is a second line behind the code, not a substitute for it.
- **What B shows:** B prints the fingerprint with its own name and id, so everything A's question names can be compared on B's screen.
- **Confirmation on A only:** A is the device that gives something away. A stranger who raced the real device shows up on A as an unexpected name and id, while the real device reports `code … was already used`.
- **Names:** before asking, A checks B's name against A's own name and against every device that any of the owner's local manifests lists, not only the scopes being paired. `add-device-keys` makes names unique among all the owner's devices, and unions them into every new scope. A device already listed under the same id, as after an interrupted pairing, does not take its own name.
- **Terminals only on A:** A runs only when stdin and stderr are terminals and neither `CLAUDECODE` nor `CODEX_THREAD_ID` is set and not empty, the rule `add-device-keys` applies to the phrase.
  - The agent markers only stop an agent that runs `bilbo pair` by accident: `env -u CLAUDECODE -u CODEX_THREAD_ID` removes them.
  - The terminal requirement is the boundary this change relies on. An agent's tool calls get no terminal, so an agent cannot answer A's question in the normal course.
  - Neither rule stops a program that runs as the user and sets out to get around it. Such a program can read `owner.key` and `device.key` directly, which `add-device-keys` names as its own limit.
  - B needs no such rule, because B gives nothing away.
- **No `--yes` and no hidden way in:** no flag, variable or build mode lets A take an answer without a terminal. A debug-only hook would ship in every `cargo build` without `--release` and reopen this door, and `nix flake check` runs the tests in release, where the hook would be off.

### The transport URL on the new device is typed

- **Why typed:** B has no config and no identity, so it cannot look the URL up. Putting it in the code would make the code long, and for `file://` it would be wrong: the same Dropbox folder is `/Users/a/Dropbox/bilbo` on a Mac and `/home/a/Dropbox/bilbo` on Linux. A prints its own URL in the command to run, and the user corrects the path when it differs.
- **Mapping each scope:**
  - A scope whose URL on A is literally the URL A pairs over gets B's `--via` URL.
  - A scope on an `https://` relay keeps its URL, since a host name means the same thing everywhere.
  - A scope on another `file://` URL cannot be mapped, so A refuses to pair it in the same run.
  - URLs are compared as written, so `file:///srv/sync/` and `file:///srv/sync` differ, and the usage error names both.
  - The mapping touches only B's config. A manifest pins only `file://` for a folder, so a path that differs between the devices writes no manifest version, and both devices see the scope as valid.
  - A `--via` that no paired scope uses is a usage error too.
- **`https://` before `add-relay`:** there is no client yet, so a `--via https://…`, or a scope on a relay, is refused with `this bilbo cannot reach https:// transports yet`. `add-relay` lifts that refusal.

### What A sends, and what B checks

- **On A:** for each scope, A uses `manifest::add_device`, the operation `bilbo device recover` uses, given the epoch key A opens through its own `sealed` entry. It writes version n+1 on the latest local version, pending or not, as the `scope-manifest` spec's Pending epochs says. That version lists B, with the epoch key sealed to B's box key, and A writes it locally and to the transport, create-only. When the newest version already lists B's id (a pairing that stopped after this step), it writes nothing. Only then does it create `c.msg`, so no secret leaves A without a record of who received it.
- **An enrolled B:** when `b.msg` carries an owner key, A compares it with its own. Another owner ends the pairing with `other-owner`. The same owner gets the payload without the seed, since B holds it.
- **The payload:** A's name and id, and, for a B without keys, the owner signing seed as `owner.key` holds it (32 bytes), the only owner secret a device keeps. The owner's box public key, which `owner.key` also holds, B takes from the verified manifests' `owner_box`. B gets each scope's epoch key only as A sealed it to B's box key in the new manifest, and older epochs through that manifest's chain. Per scope the payload carries:
  - its name, id and `embedder` setting;
  - the newest manifest version and that version's SHA-256;
  - for an `https://` scope, its URL.

  Never the phrase, which no device stores.
- **On B:** B fetches versions 1 to n of each scope from the transport, skipping those already in its store. It checks them as a chain, backwards from the newest:
  - the newest version's hash matches what A sent over the authenticated channel;
  - each earlier version's hash matches its successor's `prev`, and version 1's `prev` is null;
  - the Ed25519 public key derived from the seed A sent equals every version's `owner`, and every signature verifies against it;
  - the newest version lists B's id and keys;
  - B's sealed epoch key opens.

  A missing or failing version refuses the whole pairing before anything is written. B then holds every version `manifest::verify_scope` walks. B fetches them through `sync::scopes::chain`. The cost is a few small files, read once.
- **Why fetch rather than carry the manifests:** a manifest grows with its device list and epoch chain, and can pass 4 KiB. The transport holds every version anyway, and the hash sent over the authenticated channel anchors the chain.
- **Scope selection is a boundary.**
  - B can open only the epoch keys A sealed to it, so it reads only the scopes paired. It cannot open a scope's owner copy, since no device holds the owner box secret.
  - Holding the signing seed does not let B add itself to another scope. A version under a new epoch needs a correct `chain` entry for the current one, and members check that entry against the key they hold.
  - What B can do with the seed is disrupt: sign versions that members reject, or that change the device list. `add-device-keys` states the guarantee, and this change repeats it: "After a confirmed revocation a revoked device, even one using the owner signing seed, cannot read anything written under later epochs; it can still disrupt by signing versions that members reject or that change the device list, which watch announces."
  - The same holds for a device paired into some scopes: it reads only epochs sealed to it, of the scopes it was paired into.

### Where B writes, and in what order

0. **Before the exchange**, a B without keys loads or creates its pending device key in `<state>/bilbo/pair/device.key` (folder 0700, file 0600, created with `create_new`), and uses it for this run and any retry. An enrolled B answers with the device key in `keys/`, and skips step 4.
   - It sits outside `keys/`, so `add-device-keys`' rule that a `keys` folder holding one file is damaged stands.
   - B's checks for an enrolled device look only at `keys/`.
1. **Verify everything:** the checks above, and that any manifest already in B's store has the same owner. A store of another owner is refused, naming both fingerprints.
2. **Write the manifests** under `<root>/.bilbo/scopes/<scope_id>/`, through `manifest::adopt` under `manifest::lock`, which writes each version as a confirmed copy, with no `.pending` marker.
3. **Rewrite the config.**
4. **Write the keys:** through `keys::write_identity`, which under `<state>/bilbo/keys.lock` removes a leftover `keys.new/` as `init` and `recover` do, writes `owner.key` and a copy of the pending device key into `keys.new/`, and moves that folder to `keys` with `host::swap::rename_new`. This is `add-device-keys`' writer, so the identity appears whole. Then B removes `<state>/bilbo/pair/`. `add-device-keys`' Writing keys requirement gives that duty to every verb that writes an identity under `keys.lock`, so it covers `pair`.

- **Holding keys is what makes a device enrolled**, so a crash or a refusal before step 4 leaves B unenrolled.
  - The next `bilbo pair` reuses the pending key and so answers with the same id.
  - A finds that id already listed and writes no new version.
  - B accepts manifests of the same owner that it already holds.
  - No ghost device is left in the manifests.
- **The config edit:** `src/shared/config.rs` gains `set_keys(path, &[(key, value)])`. It replaces a key's line in place or appends it, keeps every other line and comment as written, writes each value through `config::quote`, and writes through `write_config`, which moves from `src/setup/apply.rs` to `src/shared/config.rs` unchanged, `.bak` included. A file that does not exist yet is created, with no `.bak`.
- **Which keys B sets:**
  - `scope.<name>.sync`;
  - `scope.<name>.embedder = local` when either device says `local`. A stricter choice on B stays: remote embedders are where note text leaves the device, so pairing never loosens it.
- **Not copied:** `paths`, because folders differ by machine, and `marks` and `scope.default`, which are per-device habits. Leaving them out also keeps `c.msg` small.
- **Notes that already carry the scope:** before its stdout lines, B prints `<n> notes already carry scope: <name> and sync from now on` for each paired scope that some of its notes already name.
  - Such notes sat under `sync = off`, and a mark never holds a push back, so they upload within one cycle.
  - The line comes after A's confirmation, because B learns the scope names only from the encrypted reply.
  - The user can still move such a note out with `bilbo scope set` before it syncs.

### Timing

- **A:** the code is good for 10 minutes, from showing it to creating `c.msg`.
  - The question is a blocking read of one line. When the answer comes after the window, A creates `c.msg` with `expired` and sends no secret.
  - A never answers for a user who has walked away: it simply waits for the line or for Ctrl-C.
- **B:**
  - It waits up to 2 minutes for `a.msg` to appear, which allows for a synced folder delivering it.
  - It waits up to 10 minutes from its own start for `c.msg`.
  - After `c.msg` it has its own 2 minutes for the manifest versions to arrive. A user who confirms at minute 9 does not leave the folder only a minute.
- **Polling:** every 250 ms on `file://`, every 2 s on `https://`. B's unsigned polling is 30 requests a minute, under the relay's 60.
- **Clocks:** each side measures time on its own monotonic clock. Only the 30-minute sweep reads a file time, and it compares that with the clock of the machine that sees the file.

### The mailbox on each transport

- **Layout:** `pair/<nameplate>/a.msg`, `b.msg` and `c.msg`, within the layout and removal rule that add-sync's `sync-transport` spec owns. This change adds no transport spec of its own. The `Transport` trait gains one method, `sweep_mailboxes(now, age)`, which removes on `file://` each `pair/<nameplate>/` whose `a.msg` was last modified more than `age` before `now`, and does nothing on `https://`; the trait has no listing of `pair/`, and the age is a file time only the folder knows.
- **`file://`:**
  - B removes the mailbox after reading any result but `wrong-code`. After a wrong code it leaves the mailbox in place, so a retry of the right code finds `b.msg` and says `code … was already used` at once, instead of waiting 2 minutes and reporting a missing mailbox.
  - A removes an unanswered mailbox when its code expires.
  - Every `bilbo pair` removes mailboxes whose `a.msg` was last modified more than 30 minutes ago. That covers a killed side and a mailbox left after a wrong code.
- **`https://`:** `remove_mailbox` does nothing, and the relay deletes a nameplate 30 minutes after its first message. That is well past the 10-minute window plus B's reading of `c.msg`, and it matches the folder sweep.
- **What pairing relies on from the relay (`add-relay`), all in its current draft:**
  - `a.msg` signed by a listed device;
  - `b.msg` as the nameplate's one unsigned message;
  - `c.msg` signed by `a.msg`'s key, so a stranger gets at most one write per nameplate and cannot create `c.msg` before A does;
  - at most 4 KiB per message, 3 of the 8 messages a nameplate may hold, one of the 32 open nameplates, and 30 minutes of life;
  - unsigned requests under 60 a minute, with 4 distinct nameplates in 10 minutes. That leaves room for a typo in the nameplate.

### The verb's shape, and how it is tested

- **The signature:** `identity::pair::run(args, env, terminal: bool, answer: &mut dyn BufRead, limits: &Limits, out: &mut dyn FnMut(&str), err: &mut dyn FnMut(&str)) -> Result<(), Failure>`, the shape of `device::run` with the line printers `setup` and `watch` take.
  - `main` passes `stdin().is_terminal() && stderr().is_terminal()`, the locked stdin, `Limits::default()`, and two line printers, stdout and stderr with the `bilbo: ` prefix.
  - `Limits` holds the window (10 minutes), the appear and manifest waits (2 minutes each), the two poll intervals and the sweep age (30 minutes).
  - No environment variable or build mode changes any of them.
- **Order of A's checks:** usage and config errors first, then no owner key, no syncing scope and the scope and `--via` rules, then the terminal rule, all before the sweep and the mailbox. So the binary tests reach each refusal without a terminal.
- **Streams:** stdout carries only the result: `paired …` on A; `paired with …` and the line saying `bilbo watch` starts syncing within one cycle on B. The code, the instructions, the fingerprint and the question go to stderr. So `bilbo pair > out` hides nothing from the user, and a failed pairing leaves stdout empty, as the `cli` spec's output streams ask.
- **Library and verb:** `src/identity/pake.rs` is a library module with no I/O. It covers parsing and canonicalizing a code, `KeysRng`, the three message formats, the key schedule, the boxes and the fingerprint. The verb is the folder `src/identity/pair/`: `mod.rs` (arguments, the checks before the mailbox, `Limits`, polling), `show.rs` (A), `join.rs` (B) and the test-only `exchange.rs`.
- **Why `identity`:** pairing enrolls a device, writing keys and manifest versions as `bilbo device` does, and `device-pairing` is that capability's sibling. A verb may use any domain, so the verb reaches `sync::transport` and `sync::scopes` as `identity/device.rs` does, while `pake`, a library module, uses only `keys`, `phrase` and `shared::hash`, so `identity`'s library code still never uses `sync`. `sync/pair/` would also pass the cycle check, but `sync` is the replication engine `watch` drives, and the verb would then write identities from it. A folder, because A, B and the one-process exchange tests are each several hundred lines.
- **Usage:** `bilbo pair [--scope <name>]... [--via <url>]` shows a code, and `bilbo pair <code> --via <url> [--name <name>]` joins. A first argument that does not start with `-` is a code.
- **The exchange is tested in one process**, in `src/identity/pair/exchange.rs`, a test-only module of the verb:
  - two threads run A and B with two `Env`s;
  - A gets `terminal = true` and a scripted answer;
  - both get millisecond `Limits`.
- **Each side has its own folder.** A copier thread moves each new file across after a set delay, imitating the cloud tool. That exercises the path mapping, the late manifest and the appear wait for real.
- **The copier also checks the mailbox:** it sees every mailbox file as it is created, so the opaque-mailbox scenario searches those bytes before B removes them.
- **`tests/pair.rs` tests through the built binary only what needs no terminal and no waiting:**
  - A's refusals, including the terminal, `CLAUDECODE` and `CODEX_THREAD_ID` rule;
  - B's refusals before the mailbox;
  - usage errors;
  - config errors;
  - `Pair is a verb`.
- **The whole binary on two machines with real terminals** is the smoke test.

## Risks / Trade-offs

- [`spake2` 0.5.0-pre.0 is a pre-release, and neither version is audited] → The protocol code is 0.4.0's, `Cargo.lock` pins it, the crate's own vector and bilbo's golden key catch a protocol change, and the surface is two calls.
- [Every paired device holds the owner signing seed] → "After a confirmed revocation a revoked device, even one using the owner signing seed, cannot read anything written under later epochs; it can still disrupt by signing versions that members reject or that change the device list, which watch announces." A stolen paired device reads only the scopes it was paired into. Pairing does not widen this: a device enrolled with the phrase holds the same seed.
- [The user confirms without comparing the fingerprint] → The code still allows one guess at 1 in 2^33. The fingerprint only matters when someone already has the code.
- [Anyone who can write the shared folder can stop pairing] → The cloud vendor or anyone the folder is shared with could:
  - squat all 999 nameplates;
  - create `b.msg` under each new `a.msg` to burn every code;
  - forge a plain `c.msg` result;
  - delete a mailbox.

  None of it leaks a secret, and the same writer can already delete segments, so the exposure is not new. A full folder ends in the `no free pairing number` message, and a burned code shows on A as a wrong code.
- [A cloud folder is not truly create-only across machines] → Two new devices answering one code within seconds could both create `b.msg` locally, and the cloud tool keeps a conflict copy. A reads only `b.msg`, and the other device's reply cannot open its box, so that device reports a failed pairing. Likewise, two A's on two machines that pick one nameplate within seconds leave a conflict copy of `a.msg`. The A whose copy lost waits out its window and reports the code expired.
- [A stops between publishing the manifest and creating `c.msg`] → B is listed but holds nothing. Pairing again, with B's pending key, finishes it without a new version. Otherwise `bilbo device revoke` removes it.
- [A pending key that is never used] → `<state>/bilbo/pair/device.key` stays until a pairing succeeds. It is a device key pair with no owner secret, in a 0600 file.
- [A killed with Ctrl-C leaves a mailbox] → The relay drops it after 30 minutes, and the next `bilbo pair` on the folder sweeps it after 30.

## Migration Plan

Nothing to migrate. Devices enrolled through `bilbo device recover` and through `bilbo pair` hold the same files. Rolling back removes the verb. Paired devices stay enrolled, because their keys and manifests are `add-device-keys`' formats.
