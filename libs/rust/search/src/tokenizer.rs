use bilbo_kinds::tokens::Tokens;

/// The tokenizer the chunks are counted with, and what the index's manifest calls it.
pub struct Tokenizer {
    pub tokens: Box<dyn Tokens>,
    pub id: &'static str,
}
