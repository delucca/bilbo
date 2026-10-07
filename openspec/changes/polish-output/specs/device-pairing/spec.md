## MODIFIED Requirements

### Requirement: Show a pairing code
`bilbo pair [--scope <name>]... [--via <url>]` on an enrolled device SHALL create a mailbox on the transport and show on stderr first the command to run on the new device, `bilbo pair <code> --via <url>`, then the code `<nameplate>-<word>-<word>-<word>`, with a random nameplate from 1 to 999 and three random words from the BIP39 English list, and that the code works once for 10 minutes: all in a box titled `On the new device, run` when the command fits in it on one line, else the command on one line above a box titled `Pairing code`. It SHALL then wait for the new device. stdout SHALL carry only the final `paired` line.

#### Scenario: A code is shown
- **WHEN** a device enrolled with scope `personal` syncing through `file:///srv/sync` runs `bilbo pair`
- **THEN** stderr's box holds the line `bilbo pair <code> --via file:///srv/sync` and, below it, `The code is <code>. It works once, for 10 minutes.`, where `<code>` matches `^[1-9][0-9]{0,2}(-[a-z]+){3}$` with every word in the BIP39 English list; stdout is empty; and `/srv/sync/pair/<nameplate>/a.msg` exists

#### Scenario: A long command is never broken
- **WHEN** a user runs `bilbo pair` in a terminal 60 columns wide and the command is 70 columns long
- **THEN** stderr holds the whole command on one line with no box border in it, and below it a box titled `Pairing code` holding `The code is <code>. It works once, for 10 minutes.`

#### Scenario: Two pairings at once
- **WHEN** a second `bilbo pair` runs while the first still waits on the same transport
- **THEN** the two codes have different nameplates

#### Scenario: No free nameplate
- **WHEN** A picks 20 nameplates in a row whose mailboxes already exist
- **THEN** stderr says `no free pairing number at <url>; try again later`, the exit code is 1, and no mailbox is created

#### Scenario: A stray argument
- **WHEN** a user runs `bilbo pair --scope`
- **THEN** bilbo prints a usage message to stderr, exits 2, and creates no mailbox

### Requirement: Confirm the fingerprint
Once the new device has answered, both devices SHALL show the same fingerprint, twelve digits in three groups of four, derived from the session, and B SHALL show its own name and device id with it. A SHALL name the new device, its device id and the scopes it will join, then ask `Fingerprint <F>: does <B's name> show the same?` with the answers yes and no, no chosen at first, and enroll the new device only when the answer is yes, given before the code expires. Otherwise, on no or a cancelled question, A SHALL send no secret, and both devices SHALL exit 1.

#### Scenario: Matching fingerprints
- **WHEN** B answers A's code with B's stderr piped
- **THEN** A's stderr names B's name and id and holds the question with `Fingerprint <F>`, and B's stderr holds `bilbo: fingerprint <F> for <B's name> <B's id>; confirm on the device that showed the code`, with the same `<F>`

#### Scenario: Declined
- **WHEN** the user answers no on A
- **THEN** A says `not confirmed; nothing was sent`, B says `the other device declined; nothing was received`, both exit 1, no manifest changes, and B holds no keys

#### Scenario: End of input
- **WHEN** A's input ends at the question, as when its terminal closes, before an answer
- **THEN** A says `not confirmed; nothing was sent`, and both devices exit 1

#### Scenario: The question is cancelled
- **WHEN** the user presses Esc or Ctrl-C at A's question
- **THEN** A says `not confirmed; nothing was sent`, B says `the other device declined; nothing was received`, and both devices exit 1

#### Scenario: Enter alone declines
- **WHEN** the user presses Enter at A's question without moving to yes
- **THEN** A says `not confirmed; nothing was sent` and sends no secret

