use crate::note::fence_run;

pub const PART_BYTES: usize = 4000;

const K1: f64 = 1.2;
const B: f64 = 0.75;

#[derive(Debug, Clone, PartialEq)]
pub struct Passage {
    /// Heading texts from the note's title down; never empty.
    pub path: Vec<String>,
    /// Physical line the passage or part starts on.
    pub line: usize,
    /// The passage's lines without its heading line, from the first to the last non-blank line, joined with '\n'.
    pub text: String,
}

pub struct Document {
    pub passages: Vec<Passage>,
}

#[derive(Debug, PartialEq)]
pub struct Hit {
    /// Index into the documents given to `rank`.
    pub document: usize,
    /// Index into that document's passages.
    pub passage: usize,
}

/// The folded words of `text`, in order.
pub fn words(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    each_word(text, |word| found.push(word.to_string()));
    found
}

/// Runs of alphanumeric chars, lowercased and accent-folded, of 2 or more chars once folded.
fn each_word(text: &str, mut f: impl FnMut(&str)) {
    let mut word = String::new();
    for c in text.chars().chain([' ']) {
        if is_mark(c) {
            continue;
        }
        if c.is_alphanumeric() {
            for lower in c.to_lowercase().filter(|l| !is_mark(*l)) {
                match base(lower) {
                    Some(b) => word.push_str(b),
                    None => word.push(lower),
                }
            }
        } else {
            if word.chars().count() >= 2 {
                f(&word);
            }
            word.clear();
        }
    }
}

/// Combining diacritical marks: part of the word they follow, and dropped.
fn is_mark(c: char) -> bool {
    ('\u{300}'..='\u{36f}').contains(&c)
}

/// The base letters of a lowercase Latin-1 Supplement or Latin Extended-A letter.
fn base(c: char) -> Option<&'static str> {
    Some(match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => "a",
        'æ' => "ae",
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => "c",
        'ð' | 'ď' | 'đ' => "d",
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => "e",
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => "g",
        'ĥ' | 'ħ' => "h",
        'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => "i",
        'ĳ' => "ij",
        'ĵ' => "j",
        'ķ' | 'ĸ' => "k",
        'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => "l",
        'ñ' | 'ń' | 'ņ' | 'ň' | 'ŉ' | 'ŋ' => "n",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => "o",
        'œ' => "oe",
        'ŕ' | 'ŗ' | 'ř' => "r",
        'ś' | 'ŝ' | 'ş' | 'š' | 'ſ' => "s",
        'ß' => "ss",
        'ţ' | 'ť' | 'ŧ' => "t",
        'þ' => "th",
        'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => "u",
        'ŵ' => "w",
        'ý' | 'ÿ' | 'ŷ' => "y",
        'ź' | 'ż' | 'ž' => "z",
        _ => return None,
    })
}

/// An ATX heading outside a fence: its level and its text with the closing sequence removed and whitespace collapsed.
fn heading(line: &str) -> Option<(usize, String)> {
    let level = line.bytes().take_while(|b| *b == b'#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let mut text = line[level..].strip_prefix(' ')?.trim_end();
    if text.ends_with('#') {
        let stripped = text.trim_end_matches('#');
        if stripped.is_empty() || stripped.ends_with(char::is_whitespace) {
            text = stripped;
        }
    }
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (!text.is_empty()).then_some((level, text))
}

/// `lines` is a note's body, whose first line is physical line `first_line`. `fallback_title` is the title when the
/// body has no `# ` heading.
pub fn passages(lines: &[&str], first_line: usize, fallback_title: &str) -> Vec<Passage> {
    let mut fence: Option<(char, usize)> = None;
    let mut headings = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let run = fence_run(line);
        match (fence, run) {
            (Some((ch, len)), Some((c, l, rest)))
                if c == ch && l >= len && rest.trim().is_empty() =>
            {
                fence = None;
            }
            (Some(_), _) => {}
            (None, Some((c, l, _))) => fence = Some((c, l)),
            (None, None) => {
                if let Some((level, text)) = heading(line) {
                    headings.push((i, level, text));
                }
            }
        }
    }

    let title_at = headings.iter().position(|(_, level, _)| *level == 1);
    let title = title_at.map_or(fallback_title, |t| headings[t].2.as_str());
    let mut stack: Vec<(usize, &str)> = Vec::new();
    let mut paths = Vec::new();
    for (h, (_, level, text)) in headings.iter().enumerate() {
        if Some(h) == title_at {
            stack.clear();
        } else {
            while stack.last().is_some_and(|(top, _)| top >= level) {
                stack.pop();
            }
            stack.push((*level, text));
        }
        let mut path = vec![title.to_string()];
        if Some(h) != title_at {
            path.extend(stack.iter().map(|(_, text)| text.to_string()));
        }
        paths.push(path);
    }

    let mut found = Vec::new();
    let preamble_end = headings.first().map_or(lines.len(), |(i, _, _)| *i);
    if let Some((from, to)) = trimmed(&lines[..preamble_end]) {
        found.extend(parts(
            vec![title.to_string()],
            &lines[from..to],
            first_line + from,
            first_line + from,
        ));
    }
    for (h, (i, _, _)) in headings.iter().enumerate() {
        let end = headings
            .get(h + 1)
            .map_or(lines.len(), |(next, _, _)| *next);
        let body = &lines[i + 1..end];
        let (from, to) = trimmed(body).unwrap_or((0, 0));
        found.extend(parts(
            paths[h].clone(),
            &body[from..to],
            first_line + i + 1 + from,
            first_line + i,
        ));
    }
    found
}

