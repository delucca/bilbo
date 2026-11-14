use bilbo_kinds::tokens::Tokens;

use super::hit::Hit;

pub fn pack<E>(
    mut hits: impl Iterator<Item = Result<Hit, E>>,
    tokens: &dyn Tokens,
    shown: &dyn Fn(&Hit) -> String,
    keep: &mut dyn FnMut(&Hit) -> bool,
    budget: usize,
) -> Result<Vec<Hit>, E> {
    let mut packed = Vec::new();
    let mut left = budget;
    while left > 0 {
        let Some(next) = hits.next() else { break };
        let mut hit = next?;
        let cost = tokens.count(&shown(&hit));
        if cost <= left {
            if keep(&hit) {
                left -= cost;
                packed.push(hit);
            }
            continue;
        }
        if !cut(&mut hit, tokens, shown, left) {
            break;
        }
        if keep(&hit) {
            packed.push(hit);
            break;
        }
    }
    Ok(packed)
}

fn cut(hit: &mut Hit, tokens: &dyn Tokens, shown: &dyn Fn(&Hit) -> String, left: usize) -> bool {
    hit.cut = true;
    for budget in (1..=left).rev() {
        let at = tokens.prefix(&hit.text, budget);
        if at == 0 {
            return false;
        }
        if at == hit.text.len() {
            continue;
        }
        hit.text.truncate(at);
        if tokens.count(&shown(hit)) <= left {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::convert::Infallible;

    use bilbo_kinds::tokens::Tokens;
    use bilbo_store::entries::{Generation, RecordRange};
    use bilbo_store::id::Id;

    use super::{Hit, pack};
    use crate::index::Extent;
    use crate::lookup::Found;

    struct Bytes;

    impl Tokens for Bytes {
        fn ends(&self, text: &str) -> Vec<usize> {
            (1..=text.len()).collect()
        }
    }

    fn hit(text: &str) -> Hit {
        Hit {
            found: Found {
                entry: Id::from_bytes([1; 16]),
                kind: "note".to_owned(),
                extent: Extent {
                    generation: Generation(1),
                    records: RecordRange { first: 1, count: 1 },
                    part: None,
                    date: 0,
                },
                digest: 0,
            },
            preface: None,
            text: text.to_owned(),
            cut: false,
            shared_short: false,
        }
    }

    fn shown(hit: &Hit) -> String {
        format!("{}{}", if hit.cut { "===c" } else { "===" }, hit.text)
    }

    fn texts(hits: &[Hit]) -> Vec<&str> {
        hits.iter().map(|hit| hit.text.as_str()).collect()
    }

    fn ok(texts: &[&str]) -> impl Iterator<Item = Result<Hit, Infallible>> {
        texts
            .iter()
            .map(|text| Ok(hit(text)))
            .collect::<Vec<_>>()
            .into_iter()
    }

    fn packed(texts_in: &[&str], budget: usize) -> Vec<Hit> {
        let Ok(hits) = pack(ok(texts_in), &Bytes, &shown, &mut |_| true, budget);
        hits
    }

    #[test]
    fn hits_are_taken_whole_while_their_printed_forms_fit() {
        let all = ["aaaa", "bbbb", "cccc"];
        for budget in [21, 22] {
            assert_eq!(texts(&packed(&all, budget)), all);
            assert!(packed(&all, budget).iter().all(|hit| !hit.cut));
        }
        assert_eq!(texts(&packed(&all, 14)), ["aaaa", "bbbb"]);
        assert!(packed(&all, 14).iter().all(|hit| !hit.cut));
        assert_eq!(texts(&packed(&all, 7)), ["aaaa"]);
        assert!(!packed(&all, 7)[0].cut);
        let one_under = packed(&all, 20);
        assert_eq!(texts(&one_under), ["aaaa", "bbbb", "cc"]);
        assert!(!one_under[1].cut && one_under[2].cut);
    }

    #[test]
    fn the_hit_that_does_not_fit_is_cut_at_a_token_and_marked() {
        let hits = packed(&["aaaa", "bbbbbbbbbb", "cccc"], 15);
        assert_eq!(texts(&hits), ["aaaa", "bbbb"]);
        assert!(!hits[0].cut);
        assert!(hits[1].cut);
        let cost: usize = hits.iter().map(|hit| Bytes.count(&shown(hit))).sum();
        assert_eq!(cost, 15);
        let longer = Hit {
            text: "bbbbb".to_owned(),
            cut: true,
            ..hit("")
        };
        assert!(
            7 + Bytes.count(&shown(&longer)) > 15,
            "one more byte would not fit"
        );
    }

    #[test]
    fn a_hit_with_no_room_for_one_token_of_text_is_dropped() {
        let hits = packed(&["aaaa", "bbbb"], 11);
        assert_eq!(texts(&hits), ["aaaa"]);
        assert!(!hits[0].cut);
        assert!(packed(&["bbbb"], 4).is_empty());
        assert!(packed(&["bbbb"], 3).is_empty());
        assert!(packed(&[""], 2).is_empty());
    }

    #[test]
    fn nothing_follows_a_cut_or_dropped_hit() {
        assert_eq!(
            texts(&packed(&["aaaa", "bbbbbbbb", "c"], 12)),
            ["aaaa", "b"]
        );
        assert_eq!(texts(&packed(&["aaaa", "bbbbbbbb", "c"], 11)), ["aaaa"]);
    }

    fn counted<'a>(
        pulled: &'a Cell<usize>,
        texts: &'a [&'a str],
    ) -> impl Iterator<Item = Result<Hit, Infallible>> + 'a {
        texts.iter().map(move |text| {
            pulled.set(pulled.get() + 1);
            Ok(hit(text))
        })
    }

    #[test]
    fn no_hit_is_read_after_the_budget_is_full() {
        let all = ["aaaa", "bbbb", "cccc"];
        let pulled = Cell::new(0);
        let Ok(whole) = pack(counted(&pulled, &all), &Bytes, &shown, &mut |_| true, 14);
        assert_eq!(whole.len(), 2);
        assert_eq!(pulled.get(), 2);
        pulled.set(0);
        let Ok(cut) = pack(counted(&pulled, &all), &Bytes, &shown, &mut |_| true, 13);
        assert_eq!(texts(&cut), ["aaaa", "bb"]);
        assert_eq!(pulled.get(), 2);
        pulled.set(0);
        let Ok(none) = pack(counted(&pulled, &all), &Bytes, &shown, &mut |_| true, 0);
        assert!(none.is_empty());
        assert_eq!(pulled.get(), 0);
    }

    #[test]
    fn a_hit_keep_refuses_is_not_packed_and_costs_nothing() {
        let mut seen = 0;
        let mut keep = |_: &Hit| {
            seen += 1;
            seen != 2
        };
        let Ok(hits) = pack(ok(&["aaaa", "bbbb", "cccc"]), &Bytes, &shown, &mut keep, 14);
        assert_eq!(texts(&hits), ["aaaa", "cccc"]);
        assert!(hits.iter().all(|hit| !hit.cut));
    }

    #[test]
    fn keep_sees_a_cut_hit_as_it_is_printed() {
        let mut asked = Vec::new();
        let mut keep = |hit: &Hit| {
            asked.push((hit.text.clone(), hit.cut));
            true
        };
        let Ok(_) = pack(ok(&["aaaa", "bbbbbbbbbb"]), &Bytes, &shown, &mut keep, 15);
        assert_eq!(
            asked,
            [("aaaa".to_owned(), false), ("bbbb".to_owned(), true)]
        );
    }

    #[test]
    fn a_refused_cut_hit_does_not_end_the_packing() {
        let mut keep = |hit: &Hit| !(hit.cut && hit.text.starts_with('b'));
        let Ok(hits) = pack(
            ok(&["aaaa", "bbbbbbbbbb", "cccccccccc"]),
            &Bytes,
            &shown,
            &mut keep,
            15,
        );
        assert_eq!(texts(&hits), ["aaaa", "cccc"]);
        assert!(hits[1].cut);
    }

    #[test]
    fn a_cut_hit_that_is_kept_ends_the_packing() {
        let pulled = Cell::new(0);
        let all = ["aaaa", "bbbbbbbbbb", "cccc", "dddd"];
        let Ok(hits) = pack(counted(&pulled, &all), &Bytes, &shown, &mut |_| true, 15);
        assert_eq!(texts(&hits), ["aaaa", "bbbb"]);
        assert_eq!(pulled.get(), 2);
    }

    #[test]
    fn an_error_from_the_iterator_is_returned() {
        let hits = vec![Ok(hit("aaaa")), Err("gone"), Ok(hit("bbbb"))];
        assert_eq!(
            pack(hits.into_iter(), &Bytes, &shown, &mut |_| true, 100).expect_err("an error"),
            "gone"
        );
    }

    #[test]
    fn a_multi_byte_character_is_never_split_by_a_cut() {
        for budget in 4..16 {
            let hits = packed(&["ééééééé"], budget);
            for hit in &hits {
                assert!(
                    hit.text.chars().all(|c| c == 'é'),
                    "{budget}: {:?}",
                    hit.text
                );
                assert!(hit.cut);
                assert_eq!(hit.text.len() % 2, 0);
            }
            let cost: usize = hits.iter().map(|hit| Bytes.count(&shown(hit))).sum();
            assert!(cost <= budget, "{budget}");
        }
        let hits = packed(&["ééééééé"], 9);
        assert_eq!(texts(&hits), ["éé"]);
    }
}
