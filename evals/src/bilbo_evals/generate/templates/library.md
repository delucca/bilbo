You are choosing what to keep of a web page that was converted to Markdown, for a library of reference sources that coding agents read and cite. The page is from the public documentation of a software project.

URL: $url
Staged text: $lines lines, titled "$title". The converter suggests keeping lines $default_keep.

Headings of the staged text (line number, then the heading):
$headings

The first lines of the staged text:
$head

The last lines of the staged text:
$tail

Return:
- keep: the line ranges that are the page's own content, without navigation menus, site headers, search boxes, tables of contents of other pages, footers or page-generation notices. Write them as `a-b`, or `a-b,c-d` for several, ascending and not overlapping, with 1 <= a <= b <= $lines.
- name: a lowercase kebab-case ASCII name for the source, one to three words from the page's subject, such as `wal` or `busy-timeout`.
- guide_entry: two plain sentences saying what the source covers and when to consult it. No URL, no marketing words, no "TODO".