/// The range from the first to the last non-blank line, or `None` when every line is blank.
fn trimmed(lines: &[&str]) -> Option<(usize, usize)> {
    let from = lines.iter().position(|l| !l.trim().is_empty())?;
    let to = lines.iter().rposition(|l| !l.trim().is_empty())?;
    Some((from, to + 1))
}

/// One passage, or several of at most `PART_BYTES` bytes when the text is longer. `lines` run from the first to the
/// last non-blank line and start on physical line `first_line`; the first part starts on `start_line`.
fn parts(path: Vec<String>, lines: &[&str], first_line: usize, start_line: usize) -> Vec<Passage> {
    let text = lines.join("\n");
    if text.len() <= PART_BYTES {
        return vec![Passage {
            path,
            line: start_line,
            text,
        }];
    }

    // Each part is its first physical line and its text.
    let mut done: Vec<(usize, String)> = Vec::new();
    let mut current: Option<(usize, usize)> = None;
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim().is_empty() {
            i += 1;
            continue;
        }
        let start = i;
        while i < lines.len() && !lines[i].trim().is_empty() {
            i += 1;
        }
        let end = i - 1;
        let paragraph = lines[start..=end].join("\n");
        if paragraph.len() > PART_BYTES {
            if let Some((a, b)) = current.take() {
                done.push((first_line + a, lines[a..=b].join("\n")));
            }
            let mut rest = paragraph.as_str();
            let mut cut_at = 0;
            loop {
                // A piece that begins at a line break starts on the next line.
                let lead = rest.len() - rest.trim_start_matches('\n').len();
                let skipped = paragraph[..cut_at + lead].matches('\n').count();
                let cut = if rest.len() > PART_BYTES {
                    rest.floor_char_boundary(PART_BYTES)
                } else {
                    rest.len()
                };
                done.push((first_line + start + skipped, rest[..cut].to_string()));
                cut_at += cut;
                rest = &rest[cut..];
                if rest.is_empty() {
                    break;
                }
            }
            continue;
        }
        current = match current {
            Some((a, _)) if lines[a..=end].join("\n").len() <= PART_BYTES => Some((a, end)),
            Some((a, b)) => {
                done.push((first_line + a, lines[a..=b].join("\n")));
                Some((start, end))
            }
            None => Some((start, end)),
        };
    }
    if let Some((a, b)) = current {
        done.push((first_line + a, lines[a..=b].join("\n")));
    }

    done.into_iter()
        .enumerate()
        .map(|(n, (line, text))| Passage {
            path: path.clone(),
            line: if n == 0 { start_line } else { line },
            text,
        })
        .collect()
}

