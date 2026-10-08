"""words.py: bilbo's word folding, stemming and distinctive tokens."""

from collections import Counter

from bilbo_evals import words as w


def test_words_fold_accents_and_case():
    assert w.words("Ação, MIGRAÇÃO e São Paulo!") == ["acao", "migracao", "sao", "paulo"]


def test_words_drop_one_letter_runs_and_split_on_punctuation():
    assert w.words("a b SQLITE_BUSY: x1") == ["sqlite", "busy", "x1"]


def test_words_drop_combining_marks_and_expand_ligatures():
    assert w.words("café straße œuvre") == ["cafe", "strasse", "oeuvre"]


def test_fold():
    assert w.fold("Ação São") == "acao sao"


def test_stem_by_language():
    assert w.stem("caches", "en") == "cach"
    assert w.stem("notas", "pt") == w.stem("nota", "pt")


def test_doc_freq_counts_documents_not_occurrences():
    df = w.doc_freq([("cache cache cache", "en"), ("the cache", "en"), ("other thing", "en")])
    assert df["cach"] == 2
    assert df["thing"] == 1


def test_distinctive_applies_the_two_percent_rule():
    n = 100
    df = Counter({w.stem("checkpoint", "en"): 1, w.stem("database", "en"): 3, w.stem("fsync", "en"): 1})
    got = w.distinctive("checkpoint database fsync wal", "en", df, n)
    assert w.stem("checkpoint", "en") in got
    assert w.stem("fsync", "en") in got
    assert w.stem("database", "en") not in got  # 3 of 100 is over the limit of 2
    assert w.stem("wal", "en") not in got  # under four letters


def test_distinctive_skips_stopwords_and_short_words():
    got = w.distinctive("about this when cat", "en", Counter(), 100)
    assert got == set()


def test_distinctive_skips_portuguese_stopwords_after_folding():
    assert w.distinctive("você também sobre", "pt", Counter(), 100) == set()


def test_shared_distinctive_between_a_query_and_a_note():
    df = Counter()
    shared = w.shared_distinctive(
        "when does the checkpoint run after fsync", "en", "A checkpoint calls fsync twice.", "en", df, 100
    )
    assert shared == {w.stem("checkpoint", "en"), w.stem("fsync", "en")}


def test_shared_distinctive_ignores_common_stems():
    df = Counter({w.stem("checkpoint", "en"): 30})
    assert w.shared_distinctive("checkpoint", "en", "a checkpoint", "en", df, 100) == set()


def test_shared_distinctive_across_languages_matches_identical_tokens():
    shared = w.shared_distinctive("when does fsync happen", "en", "O fsync roda depois da escrita.", "pt", Counter(), 100)
    assert shared == {w.stem("fsync", "pt")}


def test_a_small_corpus_keeps_a_floor_of_one_document():
    df = Counter({w.stem("checkpoint", "en"): 1, w.stem("database", "en"): 2})
    got = w.distinctive("checkpoint database", "en", df, 12)
    assert got == {w.stem("checkpoint", "en")}
