# bilbo

## Testing

- Read `docs/testing.md` before adding or moving a test.
- `nix develop -c bazel test //...` runs every layer except tests that need the screen (`desktop`) and release.
- `--config=<layer>` runs one layer (`unit`, `contract`, `acceptance`, `integration`, `snapshot`, `e2e`, `release`); `--config=desktop` runs only screen tests; `--config=all` runs everything. Desktop tests take focus and keyboard input.
