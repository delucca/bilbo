# Proposal

## Why

After `add-library-store`, a source can only come from a file another tool wrote, and every such source is labelled `capture: external`, because bilbo cannot vouch for its text. Today's notebook `ingest` skill fetches through WebFetch, whose text a model has already processed, and has the model retype the page through Write; it never landed a real source (`design-bilbo-library.md` in the planning notebook, "The library today", problem 3). This change lets bilbo fetch a page itself, convert HTML to Markdown in code, keep the raw bytes as evidence, warn about conversion defects, and ships the `ingest` skill that drives it, so a source can be verbatim by construction from the network to the file.

## What Changes

- `bilbo library stage <url>`: one GET through the existing `ureq`, redirects followed, for `http://` and `https://` URLs. The origin is `url: <url>` and `fetched` is today.
  - An HTML answer is converted to Markdown in code.
  - Other `text/*` answers, such as a site's own `.md` rendition of a page, are kept as text.
  - A PDF, an image, a non-2xx status, a body over 16 MiB, a body that is not UTF-8, or a page that converts to no text is refused with exit 1. The PDF refusal names the route for its text.
- HTML to Markdown through one new crate, `htmd` 0.5.5 (Apache-2.0), with bilbo's own rules on top:
  - every `<pre>` is a fenced code block;
  - a heading is one line of text, with no links or images;
  - a table without a header row still becomes a pipe table;
  - `script`, `style` and similar elements are dropped.
- The raw answer and a `fetch.json` record (URL, final URL, status, media type, time, converter) are written to the stage. `land`, unchanged, carries them into the capture folder beside `capture.md`.
- Stage warnings on stderr, for every stage:
  - an unclosed code fence;
  - a page heading that did not survive the conversion (HTML only);
  - runs of link-only lines that look like navigation.

  They warn and never refuse.
- Stage output gains lines:
  - `raw:`, `media type:` and `final url:` for a fetch;
  - `content:`, the lines that came from the page's `<main>` (or its only `<article>`), which also drive the suggested `title` and `keep`;
  - `existing:`, naming each source that already has this origin, which is how a re-ingest is noticed.
- `bilbo library stage <file> --html` converts a saved HTML file the same way. Its source stays `capture: external`.
- A source landed from a URL stage has no `capture` key.
- The `ingest` plugin skill (`plugins/bilbo/skills/ingest/SKILL.md`, offered as `bilbo:ingest`). It runs `stage`, reads the capture, chooses keep ranges, runs `land`, writes the guide entry and runs `bilbo check`.
  - WebFetch is never a route.
  - Text bilbo did not fetch goes through `stage <file>` and is labelled `capture: external`: a local file, a page saved by another tool, or a PDF's text from `pdftotext`.

## Capabilities

### New Capabilities

- `library-fetch`: staging a URL, what each kind of answer becomes, the fetch refusals, the HTML conversion rules, the raw fetch and `fetch.json`, and the three stage warnings.

### Modified Capabilities

- `library-ingest` (added by `add-library-store`, not yet in `openspec/specs/`):
  - Stage a file: gains `--html`.
  - Stage output: gains `raw:`, `media type:`, `final url:`, `content:` and `existing:`, and `title` and `keep` follow the content lines.
  - Land a source: no `capture` key for a URL stage.
- `agent-plugin`: Both MODIFIED requirements build on `add-library-reading`'s versions, which add `reference`, so they name all four skills.
  - One plugin for Claude Code and Codex: the plugin holds the `ingest` skill too.
  - A missing binary: the rule covers `ingest`, with its own line.
  - New requirements: The ingest skill, Ingest routes, Choosing keep ranges, Landing a source, The guide entry after ingest, and Report the ingest.

## Non-goals

- A product verb or flag that accepts text a model produced. WebFetch output has no route into the library.
- PDF extraction inside bilbo. A PDF's text comes from an external tool through `stage <file>`, labelled `capture: external`.
- Running JavaScript, logging in, cookies, or getting past bot protection. A page that needs any of these is saved by another tool and staged as a file.
- Removing navigation in code. The agent cuts it with keep ranges; bilbo only warns and suggests.
- A charset other than UTF-8, and Brotli bodies (`ureq` does not decode them).
- A setting for the timeout, the size limit or the user agent. No `library.` key is added.
- `library plan` and `library read` (`add-library-reading`, which ships first). The skill reads the landed source through them, and uses Read only on the staged `capture.md`.
- The citation pre-check on `land --replace` (`add-library-reading`). The skill only reacts to it when it appears.
- Changing the dnix `ingest` skill, its scripts or its consumers. The cutover switches them.
- The plugin manifests' descriptions. They stay as `add-library-reading` writes them.

## Impact

- `src/library.rs` (from `add-library-store`): `stage` tells a URL from a file, takes `--html`, prints the new header lines and passes on the warnings.
- New library modules:
  - `src/fetch.rs`: the GET through `ureq`, the media type, the final URL and the size limit.
  - `src/html.rs`: the conversion, the heading list for the heading gate, and the content lines. It is the one user of `htmd` and `markup5ever_rcdom`.
- `src/source.rs` (from `add-library-store`) gains the unclosed-fence and navigation checks over capture lines.
- `src/main.rs`: the USAGE line for `stage`.
- `Cargo.toml` and `Cargo.lock`:
  - `htmd` 0.5.5 and `markup5ever_rcdom` 0.38.0.
  - These add 20 crates in all, every one pure Rust under MIT and/or Apache-2.0 (design.md).
- Tests:
  - `tests/library.rs` gains the URL stage tests against a fake page server on 127.0.0.1, added to `tests/common/`.
  - `tests/fixtures/pages/` holds HTML pages and their expected Markdown.
  - `tests/plugin.rs` gains the `ingest` skill checks.
  - Unit tests go in the new modules.
- `plugins/bilbo/skills/ingest/SKILL.md` (new).
- `README.md`: `stage <url>`, `--html` and the `ingest` skill. `AGENTS.md`: the new modules, the dependency rule, and re-recording the page fixtures after an `htmd` bump.
- On disk: stage folders and capture folders also hold `raw` and `fetch.json`.
- Archive order: `library-ingest` must exist in `openspec/specs/` first, so `add-library-store` archives before this change. `openspec validate` says so as an info line.
