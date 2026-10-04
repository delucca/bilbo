# library-fetch Specification

## Purpose
`bilbo library stage <url>` fetches a page itself, converts HTML to Markdown in code, and keeps the raw answer beside the capture. The text of a source can then be verbatim from the network to the file, and the stage warnings point at conversion defects before anything lands.

## Requirements

### Requirement: Stage a URL
An argument to `stage` that starts with `http://` or `https://` SHALL be fetched with one GET request, following at most 10 redirects. On a 2xx answer, `stage` SHALL write a new folder `<state>/bilbo/staging/<stage>/`, as for a file, holding `capture.md`, `raw` and `fetch.json`. The stage's origin SHALL be `url: <url>` with the URL as given, and its fetched date today's local date. Staging SHALL change nothing under the store root.

#### Scenario: A page is staged
- **WHEN** an agent runs `bilbo library stage https://go.dev/doc/effective_go` and the server answers 200 with `text/html`
- **THEN** the exit code is 0, the new stage folder holds `capture.md`, `raw` and `fetch.json`, and no entry under the store root is created or changed

#### Scenario: The origin is the URL as given
- **WHEN** `https://example.org/a` redirects to `https://example.org/b`, and the agent stages `https://example.org/a` and lands it
- **THEN** the source has `origin: "url: https://example.org/a"` and today's date as `fetched`

#### Scenario: Without a scheme it is a file
- **WHEN** an agent runs `bilbo library stage go.dev/doc/effective_go --origin "url: https://go.dev/doc/effective_go"` and no such file exists
- **THEN** no request is sent, stderr says the file is missing, and the exit code is 1

### Requirement: URL arguments
`--origin`, `--fetched` and `--html` SHALL be usage errors with a URL: bilbo sets the origin and the date, and reads the media type. A URL holding whitespace, `"`, `\` or a `#` fragment SHALL be a usage error. An argument that starts with letters followed by `://` and a scheme other than `http` or `https` SHALL be a usage error naming the schemes bilbo fetches.

#### Scenario: An origin with a URL
- **WHEN** an agent runs `bilbo library stage https://go.dev/doc/effective_go --origin "url: https://go.dev"`
- **THEN** bilbo prints a message naming `--origin` to stderr, exits 2 and sends no request

#### Scenario: A fragment
- **WHEN** an agent runs `bilbo library stage https://go.dev/doc/effective_go#names`
- **THEN** bilbo prints a message naming the fragment to stderr, exits 2 and sends no request

#### Scenario: Another scheme
- **WHEN** an agent runs `bilbo library stage ftp://example.org/spec.txt`
- **THEN** bilbo prints a message naming `http` and `https` to stderr and exits 2

### Requirement: What an answer becomes
The media type is the answer's `Content-Type` without parameters. `text/html` and `application/xhtml+xml` SHALL be converted to Markdown by the HTML conversion rules. Any other `text/*` type SHALL be kept as text. With no `Content-Type`, a body that starts, after whitespace, with `<!doctype html` or `<html` in any case SHALL be converted, and any other body kept as text. Kept text gets the normalization `stage` gives a file. Any other media type SHALL be refused.

#### Scenario: A site's Markdown
- **WHEN** the server answers `text/markdown; charset=utf-8` with Markdown that has CRLF line endings
- **THEN** `capture.md` holds that Markdown with LF line endings and nothing else changed

#### Scenario: HTML is converted
- **WHEN** the server answers `text/html` with `<h1>Errors</h1><p>Wrap them.</p>`
- **THEN** `capture.md` is `# Errors`, a blank line and `Wrap them.`, each line ending with a newline

#### Scenario: An image
- **WHEN** the server answers `image/png`
- **THEN** stderr names the media type, the exit code is 1, and no stage folder is left

### Requirement: PDF answers
A `application/pdf` answer SHALL be refused with exit 1, and the message SHALL say to extract the text with a PDF tool and stage that text file with `--origin "url: <url>"`.

#### Scenario: A PDF
- **WHEN** an agent stages `https://example.org/paper.pdf` and the server answers `application/pdf`
- **THEN** stderr names the URL, says it is a PDF and shows `--origin "url: https://example.org/paper.pdf"`, the exit code is 1, and no stage folder is left

### Requirement: Fetch refusals
`stage` SHALL exit 1, name the URL and the reason on stderr, and leave no stage folder when: the server cannot be reached; the answer is not 2xx after redirects, naming the status; more than 10 redirects are needed; the fetch has not finished within 60 seconds; the body is over 16 MiB; the body is not valid UTF-8 after a leading byte order mark is dropped; or the capture would hold only whitespace.

