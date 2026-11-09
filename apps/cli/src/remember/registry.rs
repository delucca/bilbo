use bilbo_kinds::kind::Kind;

pub fn kinds() -> Vec<Box<dyn Kind>> {
    vec![
        Box::new(bilbo_markdown::Markdown),
        Box::new(bilbo_session::Session),
    ]
}

#[cfg(test)]
mod tests {
    use super::kinds;

    #[test]
    fn every_kind_in_the_registry_has_its_own_lowercase_name() {
        let names: Vec<&str> = kinds().iter().map(|kind| kind.name()).collect();
        assert!(!names.is_empty());
        for name in &names {
            assert!(
                !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase()),
                "{name:?}"
            );
        }
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique, {
            let mut sorted = names;
            sorted.sort_unstable();
            sorted
        });
    }
}
