"""The passage port follows the tests of src/search/rank.rs, and an alteration of it changes bilbo's inputs."""

from __future__ import annotations

import pytest

from bilbo_evals import passages as ps
from bilbo_evals.passages import Passage, embed_input, split_body


def texts(found):
    return [(p.heading_path, p.line, p.text) for p in found]


def test_input_carries_the_heading_path():
    p = Passage(["Note store", "Layout"], "One flat folder.", 1)
    assert embed_input(p) == "Note store > Layout\nOne flat folder."


def test_input_is_cut_on_a_char_boundary():
    text = "ã" * 2000
    cut = embed_input(Passage(["Ta"], text, 1))
    uncut = f"Ta\n{text}"
    assert ps.INPUT_BYTES - 1 <= len(cut.encode()) <= ps.INPUT_BYTES
    assert uncut.startswith(cut)


def test_empty_text_has_no_input():
    assert embed_input(Passage(["T"], "", 1)) is None


def test_nested_heading_path():
    lines = ["# Embedder", "", "## Gotchas", "", "### Two slots", "", "Use two slots."]
    last = split_body(lines, 6, "fallback")[-1]
    assert last.heading_path == ["Embedder", "Gotchas", "Two slots"]
    assert last.line == 10
    assert last.text == "Use two slots."


@pytest.mark.parametrize("fence", ["```", "~~~"])
def test_fenced_heading_opens_no_passage(fence):
    lines = ["# T", "## Setup", "", fence, "# install deps", fence, "## After"]
    found = split_body(lines, 1, "fb")
    assert [" > ".join(p.heading_path) for p in found] == ["T", "T > Setup", "T > After"]
    assert "# install deps" in found[1].text


def test_unclosed_fence_hides_the_rest():
    found = split_body(["## Setup", "~~~", "# hidden", "## also hidden"], 1, "fb")
    assert len(found) == 1
    assert found[0].heading_path == ["fb", "Setup"]
    assert "also hidden" in found[0].text


def test_a_hash_tag_line_is_text():
    assert texts(split_body(["#tag", "text"], 1, "fb")) == [(["fb"], 1, "#tag\ntext")]


def test_closing_hashes_and_spacing_are_dropped_from_a_heading():
    found = split_body(["# A   b ##", "x", "## C#", "y"], 1, "fb")
    assert [p.heading_path for p in found] == [["A b"], ["A b", "C#"]]


def test_preamble_and_title():
    lines = ["", "intro text", "more", "", "# T", "x", "# Two", "y"]
    assert texts(split_body(lines, 1, "fb")) == [
        (["T"], 2, "intro text\nmore"),
        (["T"], 5, "x"),
        (["T", "Two"], 7, "y"),
    ]
    assert texts(split_body(["", "# T"], 1, "fb")) == [(["T"], 2, "")]
    assert texts(split_body(["## Early", "a", "# T"], 1, "fb")) == [(["T", "Early"], 1, "a"), (["T"], 3, "")]


def test_no_title_uses_the_fallback():
    assert texts(split_body(["## A", "text"], 1, "plan-x")) == [(["plan-x", "A"], 1, "text")]
    assert texts(split_body(["just text"], 1, "plan-x")) == [(["plan-x"], 1, "just text")]


def test_lines_follow_first_line():
    found = split_body(["a", "b", "c", "## H", "d"], 5, "fb")
    assert (found[0].line, found[1].line) == (5, 8)


def test_exactly_4000_bytes_is_one_part():
    assert len(split_body(["## S", "a" * ps.PART_BYTES], 1, "fb")) == 1
    found = split_body(["## S", "a" * (ps.PART_BYTES + 1)], 1, "fb")
    assert [len(p.text) for p in found] == [ps.PART_BYTES, 1]


def test_long_section_splits_at_blank_lines():
    paragraphs = [f"p{i}: " + "w" * 895 for i in range(10)]
    lines = ["## S"]
    for p in paragraphs:
        lines += ["", p]
    found = split_body(lines, 1, "fb")
    assert len(found) > 1 and found[0].line == 1
    for part in found:
        assert len(part.text.encode()) <= ps.PART_BYTES
        assert part.heading_path == ["fb", "S"]
        assert part.text.startswith("p")
    for i, p in enumerate(paragraphs):
        assert sum(p in part.text for part in found) == 1, i
    for part in found[1:]:
        i = int(part.text[1 : part.text.index(":")])
        assert part.line == 3 + 2 * i


def test_long_paragraph_cuts_on_a_char_boundary():
    straddle = "a" * 3999 + "ã" * 500
    found = split_body(["## S", straddle], 1, "fb")
    assert len(found) == 2
    assert len(found[0].text.encode()) == 3999
    assert found[0].text + found[1].text == straddle

    tail = "ã" * 2500
    found = split_body(["## S", "line one", tail], 1, "fb")
    assert [p.line for p in found] == [1, 3]
    assert found[0].text + found[1].text == f"line one\n{tail}"


def test_a_cut_before_a_line_break_starts_on_the_next_line():
    found = split_body(["## S", "a" * ps.PART_BYTES, "bbb"], 1, "fb")
    assert len(found) == 2
    assert found[1].text == "\nbbb"
    assert found[1].line == 3


def test_passages_of_a_note_file_skip_the_frontmatter():
    note = "---\nid: 01J0000000000000000000000A\ncreated: 2025-06-02T10:14-03:00\n---\n\n# Title\n\nbody\n"
    found = ps.passages(note, "plan-x")
    assert texts(found) == [(["Title"], 6, "body")]


def test_a_note_without_frontmatter_starts_on_line_one():
    assert texts(ps.passages("# T\nx\n", "plan-x")) == [(["T"], 1, "x")]


def test_inputs_cover_valid_notes_only(tmp_path):
    notes = tmp_path / "notes"
    notes.mkdir()
    body = "---\nid: 01J0000000000000000000000A\ncreated: 2025-06-02T10:14-03:00\n---\n\n# T\n\n## S\n\ntext\n"
    (notes / "plan-one.md").write_text(body)
    (notes / "notakind-two.md").write_text(body)
    (notes / "plan-Bad.md").write_text(body)
    (notes / ".plan-hidden.md").write_text(body)
    (notes / "plan-empty.md").write_text("---\nid: 01J0000000000000000000000B\ncreated: 2025-06-02T10:14-03:00\n---\n\n# Only a title\n")
    assert ps.inputs(tmp_path) == ["T > S\ntext"]