#### Scenario: Not found
- **WHEN** the server answers 404
- **THEN** stderr names the URL and `404`, the exit code is 1, and `<state>/bilbo/staging/` has no new folder

#### Scenario: Nobody answers
- **WHEN** nothing listens at the URL's host and port
- **THEN** stderr names the URL and says it is unreachable, and the exit code is 1

#### Scenario: Another charset
- **WHEN** the server answers `text/html; charset=iso-8859-1` with a body holding the byte `0xE9`
- **THEN** stderr says the body is not UTF-8, and the exit code is 1

#### Scenario: A page with no text
- **WHEN** the server answers `text/html` with a body that is only `<script>` elements and an empty `<div id="root">`
- **THEN** stderr says the page converted to no text, and the exit code is 1

#### Scenario: Too big
- **WHEN** the body is 16,777,217 bytes long
- **THEN** stderr names the 16 MiB limit, and the exit code is 1

### Requirement: HTML headings
Each `<h1>` to `<h6>` SHALL become one ATX heading line of its level. Its text is the heading's text content with links replaced by their text, images dropped, zero-width characters (U+200B, U+200C, U+200D, U+2060, U+FEFF) removed, and every run of whitespace, line breaks included, turned into one space. A heading with no text left SHALL produce no line.

#### Scenario: A self-link in a heading
- **WHEN** the page holds `<h2 id="x"><a href="#x">The <code>[package]</code> section</a></h2>`
- **THEN** the capture holds the line ``## The `[package]` section``

#### Scenario: A line break and a block in a heading
- **WHEN** the page holds `<h3>Sub<br>heading</h3>` and `<h2><div><a href="#b">&#8203;</a></div>Bundled skills</h2>`
- **THEN** the capture holds the lines `### Sub heading` and `## Bundled skills`

#### Scenario: An empty heading
- **WHEN** the page holds `<h4></h4>` or `<h4><a href="#y"><img src="link.svg"></a></h4>`
- **THEN** the capture holds no line starting with `#### `

### Requirement: HTML code blocks
Each `<pre>` element SHALL become a fenced code block whose content is the element's text with its whitespace kept. The fence SHALL be backticks, at least three and one more than the longest run of backticks in the content. The language SHALL be the part after `language-` of a `language-*` class on the `<pre>` or its `<code>`, and nothing when neither has one.

#### Scenario: Highlighted code with no code element
- **WHEN** the page holds `<pre><span class="kw">func</span> main() {}</pre>`
- **THEN** the capture holds ```` ``` ````, `func main() {}` and ```` ``` ```` on three lines

#### Scenario: Backticks inside the code
- **WHEN** the page holds `<pre><code class="language-rust">// ```</code></pre>`
- **THEN** the capture holds ```` ````rust ````, ```` // ``` ```` and ```` ```` ```` on three lines

#### Scenario: A heading-like line in code
- **WHEN** a `<pre>` holds the line `# install the tool`
- **THEN** that line sits inside a fenced block in the capture, and it is not a heading of the capture's outline

### Requirement: HTML tables
A `<table>` SHALL become a pipe table. When it has no header cell, its first row SHALL be the header row. A cell's content SHALL stay on one line, and a `|` in it SHALL be written so it does not end the cell.

#### Scenario: A table with a header
- **WHEN** the page holds a table with the header cells `Key` and `Type` and one row `name`, `string | null`
- **THEN** the capture holds a pipe table of two columns whose first row is `Key` and `Type`, followed by a delimiter row and one data row, and the data row has exactly two cells

#### Scenario: A table without a header
- **WHEN** the page holds `<table><tr><td>no</td><td>head</td></tr><tr><td>a</td><td>b</td></tr></table>`
- **THEN** the capture holds a pipe table whose first row holds `no` and `head`, then a delimiter row, then a row holding `a` and `b`, and no line holding only `no`

### Requirement: HTML conversion
The conversion SHALL drop `<head>`, `<script>`, `<style>`, `<noscript>`, `<template>` and `<svg>` with their content, write links inline as `[text](target)` with the target as written, write list items with `-` or `1.`, escape text that would otherwise read as Markdown syntax, and end every line, the last included, with a newline. It SHALL NOT remove navigation, headers or footers.

#### Scenario: Scripts and styles leave no text
- **WHEN** the page's `<head>` holds `<title>`, `<style>` and `<script>`, and its body holds an inline `<script>`
- **THEN** no text from those elements appears in the capture

