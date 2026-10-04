//! HTML to Markdown, on `htmd` with four bilbo handlers.

use std::rc::Rc;

use htmd::element_handler::{HandlerResult, Handlers};
use htmd::options::{BulletListMarker, Options};
use htmd::{Element, HtmlToMarkdown, Node};
use markup5ever_rcdom::NodeData;

use crate::markdown;

const DROPPED: [&str; 6] = ["head", "script", "style", "noscript", "template", "svg"];
const ZERO_WIDTH: [char; 5] = ['\u{200B}', '\u{200C}', '\u{200D}', '\u{2060}', '\u{FEFF}'];

/// A page converted to Markdown.
pub struct Conversion {
    pub markdown: String,
    /// Level and text of every page heading outside dropped elements.
    pub headings: Vec<(u8, String)>,
    /// Markdown of the page's only `<main>`, or with none its only `<article>`.
    pub content: Option<String>,
}

fn tag(node: &Node) -> Option<&str> {
    match &node.data {
        NodeData::Element { name, .. } => Some(&name.local),
        _ => None,
    }
}

fn attr(node: &Node, key: &str) -> Option<String> {
    match &node.data {
        NodeData::Element { attrs, .. } => attrs
            .borrow()
            .iter()
            .find(|a| &*a.name.local == key)
            .map(|a| a.value.to_string()),
        _ => None,
    }
}

// `Node::parent` is a `Cell<Option<Weak<Node>>>`: take it, upgrade, put it back.
fn parent(node: &Rc<Node>) -> Option<Rc<Node>> {
    let weak = node.parent.take();
    let up = weak.as_ref().and_then(|w| w.upgrade());
    node.parent.set(weak);
    up
}

fn has_ancestor(node: &Rc<Node>, tags: &[&str]) -> bool {
    let mut cur = parent(node);
    while let Some(p) = cur {
        if tag(&p).is_some_and(|t| tags.contains(&t)) {
            return true;
        }
        cur = parent(&p);
    }
    false
}

const HEADINGS: [&str; 6] = ["h1", "h2", "h3", "h4", "h5", "h6"];

fn raw_text(node: &Rc<Node>, out: &mut String) {
    for child in node.children.borrow().iter() {
        match &child.data {
            NodeData::Text { contents } => out.push_str(&contents.borrow()),
            NodeData::Element { name, .. } => match &*name.local {
                "br" => out.push('\n'),
                t if DROPPED.contains(&t) => {}
                _ => raw_text(child, out),
            },
            _ => {}
        }
    }
}

