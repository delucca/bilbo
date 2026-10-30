# bilbo

## Testing

- Read `docs/testing.md` before adding or moving a test.
- `nix develop -c bazel test //...` runs every layer except e2e and release.
- `--config=<layer>` runs one layer (`unit`, `contract`, `acceptance`, `integration`, `snapshot`, `e2e`, `release`); `--config=all` runs every layer. e2e drives the real desktop.
