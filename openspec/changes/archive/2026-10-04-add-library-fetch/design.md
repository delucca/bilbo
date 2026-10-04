# Design

## Context

See proposal.md for why. What exists, once `add-library-store` is applied, and constrains the approach:

- **Staging and landing.** `library stage <file>` writes `capture.md` and `stage.json` (origin, fetched, label, SHA-256 of `capture.md`) under `store::staging_dir`. `land` refuses a stage whose `capture.md` changed. It copies every file of the stage folder except `stage.json` into `<root>/.bilbo/captures/<sha256>/`, and keeps an existing capture folder as it is (`add-library-store` design.md, "Staging"; `library-ingest`, The capture land keeps). A `raw` and a `fetch.json` written into the stage therefore reach the capture with no change to `land`.
- **The outline.** `source::outline` finds headings with `rank::heading` outside fences, using `note::fence_run`. A line is a fence when it has at most three leading spaces and a run of at least three backticks or tildes. A backtick fence's info string holds no backtick (`src/note.rs`, `fence_run`). The stage warnings reuse the same rule, so "unclosed" means what the outline sees.
- **HTTP.** `ureq` 3.4.2 is already a dependency, with its default features (`rustls`, `gzip`) plus `json` (`Cargo.toml`, `Cargo.lock`). `src/embed.rs` and `src/model.rs` each build their own `ureq::Agent` with explicit timeouts and map `ureq::Error` to messages. `ureq` follows redirects (10 by default), exposes the final URI through `ResponseExt::get_uri`, and caps `read_to_vec` at 10 MiB unless `Body::with_config().limit()` raises it. Without its `charset` feature, it decodes nothing but gzip.
- **Tests.** `tests/common/mod.rs` runs a fake embedder on `127.0.0.1:0` from a thread, one request per connection. `flake.nix` sets `__darwinAllowLocalNetworking` for it and takes all of `tests/` into its fileset, so new fixtures under `tests/fixtures/` need no fileset edit.
- **The plugin.** `plugins/bilbo/skills/` holds `note` and `recall`, and `add-library-reading` adds `reference`, with its own versions of the agent-plugin requirements One plugin for Claude Code and Codex and A missing binary. `tests/plugin.rs` asserts that exact list, the frontmatter keys, and that each skill's `allowed-tools` covers every line of its `bash` blocks: a pattern ending in ` *` matches by prefix, any other must equal the line, and no line may chain.
- **The skill to replace.** dnix's `ingest` (`modules/ai/common/skills/ingest/SKILL.md`) resolves a notebook, fetches with WebFetch first (then Bright Data, then `curl | pandoc`), has the model strip navigation and Write the cleaned text, and splits long files. It then lands through `write_note.py`, keeps the raw HTML with `curl` afterwards, and has the model write index prose through `index_corpus.py`.

## Goals / Non-Goals

**Goals:**
- From `stage <url>` to `land`, no byte of a source passes through a model: the body is a function of the HTTP answer, bilbo's converter and the agent's line ranges.
- A conversion defect shows up as a warning at stage time, and the raw answer is kept to audit it later.
- The agent gets a good first guess for `--keep` and `--title` without bilbo deleting anything.
- One skill text for Claude Code and Codex.

**Non-Goals:**
- Readability-style extraction. Deciding what is navigation is judgment, and judgment stays with the agent, which names it in keep ranges.
- A perfect converter. Defects the warnings cannot see, such as a lost list nesting level, are accepted; `raw` lets a later converter be compared.
- Fetching through a proxy service or a browser.

## Decisions

### One new capability, three modified requirements

`library-fetch` holds what is new: the URL stage, what an answer becomes, the refusals, the conversion rules, the fetch record and the three warnings. The requirements of `add-library-store`'s `library-ingest` that this change alters are MODIFIED there: Stage a file (`--html`), Stage output (the new lines and the content-line rule for `title` and `keep`), and Land a source (no label for a URL stage). `openspec validate` notes that archive needs `library-ingest` in `openspec/specs/` first, which the shipping order gives.

