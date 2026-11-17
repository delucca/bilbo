use bilbo_entries::tokens::Tokens;

pub struct Tokenizer {
    pub tokens: Box<dyn Tokens>,
    pub id: &'static str,
}
