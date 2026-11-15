use std::sync::LazyLock;

use super::{Language, fold, word};

struct Table {
    words: Vec<String>,
    shared: Vec<String>,
}

pub(super) fn published() -> &'static [&'static str] {
    stop_words::lookup(Language::Portuguese.code()).expect("the crate holds the nltk lists")
}

fn table() -> &'static Table {
    static TABLE: LazyLock<Table> = LazyLock::new(|| {
        let mut words: Vec<String> = published()
            .iter()
            .map(|spelling| fold(spelling).into_owned())
            .filter(|folded| word(folded).is_some())
            .collect();
        words.sort();
        words.dedup();
        let english = stop_words::lookup("en").expect("the crate holds the nltk lists");
        let shared = words
            .iter()
            .filter(|each| english.contains(&each.as_str()))
            .cloned()
            .collect();
        Table { words, shared }
    });
    &TABLE
}

pub(super) fn leaves(word: &str) -> bool {
    table()
        .words
        .binary_search_by(|each| each.as_str().cmp(word))
        .is_ok()
}

pub(super) fn counts(word: &str) -> bool {
    leaves(word) && !table().shared.iter().any(|each| each == word)
}

#[cfg(test)]
mod tests {
    use super::{published, table};

    #[test]
    fn the_published_list_is_the_207_words_the_drop_was_measured_on() {
        let held: Vec<&str> = "\
a ao aos aquela aquelas aquele aqueles aquilo as até com como da das de dela delas dele
deles depois do dos e ela elas ele eles em entre era eram essa essas esse esses esta
estamos estar estas estava estavam este esteja estejam estejamos estes esteve estive
estivemos estiver estivera estiveram estiverem estivermos estivesse estivessem
estivéramos estivéssemos estou está estávamos estão eu foi fomos for fora foram forem
formos fosse fossem fui fôramos fôssemos haja hajam hajamos havemos haver hei houve
houvemos houver houvera houveram houverei houverem houveremos houveria houveriam
houvermos houverá houverão houveríamos houvesse houvessem houvéramos houvéssemos há hão
isso isto já lhe lhes mais mas me mesmo meu meus minha minhas muito na nas nem no nos
nossa nossas nosso nossos num numa não nós o os ou para pela pelas pelo pelos por qual
quando que quem se seja sejam sejamos sem ser serei seremos seria seriam será serão
seríamos seu seus somos sou sua suas são só também te tem temos tenha tenham tenhamos
tenho terei teremos teria teriam terá terão teríamos teu teus teve tinha tinham tive
tivemos tiver tivera tiveram tiverem tivermos tivesse tivessem tivéramos tivéssemos tu
tua tuas tém tínhamos um uma você vocês vos à às é éramos
"
        .split_whitespace()
        .collect();
        let mut found = published().to_vec();
        found.sort_unstable();
        assert_eq!(held.len(), 207);
        assert_eq!(found, held);
    }

    #[test]
    fn the_table_has_194_words_and_the_english_three_are_do_me_and_so() {
        let table = table();
        assert_eq!(table.words.len(), 194);
        assert!(table.words.windows(2).all(|pair| pair[0] < pair[1]));
        for word in &table.words {
            assert!(word.chars().all(|c| c.is_ascii_lowercase()), "{word}");
        }
        assert!(!table.words.iter().any(|word| word == "as"));
        assert_eq!(table.shared, ["do", "me", "so"]);
    }
}