Alternative: every new rule as an ADDED requirement in `library-fetch`. Stage output's `title` and `keep` rules would then be contradicted by a second spec instead of changed in place.

### Modules

- `src/library.rs` (the verb, from `add-library-store`) is extended. `stage` tells a URL from a file by its first argument. It refuses `--origin` and `--fetched` with a URL, takes `--html` with a file, and prints the new header lines and the warnings.
- `src/source.rs` (a library module from `add-library-store`) gains `capture_warnings(lines) -> Vec<String>` for the unclosed fence and the navigation runs. They are line rules over any capture and share the outline's fence logic.
- `src/fetch.rs`, a new library module, holds `get(url) -> Result<Answer, String>`. `Answer` carries the final URL, whether a redirect was followed, the status, the media type and the bytes. It is the third `ureq` user, after `embed` and `model`, each with its own agent settings, so it gets its own module instead of joining either.
- `src/html.rs`, a new library module, holds `convert(html) -> Conversion`: the Markdown, the page headings for the lost-heading check, and the content element's Markdown. It is the only user of `htmd` and `markup5ever_rcdom`.

Why not extend `source.rs` with the conversion: the converter is the one place that touches an HTML tree and two new crates. Kept apart, the dependency rule in AGENTS.md ("each dependency in its one user") reads as one line.

### The converter: `htmd` 0.5.5, with four bilbo handlers

std has no HTML parser, and none of bilbo's dependencies parses HTML. Converting in code is the point of the change, so a crate is needed. Candidates, checked on 2026-10-03 against crates.io and in scratch crates built in the dev shell (Rust 1.95.0). Each converted four real pages and one hand-written page. The real pages were go.dev's Effective Go (143 KB, 60 headings, 153 bare `<pre>` blocks) and the Cargo Book's manifest page (mdBook, 29 headings that link to themselves). The others were Claude Code's skills page (Mintlify, 1.19 MB, 62 headings whose anchor sits in a `<div>`, 12 tables) and Clippy's lint list (1.67 MB, 3,829 headings). The hand-written page held a header-less table, a `<br>` in a heading, backticks inside code, and navigation.

| Crate | Licence | Maintenance | Crates in its tree | Output |
|---|---|---|---|---|
| `htmd` 0.5.5 | Apache-2.0 | 5.3 M downloads; releases 2026-03 to 2026-07 | 30 with `markup5ever_rcdom`, 20 new to bilbo | Good lists, links, escapes and fence lengths. Stock defects: a bare `<pre>` loses its fence (0 of 153 fenced on Effective Go); a heading keeps link markup (27 of 29 on Cargo); a heading with a block child splits from its text (52 of 62 on Claude Code); a header-less table is flattened into paragraphs; a table with header cells keeps only `<th>` cells in its first row and only `<td>` cells in the others, so row headers vanish |
| `html-to-markdown-rs` 3.16.0 | MIT | 1.5 M downloads; 3.x weekly (3.14.3 on 09-19 to 3.16.0 on 10-02) | 83, among them `regex`, `url`, ICU and `tracing` | Fences bare `<pre>` and promotes a header row. Headings keep link and alt text (`[​ SVG Image](#bundled-skills) Bundled skills`). It drops `<nav>` by its own rule, adds YAML frontmatter by default, and leaves `1. not a list` unescaped, so a paragraph becomes a list |
| `mdka` 3.2.0 | Apache-2.0 | 3.0.0 to 3.2.0 within one week | 51, with `rayon` | Puts `<a id>` into heading text, does not lengthen a fence around backticks (an unclosed fence on the hand page), duplicates a `colspan` cell |
| `fast_html2md` 0.0.63 | MIT | 0.0.x | 85, async `futures` and `lol_html` | Not tried further |
| `html2md` 0.2.17 | GPL-3.0+ | | | Licence rules it out for an Apache-2.0 binary |
| `html2text` 0.17.1 | MIT | | | Renders text for terminals, not Markdown |

