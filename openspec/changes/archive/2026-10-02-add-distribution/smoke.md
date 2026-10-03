# Smoke test of the v0.1.0 release

Run on 2026-10-02 after PR #4 was rebase-merged. The signed tag `v0.1.0` points at `9207670`, the tip of `main`. Release run 37079408905 passed all eight jobs: plan, four native builds, the global artifacts, host and announce. Temp folders are shown as `<tmp>`, and each installer run uses `env -i` with a fresh `HOME`.

What it shows:

- **Assets.** The release holds four archives, four `.sha256` files, `bilbo-installer.sh`, `sha256.sum` and `dist-manifest.json`. It holds no Windows asset and no source tarball.
- **Checksums.** Every archive matches its `.sha256` and `sha256.sum`. shasum warns about one badly formatted line in each file. That line is a trailing blank line in dist's output and does not affect the check.
- **glibc floor.** The published installer requires glibc 2.35 (`check_glibc "2" "35"`), the ubuntu-22.04 floor. The Linux binaries need at most `GLIBC_2.34`.
- **macOS arm64 install.** The installer puts `bilbo` in `$HOME/.local/bin`, or in `$XDG_BIN_HOME` when that is set, and never in `~/.cargo/bin`. `--version` prints `bilbo 0.1.0`. A rerun succeeds. A `uname` shim reporting FreeBSD exits 1 with nothing written.
- **Checksum verification.** The installer checks the archive's sha256 when `sha256sum` exists, which is the case on Linux. Stock macOS has no `sha256sum`, so there it skips the check and says so. The proposal's "installer does not verify" holds only for macOS.
- **Linux x86_64 install, on gondolin.** The paths are the same as on macOS. gondolin runs NixOS, whose `/lib64` loader is a stub, so the binary was run through nixpkgs' glibc loader. The install itself is the generic-Linux path.
- **Nix.** `nix run github:delucca/bilbo/v0.1.0 -- --version` prints `bilbo 0.1.0`, and the package holds `share/bilbo` with the plugin and both marketplaces.

## Assets and checksums

```
$ gh release download v0.1.0 -R delucca/bilbo
bilbo-aarch64-apple-darwin.tar.xz
bilbo-aarch64-apple-darwin.tar.xz.sha256
bilbo-aarch64-unknown-linux-gnu.tar.xz
bilbo-aarch64-unknown-linux-gnu.tar.xz.sha256
bilbo-installer.sh
bilbo-x86_64-apple-darwin.tar.xz
bilbo-x86_64-apple-darwin.tar.xz.sha256
bilbo-x86_64-unknown-linux-gnu.tar.xz
bilbo-x86_64-unknown-linux-gnu.tar.xz.sha256
dist-manifest.json
sha256.sum

$ for f in *.tar.xz; do shasum -a 256 -c "$f.sha256"; done
bilbo-aarch64-apple-darwin.tar.xz: OK
shasum: WARNING: 1 line is improperly formatted
bilbo-aarch64-unknown-linux-gnu.tar.xz: OK
shasum: WARNING: 1 line is improperly formatted
bilbo-x86_64-apple-darwin.tar.xz: OK
shasum: WARNING: 1 line is improperly formatted
bilbo-x86_64-unknown-linux-gnu.tar.xz: OK
shasum: WARNING: 1 line is improperly formatted

$ shasum -a 256 -c sha256.sum
bilbo-aarch64-apple-darwin.tar.xz: OK
bilbo-aarch64-unknown-linux-gnu.tar.xz: OK
bilbo-x86_64-apple-darwin.tar.xz: OK
bilbo-x86_64-unknown-linux-gnu.tar.xz: OK
shasum: WARNING: 1 line is improperly formatted

$ rg -n "check_glibc \"" bilbo-installer.sh
484:            if ! check_glibc "2" "35"; then
501:            if ! check_glibc "2" "35"; then

$ tar -xOf bilbo-x86_64-unknown-linux-gnu.tar.xz --wildcards "*/bilbo" | strings | rg -o "GLIBC_2\.[0-9]+" | sort -uV | tail -1
GLIBC_2.34

$ tar -xOf bilbo-aarch64-unknown-linux-gnu.tar.xz ... (same)
GLIBC_2.34
```

## macOS arm64