### Requirement: Expiry
A SHALL send its reply within 10 minutes of showing the code; past that it SHALL print `the code expired; nothing was sent`, send no secret and exit 1, removing an unanswered mailbox. B SHALL wait up to 2 minutes for the mailbox to appear and up to 10 minutes from its start for A's reply, then exit 1 with `no answer from the other device`. After the reply, B SHALL wait up to 2 more minutes for the manifests to reach it.

#### Scenario: Nobody answers
- **WHEN** A shows a code and no device answers within 10 minutes
- **THEN** A exits 1 with the expiry message and `pair/<nameplate>/` is gone from the folder

#### Scenario: Too late
- **WHEN** B types the code after A has expired it
- **THEN** stderr says `no pairing <nameplate> at <url>` and the exit code is 1

#### Scenario: Confirmed too late
- **WHEN** B answered in time but the user answers yes on A after the 10 minutes
- **THEN** A says `the code expired; nothing was sent`, no manifest changes, and both exit 1

#### Scenario: A late manifest
- **WHEN** the user confirms at minute 9 and the synced folder delivers A's new manifest to B 90 seconds after A's reply
- **THEN** pairing succeeds

#### Scenario: A manifest that never arrives
- **WHEN** A's new manifest has not reached B 2 minutes after A's reply
- **THEN** B says `the personal manifest did not reach <url> in time; run bilbo pair again`, writes nothing, and exits 1

## ADDED Requirements

### Requirement: Showing a code on a terminal
The showing device SHALL draw with the setup wizard's prompts on stderr: it SHALL open with `bilbo pair`, show the code's box, wait under a spinner `Waiting for the new device` that is erased when the wait ends, name the new device with `<B's name> <B's id> asks to join <scopes>`, ask the question of Confirm the fingerprint, and close with `Paired` once the reply is sent. A refusal after the box SHALL close the drawing with `Not paired` before its message.

#### Scenario: The flow on A
- **WHEN** a user pairs `rhosgobel` into `personal` from A and answers yes
- **THEN** A's stderr shows, in order, `bilbo pair`, the code's box, `rhosgobel <its id> asks to join personal`, the question and `Paired`, no `Waiting for the new device` line is left on the screen, and stdout is `paired rhosgobel <its id>: personal`

#### Scenario: Declined on A
- **WHEN** the user answers no on A
- **THEN** A's drawing closes with `Not paired`, and stderr then holds `not confirmed; nothing was sent` after its error mark

### Requirement: Joining on a terminal
When stdin and stderr are terminals and no agent marker of the `device-identity` spec's Terminal-only forms is set and not empty, the joining device SHALL draw with the setup wizard's prompts on stderr: it SHALL open with `bilbo pair`, wait under a spinner `Looking for pairing <nameplate>`, show the fingerprint, its name and device id in a box titled `Fingerprint`, wait under a spinner `Waiting for the other device to confirm`, then under `Fetching the scopes`, and close with `Paired with <A's name>`. Each spinner SHALL be erased when its wait ends. Otherwise it SHALL print its lines as the other requirements state.

#### Scenario: The flow on B
- **WHEN** a user runs `bilbo pair 42-orbit-tunnel-velvet --via file:///srv/sync` on B in a terminal and confirms on A
- **THEN** B's stderr shows `bilbo pair`, the `Fingerprint` box holding the same fingerprint as A's question and B's name and id, and `Paired with <A's name>`, no spinner line is left on the screen, and stdout is `paired with <A's name>: personal` and `bilbo watch starts syncing them within one cycle`

#### Scenario: An agent joins
- **WHEN** `bilbo pair 42-orbit-tunnel-velvet --via file:///srv/sync` runs with stdin and stderr terminals and `CODEX_CI=1`
- **THEN** B draws no box or spinner, and its stderr holds `bilbo: fingerprint <F> for <B's name> <B's id>; confirm on the device that showed the code`

#### Scenario: A refusal on B
- **WHEN** B in a terminal is told the other device declined
- **THEN** B's drawing closes with `Not paired`, then stderr holds `the other device declined; nothing was received` after its error mark, and the exit code is 1