**Pick `htmd`.** It has the smallest tree, the same licence as bilbo, and the standard `html5ever` parser underneath. Its defects are local to four element types, and its builder takes custom handlers for them (`HtmlToMarkdownBuilder::add_handler`, with `Handlers::fallback` to reach the stock handler). The four handlers, in `src/html.rs`:

1. `pre`: walk the children with whitespace kept, and fence the text with a backtick fence one longer than any backtick run inside (at least three), taking `language-*` from the `<code>` child, else from the `<pre>`. The first draft fell back to `htmd` for a `<pre>` whose only child is `<code>`; review found `htmd`'s span handler drops newlines inside highlighted code, so bilbo fences every `<pre>` itself.
2. `h1` to `h6`: walk the children, drop zero-width characters, collapse all whitespace to single spaces, and emit `#` times the level, a space and the text on one line. Emit nothing when the text is empty.
3. `a` and `img` inside a heading: an `a` yields its children's text, and an `img` yields nothing. Outside headings they fall back. Finding the heading takes a walk up the `parent` links of `markup5ever_rcdom::Node`.
4. `table`: render the rows as a pipe table with the first row as header, the cells (`<td>` and `<th>`) in document order and a `<caption>` as a paragraph above it. Each cell is walked, collapsed to one line, and has `|` written as `&#124;`, as `htmd` does in its own tables. Only a table nested in a cell falls back to `htmd`, and its output is collapsed into the outer cell. An earlier draft let a table with a header cell fall back to `htmd`, until review found it dropped every `<th>` outside the first row (`<th scope="row">` key-value tables) and every `<td>` in it.

Measured with handlers 1 to 3 in the scratch crate, every `<pre>` on all four pages came out fenced (153 of 153 on Effective Go). Every heading came out as a heading with its text, except one Claude Code heading inside a `<blockquote>`. The lost-heading check reports exactly that one. Handler 4 is specified (`library-fetch`, HTML tables) and tested on the fixtures. Clippy's 1.67 MB page converts in 24 ms in release mode.

Options: `skip_tags` for `head`, `script`, `style`, `noscript`, `template` and `svg`, `-` bullets with one space, and `1.` with one space. Everything else is `htmd`'s default: ATX headings, fenced code, inline links, `TranslationMode::Pure`. `Faithful` mode was tried and keeps whole sections as raw HTML.

**`markup5ever_rcdom` 0.38.0 as a direct dependency.** `htmd` re-exports `Node` but not `NodeData`, and a handler must match on `NodeData::Element` to see a child's tag or walk to a parent heading. It is the exact crate and version `htmd` already pulls in, so it adds nothing to the tree. Its version carries `+unofficial` build metadata, which Cargo's `"0.38.0"` requirement matches. It lives in `src/html.rs` only.

