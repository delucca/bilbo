use bilbo_store::id::Id;

pub fn line(parent: Id, text: &str, at: Option<u64>, sources: &[(u64, u64)]) -> Vec<u8> {
    let at = at.map_or_else(String::new, |at| format!(",\"at\":{at}"));
    let from = if sources.is_empty() {
        String::new()
    } else {
        let each: Vec<String> = sources
            .iter()
            .map(|(first, last)| {
                format!("{{\"entry\":\"{parent}\",\"generation\":1,\"records\":[{first},{last}]}}")
            })
            .collect();
        format!(",\"from\":[{}]", each.join(","))
    };
    format!("{{\"text\":\"{text}\"{at}{from}}}\n").into_bytes()
}
