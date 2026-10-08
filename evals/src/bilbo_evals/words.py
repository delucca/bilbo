"""Word folding as bilbo does it (src/shared/text.rs each_word), stopwords, stemming and distinctive tokens."""

from __future__ import annotations

import math
import unicodedata
from collections import Counter
from functools import lru_cache
from typing import Iterable

import Stemmer

# English and Portuguese stopwords of the archived retrieval harness (accent-folded).
STOPWORDS: frozenset[str] = frozenset("""
about above after again against also among another because been before being below between both cannot could does doing down during each
even every from further have having here into itself just more most much must never only other over same should some such than that their
them then there these they this those through under until very were what when where which while whom will with would your yours
isso esse essa este esta isto aqui como para pela pelo pelos pelas porque quando onde qual quais quem mais menos muito muita muitos muitas
sobre entre depois antes ainda assim cada mesmo mesma nosso nossa nossos nossas voce voces eles elas deles delas seja sido esta estao
sera seria tambem tudo todo toda todos todas outro outra outros outras qualquer foram fazer feito tinha temos isso uma umas uns
""".split())

# The letters of text.rs `base`: lowercase Latin-1 Supplement and Latin Extended-A letters to their base letters.
_BASE: dict[str, str] = {}
for _letters, _base in [
    ("àáâãäåāăą", "a"), ("æ", "ae"), ("çćĉċč", "c"), ("ðďđ", "d"), ("èéêëēĕėęě", "e"), ("ĝğġģ", "g"),
    ("ĥħ", "h"), ("ìíîïĩīĭįı", "i"), ("ĳ", "ij"), ("ĵ", "j"), ("ķĸ", "k"), ("ĺļľŀł", "l"), ("ñńņňŉŋ", "n"),
    ("òóôõöøōŏő", "o"), ("œ", "oe"), ("ŕŗř", "r"), ("śŝşšſ", "s"), ("ß", "ss"), ("ţťŧ", "t"), ("þ", "th"),
    ("ùúûüũūŭůűų", "u"), ("ŵ", "w"), ("ýÿŷ", "y"), ("źżž", "z"),
]:
    for _c in _letters:
        _BASE[_c] = _base

_STEMMERS = {"en": "english", "pt": "portuguese"}
MIN_LETTERS = 4
MAX_DF_SHARE = 0.02


def _is_mark(c: str) -> bool:
    return "̀" <= c <= "ͯ"


def words(text: str) -> list[str]:
    """Runs of alphanumeric characters, lowercased, marks removed, base letters, of 2 or more characters."""
    found: list[str] = []
    word: list[str] = []
    for c in text + " ":
        if _is_mark(c):
            continue
        if c.isalnum():
            for lower in c.lower():
                if not _is_mark(lower):
                    word.append(_BASE.get(lower, lower))
        else:
            if len("".join(word)) >= 2:
                found.append("".join(word))
            word.clear()
    return found


def fold(text: str) -> str:
    """Lowercased with accents removed."""
    decomposed = unicodedata.normalize("NFD", text)
    return "".join(ch for ch in decomposed if not unicodedata.combining(ch)).lower()


@lru_cache(maxsize=None)
def _stemmer(lang: str) -> Stemmer.Stemmer:
    if lang not in _STEMMERS:
        raise ValueError(f"no stemmer for language {lang!r}")
    return Stemmer.Stemmer(_STEMMERS[lang])


def stem(word: str, lang: str) -> str:
    return _stemmer(lang).stemWord(word)


def stems(text: str, lang: str) -> set[str]:
    return {stem(w, lang) for w in words(text)}


def doc_freq(docs: Iterable[tuple[str, str]]) -> Counter[str]:
    """Number of documents holding each stem; `docs` yields (text, lang)."""
    df: Counter[str] = Counter()
    for text, lang in docs:
        df.update(stems(text, lang))
    return df


def distinctive(text: str, lang: str, df: Counter[str], n_docs: int) -> set[str]:
    """Stems of the words of 4 or more letters, not stopwords, found in at most max(1, floor(2%)) documents."""
    limit = max(1, math.floor(MAX_DF_SHARE * n_docs))
    return {
        s
        for w in words(text)
        if len(w) >= MIN_LETTERS and w not in STOPWORDS
        for s in [stem(w, lang)]
        if df[s] <= limit
    }


def shared_distinctive(query: str, qlang: str, note: str, nlang: str, df: Counter[str], n_docs: int) -> set[str]:
    """Distinctive stems of the query, taken in both languages, that the note holds once stemmed in its own."""
    held = stems(note, nlang)
    found: set[str] = set()
    for lang in {qlang, nlang}:
        found |= distinctive(query, lang, df, n_docs) & held
    return found
