use bilbo_kinds::tokens::Tokens;
use std::collections::HashMap;
use tokenizers::{NormalizerWrapper, Tokenizer};

/// `tokenizer.json` of Qwen/Qwen3-Embedding-0.6B at the revision `ID` names, packed with zstd.
static PACKED: &[u8] = include_bytes!("../tokenizer.json.zst");
/// The file's bytes once unpacked.
const SIZE: usize = 11_423_705;

/// The model, the revision of its repository and the SHA-256 of `tokenizer.json`, each cut to 12
/// characters, then how the file is read. It names the tokenizer in the index's manifest.
pub const ID: &str = "qwen3-embedding-0.6b/97b0c614be4d/def76fb08697/raw";

pub struct Qwen {
    tokenizer: Tokenizer,
    /// The bytes of each added token. Every other token's text has one character for each byte,
    /// which is how a byte-level vocabulary spells a token that ends inside a character.
    added: HashMap<u32, usize>,
}

#[derive(Debug)]
pub struct Unloaded(pub String);

impl std::fmt::Display for Unloaded {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for Unloaded {}

impl Qwen {
    pub fn load() -> Result<Qwen, Unloaded> {
        let file =
            zstd::bulk::decompress(PACKED, SIZE).map_err(|error| Unloaded(error.to_string()))?;
        let mut tokenizer =
            Tokenizer::from_bytes(file).map_err(|error| Unloaded(error.to_string()))?;
        // llama.cpp reads the text as it is; the file asks for NFC.
        tokenizer
            .with_normalizer(None::<NormalizerWrapper>)
            .map_err(|error| Unloaded(error.to_string()))?;
        let added = tokenizer
            .get_added_tokens_decoder()
            .into_iter()
            .map(|(id, token)| (id, token.content.len()))
            .collect();
        Ok(Qwen { tokenizer, added })
    }
}

impl Tokens for Qwen {
    fn ends(&self, text: &str) -> Vec<usize> {
        let Ok(encoding) = self.tokenizer.encode(text, false) else {
            return Vec::new();
        };
        let mut at = 0;
        let ends: Vec<usize> = encoding
            .get_ids()
            .iter()
            .zip(encoding.get_tokens())
            .map(|(id, token)| {
                at += self
                    .added
                    .get(id)
                    .copied()
                    .unwrap_or_else(|| token.chars().count());
                at
            })
            .collect();
        if at == text.len() { ends } else { Vec::new() }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::LazyLock;

    use bilbo_kinds::tokens::Tokens;

    use super::{ID, PACKED, Qwen, SIZE, Unloaded};

    static QWEN: LazyLock<Qwen> = LazyLock::new(|| Qwen::load().expect("the tokenizer loads"));

    fn fnv(data: &[u8]) -> u64 {
        data.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
        })
    }

    #[test]
    fn a_tokenizer_that_cannot_load_says_why() {
        let error: Box<dyn std::error::Error> = Box::new(Unloaded("bad zstd frame".to_string()));
        assert_eq!(error.to_string(), "bad zstd frame");
    }

    #[test]
    fn the_packed_file_is_the_pinned_tokenizer_json() {
        let file = zstd::bulk::decompress(PACKED, SIZE + 1).expect("zstd");
        assert_eq!(file.len(), SIZE);
        assert_eq!(fnv(&file), 0x40d6_3f1a_57fd_07af, "{:#018x}", fnv(&file));
        assert!(ID.starts_with("qwen3-embedding-0.6b/") && ID.ends_with("/raw"));
    }

    #[test]
    fn the_ends_rise_to_the_end_of_the_text_one_for_each_token() {
        for (text, tokens) in [
            ("", 0),
            ("a", 1),
            ("Hello, world!", 4),
            ("user: Can you check if Entire is enabled?", 10),
            ("\n\n", 1),
        ] {
            let ends = QWEN.ends(text);
            assert_eq!(ends.len(), tokens, "{text:?}: {ends:?}");
            assert_eq!(QWEN.count(text), tokens);
            assert_eq!(ends.last().copied().unwrap_or(0), text.len());
            assert!(ends.windows(2).all(|pair| pair[0] < pair[1]), "{ends:?}");
        }
    }

    #[test]
    fn a_text_is_read_as_it_is_and_never_normalized() {
        // NFC would join the letter and its mark into one character of two bytes, not three.
        for text in ["cafe\u{301}", "A\u{30a}ngstro\u{308}m", "\u{1100}\u{1161}"] {
            let ends = QWEN.ends(text);
            assert_eq!(ends.last().copied(), Some(text.len()), "{text:?}");
        }
    }

    #[test]
    fn only_an_empty_text_has_no_token() {
        assert!(QWEN.ends("").is_empty());
        for text in [
            "a",
            "Hello, world!",
            "user: Can you check if Entire is enabled?",
            "\n\n",
            "cafe\u{301}",
            "\u{1f9d1}\u{200d}\u{1f4bb} \u{10ffff}\u{e000}\u{fffd}",
            "a<|endoftext|>b<|im_start|>",
        ] {
            assert!(!QWEN.ends(text).is_empty(), "{text:?}");
        }
    }

    #[test]
    fn a_token_can_end_inside_a_character() {
        let text = "\u{1f9d1}\u{200d}\u{1f4bb} \u{10ffff}\u{e000}\u{fffd}";
        let ends = QWEN.ends(text);
        assert_eq!(ends.last().copied(), Some(text.len()));
        assert!(
            ends.iter().any(|end| !text.is_char_boundary(*end)),
            "{ends:?}"
        );
    }

    #[test]
    fn a_special_token_written_out_in_a_text_is_one_token_of_all_its_bytes() {
        let text = "a<|endoftext|>b<|im_start|>";
        assert_eq!(QWEN.ends(text), [1, 14, 15, 27]);
    }

    #[test]
    fn a_prefix_within_a_budget_ends_on_a_character() {
        let text = "né 五 🙂 and more words after";
        for budget in 0..12 {
            let prefix = QWEN.prefix(text, budget);
            assert!(text.is_char_boundary(prefix), "{budget}");
            assert!(prefix <= text.len());
        }
        assert_eq!(QWEN.prefix(text, 0), 0);
        assert_eq!(QWEN.prefix(text, 1000), text.len());
    }
}