**What the two add** (`cargo tree -e normal` on a copy of bilbo's manifest and lockfile, 2026-10-03): `htmd`, `html5ever`, `markup5ever`, `markup5ever_rcdom`, `xml5ever`, `tendril`, `string_cache`, `web_atoms`, `phf`, `phf_generator`, `phf_macros`, `phf_shared`, `precomputed-hash`, `siphasher`, `new_debug_unreachable`, `parking_lot`, `parking_lot_core`, `lock_api`, `scopeguard` and `fastrand`. That is 20 crates, all pure Rust, all MIT, Apache-2.0 or both. `libc`, `log`, `serde`, `smallvec` and the proc-macro crates are already in the tree. The `Cargo.toml` lines follow the house idiom (`htmd = "0.5.5"`, `markup5ever_rcdom = "0.38.0"`), and `Cargo.lock` pins them.

### Fetching

`src/fetch.rs` builds one agent per call:

- **Timeouts:** connect 30 seconds, whole request 60 seconds (`timeout_global`). A docs page arrives in well under a second, and 60 seconds bounds a stalled server.
- **Redirects:** 10 at most, `ureq`'s default; more is an error. `http` to `https` and back are both followed.
- **Headers:** `User-Agent: bilbo/<version>`. `Accept: text/html, application/xhtml+xml, text/markdown;q=0.9, text/plain;q=0.8, */*;q=0.1`, so a site that negotiates gives the page a browser would see. `Accept-Encoding` stays `ureq`'s `gzip`.
- **Statuses:** `ureq` turns 4xx and 5xx into `Error::StatusCode`, which becomes the refusal naming the status. A 1xx or 3xx left after redirects is refused the same way.
- **Body:** read through the decoded reader and cut at 16 MiB plus one byte, so the limit counts bytes after gzip decoding and a body of exactly 16 MiB passes; enough for Clippy's 1.67 MB page ten times over. A longer body is refused rather than cut, since a cut page is not the page.
- **Encoding:** UTF-8 only, checked on the bytes, whatever `charset` says. A page that is ASCII under another label passes. A Latin-1 page with accents is refused. The alternative, `ureq`'s `charset` feature, brings `encoding_rs` and its encoding tables, for pages docs sites rarely serve now. All four test pages were UTF-8.
- **Media type:** the `Content-Type` essence decides (`library-fetch`, What an answer becomes). With no header, a short prefix test tells HTML from text. Sniffing a typed answer was rejected: a site that labels Markdown `text/plain` is still served as text.

`http://` is accepted. Some documentation still serves plain HTTP, and the origin records which scheme was used.

### Origin, fragment and redirects

The origin is the URL as the agent gave it, and `final url:` and `fetch.json` record where it landed. The given URL is what the user named and what a re-ingest will fetch again. A redirect that marks a permanent move shows in `final url:`, and the skill can stage the final URL instead.

A `#fragment` is a usage error instead of being dropped quietly. A source is a whole document, and an origin with a fragment would make the same page look like two origins to the duplicate-origin warning and to `existing:`.

### The fetch record in the stage and the capture

`raw` is the body after gzip decoding: the bytes the converter read. `fetch.json` holds `url`, `final_url`, `status`, `media_type`, `fetched_at` (`jiff`, local offset, seconds) and `converter`. `converter` is `bilbo <version>`, because the conversion is `htmd` plus bilbo's handlers, and the bilbo version pins both through `Cargo.lock`.

`land` copies both into the capture folder unchanged. When the same `capture.md` was landed before, the folder exists and keeps the first `raw` and `fetch.json`: the same text came from that answer too. `stage.json` gains no field; its label for a URL stage is `fetched`, which `land` turns into no `capture` key.

`--html` on a file writes `raw` (the file's bytes) but no `fetch.json`, because nothing was fetched. Its source stays `capture: external`: bilbo converted the bytes, but another tool chose them.

### Content lines and the suggested keep

`html::convert` parses once (`HtmlToMarkdown::html_to_tree`) and renders the whole tree (`tree_to_markdown`). When the tree holds exactly one `<main>`, or none and exactly one `<article>`, it also renders that subtree. The content lines are where that subtree's Markdown, trimmed of blank lines, occurs in the whole, when it occurs exactly once. Measured: Effective Go 116-2266 of 2,292 lines, the Cargo manifest page 24-499 of 503, Claude Code's skills page 105-1148 of 1,172. Clippy's list has 852 `<article>` elements and no `<main>`, so it has no content lines.

`title` and `keep` then follow the content lines (`library-ingest`, Stage output). On the Cargo page that skips the book title `# The Cargo Book`, which sits outside `<main>`, and suggests `# The Manifest Format`. The suggestion only moves the starting point: the agent still reads the head and tail and cuts the in-page table of contents that Claude Code's `<main>` holds.

Rejected: printing the conversion of `<main>` as the capture. That would delete text in code by a rule some sites break, such as an `<aside>` inside `<main>` that holds the content, and the capture would no longer be the page.

### The three warnings

They go to stderr through `main`'s `bilbo: ` prefix, and `stage` still exits 0. A warning is a reason to read part of the capture, not proof of a defect, and some sites trip them by design.

- **Unclosed fence** applies to every capture. An unclosed fence hides every heading after it from the outline, so a source landed from it has a wrong outline and wrong anchors.
- **Lost heading** applies only to HTML, the only case with a second account of the headings. Comparing letters and digits in order ignores what the conversion rewrites on purpose: escapes, code spans, zero-width characters, collapsed whitespace and removed link targets. It still catches a heading that became a paragraph or sits in a blockquote. The cap of ten lines keeps a broken catalog from flooding the Bash output; the count after it says how bad it is.
- **Navigation suspect** applies to every capture. With five link-only lines as the threshold, it found 3 runs on Effective Go (the top menu and two footer blocks), 4 on Claude Code (menus and the in-page contents list), and none on the Cargo page or Clippy's list. Blank lines do not break a run, because `htmd` separates loose list items with them, and any other line does.

Rejected:
- A refusal on any warning. The agent can cut around navigation and lost chrome headings, and refusing would push it to the external route, which is worse.
- A count of `<pre>` against fenced blocks. Handler 1 fences every `<pre>` by construction, so the count could only fail on a bilbo bug, which the tests cover.

### `existing:` lines

`stage` reads the `origin` of every source in the library, read-only, and prints `existing: <corpus>/<name>` for each match. This is how the skill notices a re-ingest before landing a duplicate. `land` warns about a duplicate origin only after it wrote one. The scan reads frontmatter only, about 441 small reads on today's library.

### PDFs

bilbo does not extract PDF text. A PDF parser in Rust is a large dependency with uneven results, and `pdftotext` (poppler) already does it well. The route:

1. `stage <url>` refuses `application/pdf` and names the route, so an agent without the skill learns it too.
2. The skill downloads the PDF with `curl -fsSL -o` into a `mktemp -d` folder, writes the text with `pdftotext -layout`, and stages that text with `--origin "url: <url>"`.
3. The source is `capture: external`: bilbo chose neither the bytes' interpretation nor the tool.

Alternative: `stage <url>` keeps the PDF bytes in the stage and takes the text with a flag. That ties a product verb to an outside tool's output format, for the same `external` label.

### The skill file

`plugins/bilbo/skills/ingest/SKILL.md`, frontmatter keys in this order, as the other skills have them:

```yaml
---
name: ingest
description: Adds a web page, text file or PDF to the bilbo library as a source that bilbo fetches and keeps verbatim. Use for "ingest this", "save this page as a source", "add this doc to the library". NOT for notes (note).
license: Apache-2.0
allowed-tools: Bash(command -v bilbo), Bash(command -v pdftotext), Bash(bilbo library), Bash(bilbo library *), Bash(bilbo check), Bash(mktemp -d), Bash(curl -fsSL -o *), Bash(pdftotext -layout *), Read, Edit
---
```

`Bash(bilbo library)` is listed beside `Bash(bilbo library *)` because a bare `bilbo library` does not start with `bilbo library `. `Write` is not listed: the skill never creates a file, and the guide already exists when it edits it. The body follows `note`'s shape: a one-line purpose, numbered steps with an exit-code table per command, and a `Never` list.

1. **Missing binary.** `command -v bilbo`; when it prints nothing, stop with `ingest: bilbo is not on PATH; install the bilbo CLI first`.
2. **What was given.**
   - An `http(s)` URL: step 3.
   - A local text or Markdown file: stage it with `--origin`, `url: <where it came from>` when known, else `doc: <title or name of the document>`.
   - A saved HTML page: the same, plus `--html`.
   - A PDF: step 4.
   - Text pasted into the conversation is not a source: ask for its URL or a file.
3. **Stage the URL** with `bilbo library stage '<url>'`.

   | Exit | stderr | What to do |
   |---|---|---|
   | 0 | empty or warnings | step 5 |
   | 1 | says it is a PDF | step 4 |
   | 1 | anything else (status, unreachable, not UTF-8, no text) | say bilbo could not fetch it, ask the user to save the page with a tool of their own and give the file, then stage it with `--html` or as text |
   | 2 | anything | fix the argument and run it once more; on a second exit 2, print the first stderr line and stop |

   When many headings are lost, or the capture is mostly navigation, and the site offers a Markdown rendition of the page (`<url>.md` on many docs sites), stage that URL once. Keep it only when `media type:` is `text/markdown` or `text/plain`. go.dev answers `text/html` for `effective_go.md`, so a 200 alone proves nothing.
4. **PDF.** `command -v pdftotext`, then `mktemp -d`, then `curl -fsSL -o '<dir>/source.pdf' '<url>'` (skipped for a local PDF), then `pdftotext -layout '<pdf>' '<dir>/source.txt'`, then stage that file with `--origin`. Without `pdftotext`, stop and say what is needed.
5. **Choose the keep ranges.** Read `capture.md` from line 1 to 30 lines past the suggested `keep` start, the last 30 lines of the keep range and what follows, each navigation suspect, and each lost heading's neighbourhood. Keep the document: its text, headings, code, tables and footnotes. Drop menus, breadcrumbs, "on this page" lists, edit and feedback links, previous and next links, footers and cookie notices. Cut only between blocks, never inside a fence, and give several `--keep` ranges when chrome sits in the middle. The title is the document's, as `title:` prints it when it is right.
6. **Corpus and name.** Run `bilbo library`, and `bilbo library <corpus>` for a likely corpus. Use the corpus whose guide fits, or a new name in the topic grammar when none does; ask the user once when two fit equally. The name is two to six words of the title in kebab-case.
7. **Land** with `bilbo library land <stage> <corpus>/<name> --keep <ranges> --title '<title>'`, adding `--replace` only as the Landing a source requirement allows.

   | Exit | stderr | What to do |
   |---|---|---|
   | 0 | empty or warnings | step 8 |
   | 1 | names `--replace` | the name is taken: re-ingest or another name, as the user decides |
   | 1 | lists citations that would degrade | show them and ask before adding `--force` |
   | 1 | anything else | print the first stderr line and stop |
   | 2 | names `--keep`, `--title` or the target | fix it and run it once more; on a second exit 2, print the first stderr line and stop |

8. **Write the entry.** Run `bilbo library show <corpus>/<name>` for the outline. Read the body only through `bilbo library plan <corpus>/<name>` and `bilbo library read <plan> <slice>...`, every slice when `tokens:` is at most 60,000. Above that, or for a catalog, which `plan` refuses whole, plan the opening and the main sections by anchor up to 60,000 tokens, and say in the entry that it is a reference to look things up in. Base the entry on what was read. Read stays for `capture.md` and the `guide.md` the entry goes into, never a source. Edit the `guide.md` that `land` printed: replace `TODO: describe this source.` or the stale line with two or three sentences of your own, and replace a new corpus's `TODO: describe this corpus.` with a lead on what the corpus grounds.
9. **Check** with `bilbo check`. Fix lines naming `library/<corpus>/`, at most three runs, and leave and count the others. On `no store at <root>` or exit 2, print the first stderr line and stop.
10. **Report** as the Report the ingest requirement says.

`Never`: use WebFetch or a browser tool for the text; write or Edit a source or a `capture.md`; type source text into any file; use `curl` for anything but a PDF; set an environment variable in a command; chain commands.

**Kept from dnix's skill:** the choice of a name from the title, a PDF through an external tool, keeping the raw fetch, the entry prose ("covers, when to consult, what it gets wrong", in the agent's own words), and the final check. **Dropped:** notebook resolution, WebFetch and Bright Data, the model's cleanup through Write, `split_source.py`, `write_note.py`, `index_corpus.py`, `date +%F`, the Org link and `argument-hint`.

### Reading the landed source

`add-library-reading` ships before this change, so step 8 reads the body through `library plan` and `library read`, the one read path the `reference` skill uses too, and the read log shows what the entry rests on. Read is used only on the staged `capture.md`, which is not a source and has no plan. `Bash(bilbo library *)` covers both verbs.

Alternative: Read for the landed body, cheaper per call, which would give the plugin a second read path for sources.

### Testing without the network

A second fake in `tests/common/`, a page server on `127.0.0.1:0` in the shape of the fake embedder. It holds a table from path to answer: status, headers and body bytes. A 301 answer names its `Location`. It records requests, so a test can assert that a usage error sent none. Pages live in `tests/fixtures/pages/` as `<name>.html` and the expected `<name>.md`, and a test asserts `capture.md` equals the `.md` byte for byte. The fixtures are small pages that carry each case: bare and nested `<pre>`, backticks in code, headings with self-links, `<br>`, zero-width anchors and images, tables with and without headers, `<main>` with navigation and footer around it, a blockquoted heading, and a nav menu. When an `htmd` bump changes an expected `.md`, the diff is reviewed and the file re-recorded; AGENTS.md says so.

Not tested offline: TLS, gzip decoding and a real timeout. They are `ureq`'s own behavior, and a 60-second test would slow the suite. Task 5.4 fetches two real pages once, by hand, and records the result.

## Risks / Trade-offs

- [A site serves a bot wall with status 200] → The capture is the wall's text. The navigation and lost-heading warnings rarely fire on it, so the skill's read of the head of the capture is what catches it. The agent then takes the external route.
- [A JavaScript-rendered page converts to chrome only] → It is refused when nothing is left. Otherwise the agent sees no document in the capture and takes the external route, or the site's `.md` rendition.
- [`htmd` changes output in a patch release] → `Cargo.lock` pins it, and the page fixtures fail on any change, so an upgrade is a reviewed diff.
- [Escapes in converted text, such as `absolute\_paths`] → They are valid Markdown, and today's sources already hold them. Anchors match headings as written. `add-library-reading`'s citation normalization should treat a backslash escape as the character it escapes, and its drafter is told.
- [`&#124;` in table cells] → It renders as `|` and keeps the cell boundary. A quote copied from a cell carries the entity.
- [The first `raw` wins when the same text is landed twice] → The text is identical, so either answer proves it.
- [Plain `http://` fetches can be tampered with in transit] → The scheme is in the origin, and the skill prefers `https` when the user gives a bare host.
- [Reading a large source for the entry is costly] → Above 60,000 tokens the skill plans the opening and the main sections only, and says so in the entry.

## Migration Plan

- The product: `stage <file>` keeps its behavior without `--html`, and existing sources and captures are untouched. New captures from URLs hold `raw` and `fetch.json` beside `capture.md`.
- The plugin: `bilbo:ingest` appears beside dnix's `ingest`, as `bilbo:note` did beside dnix's `note`. The cutover switches dnix.
- Rollback: drop the two crates and the modules. Sources landed from URLs stay valid: their files follow `library-store` whether or not this change exists.

## Decisions to confirm

1. `htmd` 0.5.5 plus `markup5ever_rcdom` 0.38.0 as a direct dependency, with four bilbo handlers (`pre`, headings, links and images in headings, tables), over `html-to-markdown-rs`, which needs no handlers but brings 83 crates.
2. A header-less table gets its first row as header, instead of being flattened or kept as raw HTML.
3. UTF-8 only; no `charset` feature.
4. 60-second timeout and 16 MiB body limit, fixed, with no setting.
5. The origin is the URL as given, not the final URL. A `#fragment` is a usage error.
6. Other `text/*` answers are kept as text, so a site's `.md` rendition is a fetched source with no label.
7. `content:` lines from the only `<main>` or `<article>` drive the suggested `title` and `keep`; nothing is removed in code.
8. The three warnings never refuse. Lost headings are capped at ten lines, and navigation needs five link-only lines.
9. `existing:` lines in the stage output.
10. `stage <file> --html` for pages saved by other tools, still `capture: external`.
11. PDFs: refused by `stage <url>`, and turned into text by `pdftotext -layout` in the skill, labelled `external`.
12. The skill uses `curl` only to download a PDF, and pasted text is never a source.
13. The plugin manifests' descriptions stay as `add-library-reading` writes them.
