# Tasks

## 1. Pages

- [x] 1.1 Add `pub const HELP` with its page to each of the 16 verb files listed in design.md, above `#[cfg(test)] mod tests`: `nix develop -c cargo build --locked && nix develop -c cargo test --locked --test layout`

## 2. Help and usage rendering

- [x] 2.1 In `src/main.rs`, replace `USAGE` with `OVERVIEW` and `PAGES`, add `help_topic`, `page`, `unknown_verb`, `suggestion`, `distance`, `print_help`, `styled`, `bold`, `heading_end`, `forms` and `usage_lines`, pass the arguments to `report`, and add the unit tests for widths, page shape, forms, usage lines, suggestions and styling: `nix develop -c cargo test --locked --bin bilbo tests::`
- [x] 2.2 Rewrite the help and usage tests of `tests/cli.rs` and add the drift guards (dispatch against the overview, parser options on pages, the commands reference sections, no escape byte), and add `./docs` to the `lib.fileset` in `flake.nix`: `nix develop -c cargo test --locked --test cli`
- [x] 2.3 Move `tests/recall.rs`, `tests/library.rs` and `tests/setup.rs` off the old usage text: `nix develop -c cargo test --locked --test recall --test library --test setup`

## 3. Docs

- [x] 3.1 Update `docs/reference/commands.md` (intro, help paragraph, the `history`, `library`, `relay` and `setup` synopses), `README.md` and the new-verb rule in `AGENTS.md`: `python3 .github/wiki.py docs "$(mktemp -d)" https://example.com/repo && nix develop -c cargo test --locked --test cli every_page_links`

## 4. Verify

- [x] 4.1 Run the full verification: `nix develop -c cargo fmt --check && nix develop -c cargo clippy --locked --all-targets -- -D warnings && nix develop -c cargo test --locked && nix flake check -L`
