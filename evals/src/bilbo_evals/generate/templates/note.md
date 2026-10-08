## common
You are writing one working note that an engineer or a coding agent keeps about a project, for a synthetic benchmark. The project is fictional; its technologies are real. You cannot run commands or read files; write the note from what is given below alone.

Style guide, from the skill that agents use to write such notes (sha256 $style_sha). You cannot run its commands: take from it only how to write the body of a note, which is its step 7.
<style-guide>
$style_guide
</style-guide>

Project: $project_name. $project_summary
Technologies: $technologies
Component this note is about: $component
Note kind: $kind
Language of the note: $language

$task

Shape:
- Write about $chars characters in all, with about $headings headings of level 2 (`##`) or deeper.
$code_line
$wiki_line
- The body starts with prose, not with a heading, and has no title line and no front matter: the title is a separate field.
- Write as a person or an agent writes in a hurry: plain, concrete, no marketing words, no summary of what you were asked to do.
- Never mention a brief, instructions, a prompt or a manifest, and never say the note was generated or written from anything: write as someone who knows the project first-hand.
$extra
$feedback
Answer with the JSON object {"title": "...", "body": "..."}, where the body is Markdown.

## gold
Record the following facts. Each one comes with strings that must appear in the note exactly as written, character for character, such as an identifier, a number, a path, a port, a command or an error message; keep them in the original spelling even when the note is in another language.

$facts

Rules:
- Name the component exactly as `$component`, never by another name, unless a fact below gives another name on purpose.
- State each fact so that a reader of this note alone could answer a question about it.
- Do not add any other exact number, path, port, identifier, command or error message that is not in the facts; keep every other detail general.

## filler
Write a general working note: $activity.
Rules:
- Name the component exactly as `$component`.
- Do not state any exact number, version, path, port, identifier, command, error message or date; keep everything general.
- Do not record a decision, a requirement or a result with a specific value.

## duplicate
Write a loose, partial note on the same subject as an existing note about "$subject" (component `$component`, kind $kind), as an agent would write a second note on the same thing without checking the first.
Rules:
- Name the component exactly as `$component`.
- Cover the same subject in similar words, but do not state any exact number, version, path, port, identifier, command, error message or date; say things like "the usual value" or "the configured limit" instead.
- Do not repeat a specific value you may know; if you would need one, leave it out.
