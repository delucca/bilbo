---
id: 01JSZ19DS1CJRR28KGGY5X4TWX
created: 2025-04-28T17:26-03:00
sources:
  - "doc: Vault plugin install notes"
---

# keyvane-vault-plugin fails to start with permission denied

Vault refuses to launch keyvane-vault-plugin when the binary is not executable by the user Vault runs as. The error looks like this: `fork/exec /etc/vault/plugins/keyvane-vault-plugin: permission denied`. It reads like a missing file or a policy problem, but it is almost always a file mode or ownership issue on the plugin binary or on a directory above it. Check that first before touching Vault policy or the plugin registration.

## Symptom

Registering or enabling the secrets engine fails, or Vault logs the error above when it tries to start the plugin process. The mount never comes up. Other plugins in the same directory may work fine, which is what makes it confusing: the failure is specific to this one file.

The message comes from the Go exec call Vault makes when it spawns the plugin as a child process. "permission denied" there means the kernel refused to execute the file for the Vault user. It does not mean the plugin rejected anything, and it does not mean the plugin ever started. So there are no plugin logs to look at; the process never ran.

## Cause

The binary is not executable by the vault user. Typical ways this happens:

- The file was copied or downloaded without the execute bit set.
- The file is owned by root or a deploy user and the mode gives the vault user no execute permission.
- A directory in the path lacks search (execute) permission for the vault user.
- The artifact was unpacked from an archive or built in CI that dropped the mode on upload.
- A mount option on the filesystem (noexec) blocks execution, even when the mode looks right.

The first three are by far the most common. The noexec case is rare but shows up when the plugin directory sits on a separate volume.

## How to confirm

Look at the mode and owner of the binary and of each parent directory, as the vault user would see them. Listing the file with long format is enough to see the mode bits and owner. Then try running the file as the vault user, for example through sudo to that user. If that also says permission denied, the problem is on the filesystem side and not in Vault.

Also check that the file is a real binary for the host platform. A wrong-architecture binary gives a different error (exec format error), not permission denied, so if you see permission denied the mode or mount is the suspect.

## Fix

Make the binary executable by the vault user. Either set the owner to the vault user or set the mode so that group or other can execute, whichever matches how the rest of the plugin directory is managed. Make sure every parent directory can be searched by the vault user. If the volume is mounted noexec, move the plugin directory to a volume that allows execution rather than remounting a shared one.

After fixing the file, no Vault restart is needed in most cases, but retry the registration or the mount enable. If the mount was already marked as failed, disable and re-enable it, or restart the Vault node if that is how your setup recovers plugins.

## Things that are not the problem

- The plugin SHA256 registered in Vault. A checksum mismatch gives its own error that names the checksum.
- The plugin_directory setting. A wrong directory gives a not-found style error, not permission denied.
- mTLS or SPIFFE identity configuration. Those matter after the plugin starts, and here it never does.
- etcd availability. The plugin talks to etcd only once it is running.

## Prevention

Set the mode and owner in the build or packaging step instead of relying on a manual chmod on the host. If deployment is done by config management, declare the file mode and owner explicitly for the plugin binary and its directory. Add a smoke check after deploy that runs the binary as the vault user, or at least verifies the execute bit, before Vault is asked to register it.

When the binary is replaced during an upgrade, the new file does not inherit the old file's mode if it is written to a new inode. This is a common way for a working setup to break on the next release. Check the mode after every upgrade, and register the new checksum as usual.

## Notes for later

If this error shows up in a container, check the user the Vault process runs as inside the container and whether the image copies the binary with the right mode. Copying with a build tool that preserves the mode from a CI cache can bring back a non-executable file even after a fix on the host.

If the same message appears after the mode looks correct, suspect SELinux or AppArmor policy on the plugin directory. Those also produce permission denied from the exec call, and the audit log of the security module will name the denial.
