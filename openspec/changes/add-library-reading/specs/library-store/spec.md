# Spec Delta

## MODIFIED Requirements

### Requirement: Anchors
An anchor SHALL name a section by its heading path, or by any trailing part of it, parts joined by ` > `. It matches a section when its parts equal the last parts of that section's heading path, each part and each heading compared after the Text normalization, with case. This one rule serves every verb that takes an anchor. One matching section resolves the anchor; several make it ambiguous; none make it missing.

#### Scenario: A trailing part resolves
- **WHEN** a source has the sections `needless_return > What it does` and `needless_range_loop > What it does`, and the anchor is `needless_return > What it does`
- **THEN** the anchor resolves to the first of them

#### Scenario: A bare heading that repeats is ambiguous
- **WHEN** the anchor is `What it does` in that source
- **THEN** the anchor is ambiguous

#### Scenario: Case counts
- **WHEN** the anchor is `what it does` in that source
- **THEN** the anchor is missing

#### Scenario: Markup in a heading
- **WHEN** a source has the heading `` ## The `Option` type `` and an agent runs `bilbo library show 'rust/book#The Option type'`
- **THEN** the anchor resolves to that section, as the anchor `` The `Option` type `` does

#### Scenario: Different words stay different
- **WHEN** the anchor is `The Options type` in that source
- **THEN** the anchor is missing

## ADDED Requirements

### Requirement: Text normalization
Where a spec compares text after the Text normalization, bilbo SHALL apply Unicode NFKC, decode `&amp;`, `&lt;`, `&gt;`, `&quot;`, `&#39;` and `&#124;`, make curly quotes straight, reduce links and images to their text and autolinks to their target, drop backslash escapes, `*`, `_`, backticks and `~~`, and collapse runs of whitespace to one space, trimmed. Case is kept.

#### Scenario: An entity in a table cell
- **WHEN** a source's table cell holds `a &#124; b` and the compared text is `a | b`
- **THEN** both normalize to `a | b`

#### Scenario: Emphasis and spacing
- **WHEN** one text is `the **zero  value**` and the other is `the zero value`
- **THEN** both normalize to `the zero value`

#### Scenario: Case is not folded
- **WHEN** one text is `Option` and the other is `option`
- **THEN** they normalize to different text