fn one_line(s: &str) -> String {
    s.chars()
        .filter(|c| !ZERO_WIDTH.contains(c))
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn language(class: Option<String>) -> Option<String> {
    class?
        .split_whitespace()
        .find_map(|c| c.strip_prefix("language-").map(str::to_string))
        .filter(|l| !l.is_empty())
}

fn fence(text: &str) -> String {
    let (mut run, mut longest) = (0, 0);
    for c in text.chars() {
        run = if c == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    "`".repeat(3.max(longest + 1))
}

fn pre(_: &dyn Handlers, el: Element) -> Option<HandlerResult> {
    let mut text = String::new();
    raw_text(el.node, &mut text);
    let text = text.strip_suffix('\n').unwrap_or(&text);
    if text.trim().is_empty() {
        return None;
    }
    let code = el
        .node
        .children
        .borrow()
        .iter()
        .find(|c| tag(c) == Some("code"))
        .cloned();
    let lang = code
        .and_then(|c| language(attr(&c, "class")))
        .or_else(|| language(attr(el.node, "class")))
        .unwrap_or_default();
    let f = fence(text);
    Some(format!("\n\n{f}{lang}\n{text}\n{f}\n\n").into())
}

/// A trailing `#` run reads as an ATX closing sequence unless escaped.
fn escape_closing_hashes(text: &str) -> String {
    let head = text.trim_end_matches('#');
    if head.len() == text.len() || !(head.is_empty() || head.ends_with(' ')) {
        return text.to_string();
    }
    format!("{head}\\{}", &text[head.len()..])
}

fn heading(h: &dyn Handlers, el: Element) -> Option<HandlerResult> {
    let level: usize = el.tag[1..].parse().ok()?;
    let text = one_line(&h.walk_children(el.node).content);
    if text.is_empty() {
        return None;
    }
    Some(
        format!(
            "\n\n{} {}\n\n",
            "#".repeat(level),
            escape_closing_hashes(&text)
        )
        .into(),
    )
}

fn link(h: &dyn Handlers, el: Element) -> Option<HandlerResult> {
    if has_ancestor(el.node, &HEADINGS) {
        Some(h.walk_children(el.node))
    } else {
        h.fallback(el)
    }
}

fn image(h: &dyn Handlers, el: Element) -> Option<HandlerResult> {
    if has_ancestor(el.node, &HEADINGS) {
        None
    } else {
        h.fallback(el)
    }
}

// Rows of this table, not of a nested one.
fn rows(table: &Rc<Node>) -> Vec<Rc<Node>> {
    let mut out = Vec::new();
    for child in table.children.borrow().iter() {
        match tag(child) {
            Some("tr") => out.push(child.clone()),
            Some("thead" | "tbody" | "tfoot") => out.extend(
                child
                    .children
                    .borrow()
                    .iter()
                    .filter(|r| tag(r) == Some("tr"))
                    .cloned(),
            ),
            _ => {}
        }
    }
    out
}

fn table(h: &dyn Handlers, el: Element) -> Option<HandlerResult> {
    if has_ancestor(el.node, &["td", "th"]) {
        return h.fallback(el);
    }
    let grid: Vec<Vec<String>> = rows(el.node)
        .iter()
        .map(|r| {
            r.children
                .borrow()
                .iter()
                .filter(|c| matches!(tag(c), Some("td" | "th")))
                .map(|c| one_line(&h.walk_children(c).content).replace('|', "&#124;"))
                .collect()
        })
        .filter(|r: &Vec<String>| !r.is_empty())
        .collect();
    let cols = grid.iter().map(Vec::len).max().unwrap_or(0);
    if cols == 0 {
        return h.fallback(el);
    }
    let mut widths = vec![0; cols];
    for r in &grid {
        for (i, c) in r.iter().enumerate() {
            widths[i] = widths[i].max(c.chars().count());
        }
    }
    let line = |r: &[String]| {
        let mut s = String::from("|");
        for (i, w) in widths.iter().enumerate() {
            let c = r.get(i).map(String::as_str).unwrap_or("");
            s.push_str(&format!(" {c}{} |", " ".repeat(w - c.chars().count())));
        }
        s.push('\n');
        s
    };
    let mut md = String::from("\n\n");
    for caption in el
        .node
        .children
        .borrow()
        .iter()
        .filter(|c| tag(c) == Some("caption"))
    {
        let text = one_line(&h.walk_children(caption).content);
        if !text.is_empty() {
            md.push_str(&format!("{text}\n\n"));
        }
    }
    md.push_str(&line(&grid[0]));
    md.push('|');
    for w in &widths {
        md.push_str(&format!(" {} |", "-".repeat(*w)));
    }
    md.push('\n');
    for r in &grid[1..] {
        md.push_str(&line(r));
    }
    md.push('\n');
    Some(md.into())
}

fn converter() -> HtmlToMarkdown {
    HtmlToMarkdown::builder()
        .options(Options {
            bullet_list_marker: BulletListMarker::Dash,
            ul_bullet_spacing: 1,
            ol_number_spacing: 1,
            ..Options::default()
        })
        .skip_tags(DROPPED.to_vec())
        .add_handler(vec!["pre"], pre)
        .add_handler(HEADINGS.to_vec(), heading)
        .add_handler(vec!["a"], link)
        .add_handler(vec!["img"], image)
        .add_handler(vec!["table"], table)
        .build()
}

fn find_all(node: &Rc<Node>, name: &str, out: &mut Vec<Rc<Node>>) {
    for child in node.children.borrow().iter() {
        match tag(child) {
            Some(t) if DROPPED.contains(&t) => {}
            Some(t) if t == name => {
                out.push(child.clone());
                find_all(child, name, out);
            }
            _ => find_all(child, name, out),
        }
    }
}

fn page_headings(node: &Rc<Node>, out: &mut Vec<(u8, String)>) {
    for child in node.children.borrow().iter() {
        match tag(child) {
            Some(t) if DROPPED.contains(&t) => {}
            Some(t) if HEADINGS.contains(&t) => {
                let mut s = String::new();
                raw_text(child, &mut s);
                out.push((t.as_bytes()[1] - b'0', one_line(&s)));
            }
            _ => page_headings(child, out),
        }
    }
}

/// Normalizes line endings and ends the text with one newline.
fn finish(markdown: &str) -> String {
    let text = markdown.trim_end();
    if text.is_empty() {
        String::new()
    } else {
        format!("{text}\n")
    }
}

/// Converts `html`; never fails, because the HTML parser accepts any text.
pub fn convert(html: &str) -> Conversion {
    let c = converter();
    let tree = c
        .html_to_tree(html)
        .expect("html5ever never fails on a &str");
    let mut headings = Vec::new();
    page_headings(&tree, &mut headings);
    let markdown = finish(&c.tree_to_markdown(&tree));
    let one = |name| {
        let mut v = Vec::new();
        find_all(&tree, name, &mut v);
        (v.len() == 1).then(|| v.remove(0))
    };
    let content_node = one("main").or_else(|| {
        let mut mains = Vec::new();
        find_all(&tree, "main", &mut mains);
        if mains.is_empty() {
            one("article")
        } else {
            None
        }
    });
    let content = content_node.map(|n| finish(&c.tree_to_markdown(&n)));
    Conversion {
        markdown,
        headings,
        content,
    }
}

/// The 1-based inclusive lines of `markdown` that hold `content`, trimmed of
/// blank lines, when it occurs exactly once.
pub fn content_lines(markdown: &str, content: &str) -> Option<(usize, usize)> {
    let lines: Vec<&str> = markdown.lines().map(str::trim_end).collect();
    let mut want: Vec<&str> = content.lines().map(str::trim_end).collect();
    while want.last().is_some_and(|l| l.trim().is_empty()) {
        want.pop();
    }
    let lead = want.iter().take_while(|l| l.trim().is_empty()).count();
    let want = &want[lead..];
    if want.is_empty() || want.len() > lines.len() {
        return None;
    }
    let mut hits = lines
        .windows(want.len())
        .enumerate()
        .filter(|(_, w)| *w == want)
        .map(|(i, _)| i);
    let first = hits.next()?;
    hits.next()
        .is_none()
        .then_some((first + 1, first + want.len()))
}

/// The letters and digits of `text`, the key a heading is compared by.
fn alnum(text: &str) -> String {
    text.chars().filter(|c| c.is_alphanumeric()).collect()
}

/// The letters-and-digits keys of the heading lines the outline sees in `markdown`.
fn heading_keys(markdown: &str) -> Vec<String> {
    let lines = markdown::lines(markdown);
    markdown::outside_fences(&lines)
        .into_iter()
        .filter_map(|i| markdown::heading(lines[i]))
        .map(|(_, text)| alnum(&text))
        .collect()
}

/// The page headings with no heading line of the same letters and digits in
/// the Markdown, in page order. A heading with no letter or digit is skipped.
pub fn lost_headings(c: &Conversion) -> Vec<(u8, String)> {
    let kept = heading_keys(&c.markdown);
    c.headings
        .iter()
        .filter(|(_, text)| {
            let key = alnum(text);
            !key.is_empty() && !kept.contains(&key)
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md(html: &str) -> String {
        convert(html).markdown
    }

    fn has_line(md: &str, line: &str) -> bool {
        md.lines().any(|l| l == line)
    }

    #[test]
    fn heading_self_link_keeps_code_span() {
        let m = md(r##"<h2 id="x"><a href="#x">The <code>[package]</code> section</a></h2>"##);
        assert!(has_line(&m, "## The `[package]` section"), "{m}");
    }

    #[test]
    fn heading_with_break_and_block() {
        let m = md(
            r##"<h3>Sub<br>heading</h3><h2><div><a href="#b">&#8203;</a></div>Bundled skills</h2>"##,
        );
        assert!(has_line(&m, "### Sub heading"), "{m}");
        assert!(has_line(&m, "## Bundled skills"), "{m}");
    }

    #[test]
    fn empty_headings_make_no_line() {
        let m = md(r##"<h4></h4><h4><a href="#y"><img src="link.svg"></a></h4><p>x</p>"##);
        assert!(!m.lines().any(|l| l.starts_with("#### ")), "{m}");
    }

    #[test]
    fn image_only_heading_is_still_listed_without_text() {
        let c = convert(r#"<h2><img src="a.png"></h2>"#);
        assert_eq!(c.headings, vec![(2, String::new())]);
        assert!(lost_headings(&c).is_empty());
    }

    #[test]
    fn bare_pre_is_fenced() {
        let m = md(r#"<pre><span class="kw">func</span> main() {}</pre>"#);
        assert_eq!(m, "```\nfunc main() {}\n```\n");
    }

    #[test]
    fn fence_outgrows_backticks_and_takes_language() {
        let m = md(r#"<pre><code class="language-rust">// ```</code></pre>"#);
        assert_eq!(m, "````rust\n// ```\n````\n");
    }

    #[test]
    fn language_on_pre_and_on_code() {
        assert_eq!(
            md(r#"<pre class="x language-go">a</pre>"#),
            "```go\na\n```\n"
        );
        assert_eq!(
            md(r#"<pre class="language-sh"><code class="language-go">a</code></pre>"#),
            "```go\na\n```\n"
        );
    }

    #[test]
    fn pre_keeps_whitespace() {
        let m = md("<pre>a\n  b\n\n    c</pre>");
        assert_eq!(m, "```\na\n  b\n\n    c\n```\n");
    }

    #[test]
    fn heading_like_line_in_code_is_not_a_heading() {
        let c = convert("<pre># install the tool</pre>");
        assert_eq!(c.markdown, "```\n# install the tool\n```\n");
        assert!(heading_keys(&c.markdown).is_empty());
    }

    #[test]
    fn pre_in_a_list_item_stays_fenced() {
        let m = md("<ul><li>step<pre>run it\n  now</pre></li><li>next</li></ul>");
        let lines: Vec<&str> = m.lines().collect();
        let open = lines.iter().position(|l| l.trim() == "```").unwrap();
        assert!(lines[open + 1].trim() == "run it", "{m}");
        assert!(lines[open + 3].trim() == "```", "{m}");
        assert!(
            has_line(&m, "- next") || lines.iter().any(|l| l.starts_with("- next")),
            "{m}"
        );
    }

    #[test]
    fn table_with_header() {
        let m = md(
            "<table><tr><th>Key</th><th>Type</th></tr><tr><td>name</td><td>string | null</td></tr></table>",
        );
        let rows: Vec<&str> = m.lines().filter(|l| l.starts_with('|')).collect();
        assert_eq!(rows.len(), 3, "{m}");
        assert!(rows[0].contains("Key") && rows[0].contains("Type"));
        assert!(rows[1].contains("---"));
        assert!(rows[2].contains("name") && rows[2].contains("string"));
        assert!(
            rows[2].contains("string \\| null") || rows[2].contains("&#124;"),
            "{m}"
        );
    }

    #[test]
    fn table_without_header_uses_first_row() {
        let m = md("<table><tr><td>no</td><td>head</td></tr><tr><td>a</td><td>b</td></tr></table>");
        let rows: Vec<&str> = m.lines().filter(|l| l.starts_with('|')).collect();
        assert_eq!(rows.len(), 3, "{m}");
        assert!(rows[0].contains("no") && rows[0].contains("head"));
        assert!(rows[1].contains("---"));
        assert!(rows[2].contains('a') && rows[2].contains('b'));
        assert!(!m.lines().any(|l| l.trim() == "no"));
    }

    #[test]
    fn headerless_table_escapes_pipe_and_joins_lines() {
        let m = md("<table><tr><td>a | b</td><td>x<br>y</td></tr></table>");
        let row = m.lines().find(|l| l.starts_with('|')).unwrap();
        assert!(row.contains("a &#124; b"), "{m}");
        assert_eq!(row.matches('|').count(), 3, "{m}");
        assert!(row.contains("x y"), "{m}");
    }

    #[test]
    fn nested_table_stays_on_one_row() {
        let m = md(
            "<table><tr><td>outer</td><td><table><tr><td>in1</td><td>in2</td></tr></table></td></tr><tr><td>c</td><td>d</td></tr></table>",
        );
        let rows: Vec<&str> = m.lines().filter(|l| l.starts_with('|')).collect();
        assert!(
            rows.iter()
                .any(|r| r.contains("outer") && r.contains("in1")),
            "{m}"
        );
        assert!(
            rows.iter().any(|r| r.contains('c') && r.contains('d')),
            "{m}"
        );
    }

    #[test]
    fn dropped_elements_leave_no_text() {
        let m = md(
            "<html><head><title>T1tle</title><style>.s1{}</style><script>var s2;</script></head><body><script>var s3;</script><noscript>n0s</noscript><template>t3m</template><svg><text>sv9</text></svg><p>kept</p></body></html>",
        );
        assert_eq!(m, "kept\n");
    }

    #[test]
    fn navigation_is_kept() {
        let m = md(r#"<nav><ul><li><a href="/">Home</a></li></ul></nav><main><p>x</p></main>"#);
        assert!(has_line(&m, "- [Home](/)"), "{m}");
    }

    #[test]
    fn ordered_lists_escapes_and_final_newline() {
        let m = md("<ol><li>a</li><li>b</li></ol><p>1. not a list</p>");
        assert!(has_line(&m, "1. a"), "{m}");
        assert!(!has_line(&m, "1. not a list"), "{m}");
        assert!(m.ends_with('\n') && !m.ends_with("\n\n"));
        assert!(!m.contains('\r'));
    }

    #[test]
    fn content_is_the_only_main() {
        let c = convert("<h1>Site</h1><main><h1>Page</h1><p>body</p></main><p>foot</p>");
        assert_eq!(c.content.as_deref(), Some("# Page\n\nbody\n"));
        let lines = content_lines(&c.markdown, c.content.as_deref().unwrap());
        assert_eq!(lines, Some((3, 5)));
    }

    #[test]
    fn content_falls_back_to_the_only_article() {
        let c = convert("<p>a</p><article><p>b</p></article>");
        assert_eq!(c.content.as_deref(), Some("b\n"));
        let c = convert("<article><p>a</p></article><article><p>b</p></article>");
        assert_eq!(c.content, None);
    }

    #[test]
    fn two_mains_have_no_content() {
        let c = convert("<main><p>a</p></main><main><p>b</p></main><article><p>c</p></article>");
        assert_eq!(c.content, None);
    }

    #[test]
    fn no_main_or_article_has_no_content() {
        assert_eq!(convert("<p>a</p>").content, None);
    }

    #[test]
    fn content_occurring_twice_has_no_lines() {
        let c = convert("<p>dup</p><main><p>dup</p></main>");
        assert_eq!(c.content.as_deref(), Some("dup\n"));
        assert_eq!(content_lines(&c.markdown, "dup\n"), None);
    }

    #[test]
    fn content_lines_trims_blank_lines_and_counts_from_one() {
        let md = "a\n\nb\nc\n\nd\n";
        assert_eq!(content_lines(md, "\nb\nc\n\n"), Some((3, 4)));
        assert_eq!(content_lines(md, "b\nz\n"), None);
        assert_eq!(content_lines(md, "\n\n"), None);
    }

    #[test]
    fn heading_in_blockquote_is_lost() {
        let c = convert("<blockquote><h2>Documentation Index</h2></blockquote><h2>Kept</h2>");
        assert_eq!(
            lost_headings(&c),
            vec![(2, "Documentation Index".to_string())]
        );
    }

    #[test]
    fn surviving_headings_are_not_lost() {
        let c = convert(
            r##"<h1><a href="#x">The <code>[package]</code> section</a></h1><h2>C# &amp; Go</h2>"##,
        );
        assert!(lost_headings(&c).is_empty());
    }

    #[test]
    fn symbol_only_and_dropped_headings_are_not_counted() {
        let c = convert(
            "<h2>***</h2><noscript><h2>Inside</h2></noscript><template><h2>T</h2></template>",
        );
        assert_eq!(c.headings, vec![(2, "***".to_string())]);
        assert!(lost_headings(&c).is_empty());
    }

    #[test]
    fn lost_check_ignores_fenced_heading_lines() {
        let c = Conversion {
            markdown: "```\n# Only In Code\n```\n".into(),
            headings: vec![(1, "Only In Code".into())],
            content: None,
        };
        assert_eq!(lost_headings(&c).len(), 1);
    }

    #[test]
    fn convert_normalizes_crlf() {
        let m = md("<pre>a\r\nb</pre>");
        assert!(!m.contains('\r'), "{m:?}");
        assert!(m.ends_with("```\n"));
    }

    #[test]
    fn row_headers_keep_every_cell() {
        let m = md(
            r#"<table><tr><th scope="row">Name</th><td>x1</td></tr><tr><th scope="row">Age</th><td>y2</td></tr></table>"#,
        );
        for t in ["Name", "x1", "Age", "y2"] {
            assert!(m.contains(t), "{t} in {m}");
        }
        assert_eq!(m.lines().filter(|l| l.starts_with('|')).count(), 3, "{m}");
    }

    #[test]
    fn thead_and_tbody_headers_keep_every_cell() {
        let m = md(
            "<table><thead><tr><th>K</th><th>V</th></tr></thead><tbody><tr><th>name</th><td>x3</td></tr></tbody></table>",
        );
        for t in ["K", "V", "name", "x3"] {
            assert!(m.contains(t), "{t} in {m}");
        }
    }

    #[test]
    fn two_thead_rows_keep_both() {
        let m = md(
            "<table><thead><tr><th>a1</th><th>b1</th></tr><tr><th>a2</th><th>b2</th></tr></thead><tbody><tr><td>c</td><td>d</td></tr></tbody></table>",
        );
        for t in ["a1", "b1", "a2", "b2"] {
            assert!(m.contains(t), "{t} in {m}");
        }
    }

    #[test]
    fn caption_is_its_own_paragraph_before_the_table() {
        let m = md("<table><caption>Prices</caption><tr><td>a</td><td>b</td></tr></table>");
        assert!(m.starts_with("Prices\n\n| a"), "{m}");
    }

    #[test]
    fn closing_hash_run_in_a_heading_is_escaped() {
        let m = md("<h2>Section #</h2><h3>#</h3><h2>C#</h2>");
        assert!(has_line(&m, "## Section \\#"), "{m}");
        assert!(has_line(&m, "### \\#"), "{m}");
        assert!(has_line(&m, "## C#"), "{m}");
    }

    #[test]
    fn content_lines_ignores_trailing_whitespace() {
        let md = "head\n\nbody  \nlast \n\nfoot\n";
        assert_eq!(content_lines(md, "body\nlast\n"), Some((3, 4)));
        assert_eq!(content_lines(md, "body  \nlast   \n"), Some((3, 4)));
        let c = convert("<p>x</p><main><p>a<br></p></main>");
        assert_eq!(
            content_lines(&c.markdown, c.content.as_deref().unwrap()),
            Some((3, 3))
        );
    }

    #[test]
    fn heading_in_a_list_item_is_lost() {
        let c = convert("<ul><li><p>intro</p><h2>Inside Item</h2></li></ul>");
        assert_eq!(lost_headings(&c), vec![(2, "Inside Item".to_string())]);
    }
}
