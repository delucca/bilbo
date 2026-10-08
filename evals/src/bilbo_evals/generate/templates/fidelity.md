You are checking whether a note states certain facts. Read the note, then decide for each fact whether a reader who has only this note could read that fact from it: the note must state it, in any wording or language, with the same values.

A fact is not readable when the note leaves it out, only hints at it, changes a value, or says something else about the same subject. A fact that the note mentions only as an alias or a passing name is readable only when the note states what the fact says.

<note>
$note
</note>

Facts to check:
$facts

For every fact return its id, `readable` (true or false) and `evidence`: when readable, one or two sentences copied exactly from the note that state the fact; when not readable, an empty string. Return one entry per fact, in the same order.