```
$ uname -sm; sw_vers -productVersion
Darwin arm64
26.6.2

# default path, XDG_BIN_HOME unset
$ env -i HOME=<tmp>/mac/h PATH=/usr/bin:/bin sh -c 'curl --proto =https --tlsv1.2 -LsSf https://github.com/delucca/bilbo/releases/download/v0.1.0/bilbo-installer.sh | sh'
downloading bilbo 0.1.0 aarch64-apple-darwin
skipping sha256 checksum verification (it requires the 'sha256sum' command)
installing to <tmp>/mac/h/.local/bin
  bilbo
everything's installed!

To add $HOME/.local/bin to your PATH, either restart your shell or run:

    source $HOME/.config/bilbo/env.sh (sh, bash, zsh)
    source $HOME/.config/bilbo/env.fish (fish)
exit=0
$ mac/h/.local/bin/bilbo --version
bilbo 0.1.0
$ ls mac/h/.cargo
ls: mac/h/.cargo: No such file or directory

# XDG_BIN_HOME set
$ env -i HOME=<tmp>/mac/h2 XDG_BIN_HOME=<tmp>/mac/b PATH=/usr/bin:/bin sh -c 'curl ... | sh'
downloading bilbo 0.1.0 aarch64-apple-darwin
skipping sha256 checksum verification (it requires the 'sha256sum' command)
installing to <tmp>/mac/b
  bilbo
everything's installed!

To add <tmp>/mac/b to your PATH, either restart your shell or run:

    source $HOME/.config/bilbo/env.sh (sh, bash, zsh)
    source $HOME/.config/bilbo/env.fish (fish)
exit=0
$ mac/b/bilbo --version
bilbo 0.1.0
$ ls mac/h2/.local/bin mac/h2/.cargo/bin
ls: mac/h2/.cargo/bin: No such file or directory
ls: mac/h2/.local/bin: No such file or directory

# rerun over the default install
exit=0
bilbo 0.1.0

# unsupported platform: uname shim reporting FreeBSD amd64
ERROR: there isn't a download for your platform x86_64-unknown-freebsd
exit=1
$ find mac/h3 -name bilbo
```

## Linux x86_64 (gondolin)

```
$ uname -sm; ldd --version | head -1
Linux x86_64
ldd (GNU libc) 2.42

# default path, XDG_BIN_HOME unset
$ env -i HOME=$T/h PATH=/run/current-system/sw/bin sh -c 'curl --proto =https --tlsv1.2 -LsSf https://github.com/delucca/bilbo/releases/download/v0.1.0/bilbo-installer.sh | sh'
downloading bilbo 0.1.0 x86_64-unknown-linux-gnu
installing to $T/h/.local/bin
  bilbo
everything's installed!

To add $HOME/.local/bin to your PATH, either restart your shell or run:

    source $HOME/.config/bilbo/env.sh (sh, bash, zsh)
    source $HOME/.config/bilbo/env.fish (fish)
exit=0
$ ls -l $T/h/.local/bin; ls $T/h/.cargo
total 4180
-rwxr-xr-x 1 delucca users 4276984 Oct  2 23:51 bilbo
ls: cannot access '$T/h/.cargo': No such file or directory
# gondolin is NixOS (stub-ld), so run the generic glibc binary through nixpkgs' loader
$ $GLIBC/lib/ld-linux-x86-64.so.2 $T/h/.local/bin/bilbo --version
bilbo 0.1.0

# XDG_BIN_HOME set
downloading bilbo 0.1.0 x86_64-unknown-linux-gnu
installing to $T/b
  bilbo
everything's installed!

To add $T/b to your PATH, either restart your shell or run:

    source $HOME/.config/bilbo/env.sh (sh, bash, zsh)
    source $HOME/.config/bilbo/env.fish (fish)
exit=0
$ ls $T/b $T/h2/.local/bin $T/h2/.cargo/bin
ls: cannot access '$T/h2/.local/bin': No such file or directory
ls: cannot access '$T/h2/.cargo/bin': No such file or directory
$T/b:
bilbo
bilbo 0.1.0
```

## Nix

```
$ nix run github:delucca/bilbo/v0.1.0 -- --version
bilbo 0.1.0
$ nix build --no-link --print-out-paths github:delucca/bilbo/v0.1.0 && ls <out>/share/bilbo -A
.agents
.claude-plugin
plugins
```
