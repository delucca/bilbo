use bilbo_derive::answer::blocks;
use bilbo_derive::instruction::shipped;

fn ranges(paragraph: &str) -> Vec<&str> {
    paragraph
        .split_whitespace()
        .filter(|word| {
            let mut chars = word.chars();
            chars.next() == Some('r') && chars.next().is_some_and(|c| c.is_ascii_digit())
        })
        .map(|word| word.trim_end_matches(['.', ',', ':', ';']))
        .collect()
}

#[test]
fn every_shipped_instruction_shows_the_form_the_grammar_reads() {
    for (kind, text) in shipped() {
        let forms: Vec<&str> = text
            .split("\n\n")
            .filter(|paragraph| paragraph.starts_with("Form."))
            .collect();
        assert_eq!(
            forms.len(),
            1,
            "{kind}: the paragraph that starts with Form."
        );
        let taken = ranges(forms[0]);
        assert!(taken.len() >= 2, "{kind}: {taken:?}");
        for range in &taken {
            let found = blocks(&format!("{range}\na line of text"));
            assert_eq!(found.blocks.len(), 1, "{kind}: {range}");
            assert_eq!(found.stray, 0, "{kind}: {range}");
        }
        let answer = format!(
            "{}\na line of text\n\n{}\na line of text",
            taken[0], taken[1]
        );
        let found = blocks(&answer);
        assert_eq!(found.blocks.len(), 2, "{kind}");
        assert_eq!(found.stray, 0, "{kind}");
        let (example, _) = text
            .split_once("An example of the form")
            .and_then(|(_, rest)| rest.split_once("<example>\n"))
            .and_then(|(_, rest)| rest.split_once("</example>"))
            .expect("an example");
        let shown = blocks(example);
        let places: Vec<(u64, u64)> = shown
            .blocks
            .iter()
            .map(|block| (block.first, block.last))
            .collect();
        assert_eq!(places, [(12, 14), (15, 15)], "{kind}");
        assert_eq!(shown.stray, 0, "{kind}");
    }
}