#### Scenario: Navigation is kept
- **WHEN** the page holds `<nav><ul><li><a href="/">Home</a></li></ul></nav>` before its `<main>`
- **THEN** the capture holds the line `- [Home](/)`

### Requirement: The fetch record
For a URL stage, `raw` SHALL hold the response body as received, after any content encoding is undone, and `fetch.json` SHALL be a JSON object with `url` (as given), `final_url`, `status` (a number), `media_type` (the media type, or null), `fetched_at` (the time the answer arrived, RFC 3339 with the local offset), and `converter` (`bilbo <version>` when the capture was converted from HTML, else null). `land` SHALL keep both files in the capture folder beside `capture.md`.

#### Scenario: The record of a redirect
- **WHEN** an agent stages `http://127.0.0.1:<port>/old`, which redirects with 301 to `/new`, which answers 200 with `text/html`
- **THEN** `fetch.json` has `url` `http://127.0.0.1:<port>/old`, `final_url` `http://127.0.0.1:<port>/new`, `status` 200, `media_type` `text/html` and a `converter` naming bilbo's version, and `raw` equals the bytes `/new` served

#### Scenario: The record reaches the capture folder
- **WHEN** the agent lands that stage
- **THEN** the capture folder holds `capture.md`, `raw`, `fetch.json` and `landed`

#### Scenario: Text is not converted
- **WHEN** the server answered `text/plain`
- **THEN** `fetch.json` has `converter` null, and `raw` equals `capture.md` when the text had LF line endings and a final newline

### Requirement: Unclosed fence warning
When a code fence in the capture is never closed, `stage` SHALL print `unclosed fence: the code fence on line <n> is never closed` to stderr, `<n>` being its line, and still exit 0. A fence opens and closes as the outline's fence rule says.

#### Scenario: A fence left open
- **WHEN** a staged file opens a fence with ```` ``` ```` on line 12 and has no closing fence after it
- **THEN** stderr holds `bilbo: unclosed fence: the code fence on line 12 is never closed`, the exit code is 0, and the stage folder exists

#### Scenario: Balanced fences
- **WHEN** every fence in the capture is closed, a ```` ```` ```` fence among them holding a ```` ``` ```` line
- **THEN** stderr holds no `unclosed fence` line

### Requirement: Lost heading warning
For a capture converted from HTML, a page heading SHALL be lost when no heading line of the capture outside code fences has the same letters and digits, in the same order, ignoring every other character. `stage` SHALL print `heading lost: <h<level>> '<text>' is not a heading in the capture` to stderr for at most the first ten lost headings in page order, then `heading lost: <k> more` when there are more, and still exit 0. Headings with no letter or digit, and those inside dropped elements, are not counted.

#### Scenario: A heading inside a blockquote
- **WHEN** the page holds `<blockquote><h2>Documentation Index</h2></blockquote>`, which converts to `> ## Documentation Index`
- **THEN** stderr holds `bilbo: heading lost: <h2> 'Documentation Index' is not a heading in the capture`, and the exit code is 0

#### Scenario: Every heading survives
- **WHEN** every heading of the page has a heading line in the capture
- **THEN** stderr holds no `heading lost` line

#### Scenario: Many lost headings
- **WHEN** 14 headings are lost
- **THEN** stderr holds ten `heading lost: <h` lines and then `bilbo: heading lost: 4 more`

### Requirement: Navigation warning
A run of five or more lines outside code fences, with only blank lines between them, each holding nothing but one or more links or images, optionally after a list marker, separated by spaces, `|`, `·`, `•` or `,`, SHALL be a navigation suspect. `stage` SHALL print `navigation suspect: lines <a>-<b>, <n> lines of links only` to stderr for each such run, `<a>` and `<b>` being its first and last link lines, and still exit 0.

#### Scenario: A site menu
- **WHEN** lines 29 to 45 of the capture are nine `- [<text>](<target>)` lines with blank lines between them
- **THEN** stderr holds `bilbo: navigation suspect: lines 29-45, 9 lines of links only`, and the exit code is 0

#### Scenario: A short list of links
- **WHEN** the capture's only link-only lines are four consecutive `- [<text>](<target>)` lines
- **THEN** stderr holds no `navigation suspect` line

#### Scenario: A sentence with a link
- **WHEN** a run of six link lines has the line `See [the spec](/ref/spec) for details.` after its third line
- **THEN** that line breaks the run, and stderr holds no `navigation suspect` line for it