/// Each document's best passage holding at least one of `query` (folded words; repeats count once), best first.
/// Ties: lower document index first. Callers pass documents in path order.
pub fn rank(query: &[String], documents: &[Document]) -> Vec<Hit> {
    let mut words: Vec<&str> = Vec::new();
    for word in query {
        if !words.contains(&word.as_str()) {
            words.push(word);
        }
    }

    // (document, passage, word count, count of each query word)
    let mut stats: Vec<(usize, usize, usize, Vec<usize>)> = Vec::new();
    for (d, document) in documents.iter().enumerate() {
        for (p, passage) in document.passages.iter().enumerate() {
            let mut dl = 0;
            let mut tf = vec![0; words.len()];
            let mut count = |word: &str| {
                dl += 1;
                if let Some(q) = words.iter().position(|w| *w == word) {
                    tf[q] += 1;
                }
            };
            for segment in &passage.path {
                each_word(segment, &mut count);
            }
            each_word(&passage.text, &mut count);
            stats.push((d, p, dl, tf));
        }
    }
    let n = stats.len();
    let total: usize = stats.iter().map(|(_, _, dl, _)| dl).sum();
    let avgdl = total as f64 / n.max(1) as f64;
    let df: Vec<usize> = (0..words.len())
        .map(|q| stats.iter().filter(|(_, _, _, tf)| tf[q] > 0).count())
        .collect();

    let mut best: Vec<Option<(f64, usize)>> = vec![None; documents.len()];
    for (d, p, dl, tf) in &stats {
        let mut score = 0.0;
        let mut candidate = false;
        for (q, tf) in tf.iter().enumerate().filter(|(_, tf)| **tf > 0) {
            candidate = true;
            let (tf, df, n) = (*tf as f64, df[q] as f64, n as f64);
            let idf = (1.0 + (n - df + 0.5) / (df + 0.5)).ln();
            score += idf * tf * (K1 + 1.0) / (tf + K1 * (1.0 - B + B * *dl as f64 / avgdl));
        }
        if candidate && best[*d].is_none_or(|(top, _)| score > top) {
            best[*d] = Some((score, *p));
        }
    }

    let mut hits: Vec<(f64, Hit)> = best
        .into_iter()
        .enumerate()
        .filter_map(|(document, best)| {
            best.map(|(score, passage)| (score, Hit { document, passage }))
        })
        .collect();
    hits.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.document.cmp(&b.1.document)));
    hits.into_iter().map(|(_, hit)| hit).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_fold_case_and_accents() {
        assert_eq!(words("Decisão tomada"), ["decisao", "tomada"]);
        assert_eq!(words("DECISAO"), ["decisao"]);
    }

    #[test]
    fn words_expand_letters_without_a_base() {
        assert_eq!(words("ß"), ["ss"]);
        assert_eq!(words("Straße ẞ"), ["strasse", "ss"]);
        assert_eq!(words("Æon Œuvre þ"), ["aeon", "oeuvre", "th"]);
    }

    #[test]
    fn words_split_on_underscore_and_punctuation() {
        assert_eq!(words("snake_case"), ["snake", "case"]);
        assert_eq!(words("foo-bar.baz 2x 9"), ["foo", "bar", "baz", "2x"]);
    }

    #[test]
    fn words_drop_one_char_words() {
        assert_eq!(words("a b cd"), ["cd"]);
        assert!(words("é").is_empty());
    }

    #[test]
    fn letters_outside_the_table_pass_through() {
        assert_eq!(words("Ωμέγα"), ["ωμέγα"]);
        assert_eq!(words("Ștefan"), ["ștefan"]);
    }

    #[test]
    fn every_table_letter_folds_to_ascii() {
        for c in '\u{c0}'..='\u{17f}' {
            if !c.is_alphanumeric() {
                continue;
            }
            let found = words(&format!("{c}{c}"));
            assert_eq!(found.len(), 1, "{c}: {found:?}");
            assert!(
                found[0].bytes().all(|b| b.is_ascii_lowercase()),
                "{c}: {found:?}"
            );
        }
    }

    #[test]
    fn marks_join_and_vanish() {
        assert_eq!(words("İstanbul İ"), ["istanbul"]);
        assert_eq!(words("decisa\u{303}o"), ["decisao"]);
    }

    fn texts(found: &[Passage]) -> Vec<(Vec<&str>, usize, &str)> {
        found
            .iter()
            .map(|p| {
                let path = p.path.iter().map(String::as_str).collect();
                (path, p.line, p.text.as_str())
            })
            .collect()
    }

    #[test]
    fn nested_heading_path() {
        let lines = [
            "# Embedder",
            "",
            "## Gotchas",
            "",
            "### Two slots",
            "",
            "Use two slots.",
        ];
        let found = passages(&lines, 6, "fallback");
        let last = found.last().unwrap();
        assert_eq!(last.path, ["Embedder", "Gotchas", "Two slots"]);
        assert_eq!(last.line, 10);
        assert_eq!(last.text, "Use two slots.");
    }

    #[test]
    fn fenced_heading_opens_no_passage() {
        for fence in ["```", "~~~"] {
            let lines = [
                "# T",
                "## Setup",
                "",
                fence,
                "# install deps",
                fence,
                "## After",
            ];
            let found = passages(&lines, 1, "fb");
            let paths: Vec<_> = found.iter().map(|p| p.path.join(" > ")).collect();
            assert_eq!(paths, ["T", "T > Setup", "T > After"], "{fence}");
            assert!(found[1].text.contains("# install deps"), "{fence}");
        }
        let unclosed = ["## Setup", "~~~", "# hidden", "## also hidden"];
        let found = passages(&unclosed, 1, "fb");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, ["fb", "Setup"]);
        assert!(found[0].text.contains("also hidden"));
    }

    #[test]
    fn heading_line_rules() {
        for line in ["#", "# ", "#tag", "####### x", " ## x", "#\tx", "## ##"] {
            assert_eq!(heading(line), None, "{line:?}");
        }
        assert_eq!(heading("## Foo ##"), Some((2, "Foo".into())));
        assert_eq!(heading("## C#"), Some((2, "C#".into())));
        assert_eq!(heading("##   a   b"), Some((2, "a b".into())));
        assert_eq!(heading("## Foo ##   "), Some((2, "Foo".into())));
        let body = passages(&["#tag", "text"], 1, "fb");
        assert_eq!(texts(&body), [(vec!["fb"], 1, "#tag\ntext")]);
    }

    #[test]
    fn preamble_and_title() {
        let lines = ["", "intro text", "more", "", "# T", "x", "# Two", "y"];
        let found = passages(&lines, 1, "fb");
        assert_eq!(
            texts(&found),
            [
                (vec!["T"], 2, "intro text\nmore"),
                (vec!["T"], 5, "x"),
                (vec!["T", "Two"], 7, "y"),
            ]
        );

        let blank = passages(&["", "# T"], 1, "fb");
        assert_eq!(texts(&blank), [(vec!["T"], 2, "")]);

        let early = passages(&["## Early", "a", "# T"], 1, "fb");
        assert_eq!(
            texts(&early),
            [(vec!["T", "Early"], 1, "a"), (vec!["T"], 3, "")]
        );
    }

    #[test]
    fn no_title_uses_the_fallback() {
        let found = passages(&["## A", "text"], 1, "plan-x");
        assert_eq!(texts(&found), [(vec!["plan-x", "A"], 1, "text")]);
        let bare = passages(&["just text"], 1, "plan-x");
        assert_eq!(texts(&bare), [(vec!["plan-x"], 1, "just text")]);
    }

    #[test]
    fn lines_follow_first_line() {
        let found = passages(&["a", "b", "c", "## H", "d"], 5, "fb");
        assert_eq!(found[0].line, 5);
        assert_eq!(found[1].line, 8);
    }

    #[test]
    fn exactly_4000_bytes_is_one_part() {
        let exact = "a".repeat(PART_BYTES);
        let lines = ["## S", exact.as_str()];
        assert_eq!(passages(&lines, 1, "fb").len(), 1);
        let over = "a".repeat(PART_BYTES + 1);
        let lines = ["## S", over.as_str()];
        let found = passages(&lines, 1, "fb");
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].text.len(), PART_BYTES);
        assert_eq!(found[1].text.len(), 1);
    }

    #[test]
    fn long_section_splits_at_blank_lines() {
        let paragraphs: Vec<String> = (0..10)
            .map(|i| format!("p{i}: {}", "w".repeat(895)))
            .collect();
        let mut lines = vec!["## S"];
        for paragraph in &paragraphs {
            lines.push("");
            lines.push(paragraph);
        }
        let found = passages(&lines, 1, "fb");
        assert!(found.len() > 1);
        assert_eq!(found[0].line, 1);
        for part in &found {
            assert!(part.text.len() <= PART_BYTES);
            assert_eq!(part.path, ["fb", "S"]);
            assert!(part.text.starts_with('p'));
        }
        for (i, paragraph) in paragraphs.iter().enumerate() {
            let holders: Vec<_> = found
                .iter()
                .filter(|part| part.text.contains(paragraph.as_str()))
                .collect();
            assert_eq!(holders.len(), 1, "paragraph {i}");
        }
        for part in &found[1..] {
            let i: usize = part.text[1..part.text.find(':').unwrap()].parse().unwrap();
            assert_eq!(part.line, 3 + 2 * i);
        }
    }

    #[test]
    fn long_paragraph_cuts_on_a_char_boundary() {
        let straddle = format!("{}{}", "a".repeat(3999), "ã".repeat(500));
        let lines = ["## S", straddle.as_str()];
        let found = passages(&lines, 1, "fb");
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].text.len(), 3999);
        assert_eq!(format!("{}{}", found[0].text, found[1].text), straddle);

        let tail = "ã".repeat(2500);
        let lines = ["## S", "line one", tail.as_str()];
        let found = passages(&lines, 1, "fb");
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].line, 1);
        assert_eq!(found[1].line, 3);
        assert_eq!(
            format!("{}{}", found[0].text, found[1].text),
            format!("line one\n{tail}")
        );
    }

    #[test]
    fn a_cut_before_a_line_break_starts_on_the_next_line() {
        let long = "a".repeat(PART_BYTES);
        let lines = ["## S", long.as_str(), "bbb"];
        let found = passages(&lines, 1, "fb");
        assert_eq!(found.len(), 2);
        assert_eq!(found[1].text, "\nbbb");
        assert_eq!(found[1].line, 3);
    }

    fn document(texts: &[&str]) -> Document {
        Document {
            passages: texts
                .iter()
                .enumerate()
                .map(|(i, text)| Passage {
                    path: vec!["x".into()],
                    line: i + 1,
                    text: text.to_string(),
                })
                .collect(),
        }
    }

    fn order(hits: &[Hit]) -> Vec<usize> {
        hits.iter().map(|h| h.document).collect()
    }

    #[test]
    fn more_query_words_first() {
        let docs = [
            document(&["alpha"]),
            document(&["alpha beta"]),
            document(&["gamma"]),
        ];
        let hits = rank(&words("alpha beta"), &docs);
        assert_eq!(order(&hits), [1, 0]);
    }

    #[test]
    fn one_hit_per_document_at_its_best_passage() {
        let docs = [document(&[
            "alpha filler filler filler",
            "alpha",
            "alpha again",
        ])];
        let hits = rank(&words("alpha"), &docs);
        assert_eq!(
            hits,
            [Hit {
                document: 0,
                passage: 1
            }]
        );

        let equal = [document(&["alpha", "alpha"])];
        let hits = rank(&words("alpha"), &equal);
        assert_eq!(
            hits,
            [Hit {
                document: 0,
                passage: 0
            }]
        );
    }

    #[test]
    fn ties_go_to_the_lower_document_index() {
        let docs = [document(&["alpha"]), document(&["alpha"])];
        assert_eq!(order(&rank(&words("alpha"), &docs)), [0, 1]);
    }

    #[test]
    fn rarer_word_wins() {
        let docs = [
            document(&["alpha"]),
            document(&["beta"]),
            document(&["alpha"]),
            document(&["alpha"]),
        ];
        let hits = rank(&words("alpha beta"), &docs);
        assert_eq!(hits[0].document, 1);
    }

    #[test]
    fn denser_passage_wins() {
        let docs = [
            document(&["alpha filler filler filler"]),
            document(&["alpha alpha filler filler"]),
        ];
        assert_eq!(order(&rank(&words("alpha"), &docs)), [1, 0]);
    }

    #[test]
    fn passages_without_query_words_are_not_hits() {
        let docs = [document(&["alpha"]), document(&["beta"])];
        assert!(rank(&words("gamma"), &docs).is_empty());
        assert!(rank(&words("alpha"), &[]).is_empty());
        assert!(rank(&words("alpha"), &[Document { passages: vec![] }]).is_empty());
        assert!(rank(&[], &docs).is_empty());
    }

    #[test]
    fn repeated_query_words_count_once() {
        let docs = [
            document(&["alpha"]),
            document(&["beta"]),
            document(&["alpha gamma"]),
        ];
        let hits = rank(&words("alpha alpha alpha beta"), &docs);
        assert_eq!(hits[0].document, 1);
    }

    #[test]
    fn heading_path_word_matches() {
        let docs = [
            Document {
                passages: vec![Passage {
                    path: vec!["Embedder".into(), "Gotchas".into()],
                    line: 3,
                    text: "nothing here".into(),
                }],
            },
            document(&["unrelated"]),
        ];
        let hits = rank(&words("gotchas"), &docs);
        assert_eq!(
            hits,
            [Hit {
                document: 0,
                passage: 0
            }]
        );
    }
}
