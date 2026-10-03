# Spec Delta

## MODIFIED Requirements

### Requirement: Codex hook trust
When the codex step leaves `bilbo@bilbo` installed (`installed`, `updated` or `kept`), `setup` SHALL ask Codex, through `codex app-server`, which hooks the bilbo plugin registers and whether Codex trusts them, and SHALL mark every untrusted or changed bilbo hook trusted through Codex's own config writer, never by editing Codex's files itself. The hook line SHALL say `installed: trusted in Codex` when it trusted a hook Codex had never trusted, `updated: trusted in Codex` when the hook had changed since it was trusted, `kept: trusted in Codex` when every bilbo hook was already trusted, `skipped: no codex plugin` when the codex step did not leave the plugin installed, `skipped: codex lists no bilbo hook` when Codex lists none, `failed: <message>` when Codex cannot be asked or cannot write its config, and `failed: codex reports trust status '<status>' for a bilbo hook` when Codex lists a bilbo hook with a status other than `untrusted`, `modified`, `trusted` or `managed`, in which case `setup` SHALL write no trust. The wizard's summary SHALL name the trust whenever it installs the Codex plugin. `--remove` SHALL delete, through the same writer, every trust entry whose hook key belongs to `bilbo@bilbo`, and its hook line SHALL say `removed`, `skipped: not trusted`, `skipped: not found` when there is no `codex`, or `failed: <message>`.

#### Scenario: A fresh install trusts the hook
- **WHEN** `codex` is on PATH with no `bilbo` marketplace and a user runs `bilbo setup --yes`
- **THEN** the codex line says `installed`, the hook line says `installed: trusted in Codex`, and Codex runs the digest hook and the compaction hook without asking for a review

#### Scenario: A rerun keeps the trust
- **WHEN** setup already trusted the hook and the user runs `bilbo setup --yes` again
- **THEN** the hook line says `kept: trusted in Codex` and Codex's config is not written

#### Scenario: A changed hook is trusted again
- **WHEN** a new bilbo release changes the hook's command, so Codex lists it as changed since it was trusted
- **THEN** setup writes the new trust and the hook line says `updated: trusted in Codex`

#### Scenario: A release that adds a hook
- **WHEN** setup trusted the digest hook for an earlier release, and the installed release adds the compaction hook, which Codex lists as untrusted
- **THEN** setup writes trust for the compaction hook only, leaves the digest hook's trust as it was, and the hook line says `installed: trusted in Codex`

#### Scenario: An unknown trust status is not trusted
- **WHEN** Codex lists a bilbo hook with a trust status `setup` does not know, such as `blocked`
- **THEN** setup writes no trust, the hook line says `failed: codex reports trust status 'blocked' for a bilbo hook`, and the exit code is 1

#### Scenario: No Codex plugin, no trust
- **WHEN** a user runs `bilbo setup --yes --no-plugin`, or unticks Codex in the wizard
- **THEN** the hook line says `skipped: no codex plugin` and `codex app-server` does not run

#### Scenario: Codex cannot write its config
- **WHEN** Codex's `config.toml` is a link into a read-only folder and setup installs the plugin
- **THEN** the hook line says `failed` with Codex's message, the steps after it still run, and the exit code is 1

#### Scenario: Removing the trust
- **WHEN** setup trusted the hook and the user runs `bilbo setup --remove --yes`
- **THEN** Codex's config holds no trust entry for a `bilbo@bilbo` hook and the hook line says `removed`
