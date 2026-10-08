## common
Item: $item

You write the text a person types into the search box of their own notes. You have not seen any note: you work only from what is shown below.
Write exactly one query, 3 to 20 words, as typed (a question or a few keywords). Write it in $lang_name and set "lang" to "$lang".
Do not mention that you were given facts, notes or a table. Reply with JSON only: {"query": "...", "lang": "$lang", "hop_link": "..."}. Leave "hop_link" empty unless the task below asks for it.

## known-item
Project: $project_name ($project_summary)
Component: $component
Title of the note the person is looking for: $title
Fact the note holds: $statement

Ask for this fact the way someone who remembers the subject would, naming it by its own name, in the words the note would use.

## paraphrase
Project: $project_name ($project_summary)
Component: $component
Names the project uses for its components (alias table):
$aliases
Fact: $statement

Ask for this fact with other words than the fact uses: same intent, different vocabulary. If you name the component, use its current name "$component", never an alias. Do not copy identifiers, file paths, error strings or other rare words from the fact.

## pt-en
Project: $project_name ($project_summary)
Component: $component
Names the project uses for its components (alias table):
$aliases
Fact (stated in English): $statement

The note is written in $fact_lang_name and the fact above is stated in English; the query must be in $lang_name. Ask for this fact in $lang_name, as a native speaker would, without translating word by word. If you name the component, use its current name "$component", never an alias. Keep a product, tool or error name only if a person would not translate it.

## alias
Project: $project_name ($project_summary)
Component: $component, also known as "$alias" ($alias_type)
Names the project uses for its components (alias table):
$aliases
Fact about the component: $statement

Ask for this fact calling the component "$alias" and never "$component". Do not copy identifiers, file paths or error strings from the fact.

## supersession
Project: $project_name ($project_summary)
Component: $component
Current fact: $statement
Earlier fact it replaced: $older

Ask for the current value, as someone who does not know it changed. Do not state either value and do not hint that something was replaced.

## multi-hop
Project: $project_name ($project_summary)
Facts, each kept in a different note (not in chain order):
$facts

Write one question that needs a real chain through these facts: the answer found in one note is the very thing you must name to ask about the other. A person who does not know the first answer could not even phrase the second question.
Never join two independent questions with "and" or "also". Never invent a causal or temporal link that the facts do not state. Do not copy identifiers or error strings.
Set "hop_link" to the short phrase that carries the answer of the first hop into the second. If these facts have no such dependency, set "hop_link" to "NONE" and write your best single question anyway.

## kind-filter
Project: $project_name ($project_summary)
Component: $component
The person is looking only among notes of kind "$kind".
Fact the note holds: $statement

Ask for what such a note says about the subject. Name the subject plainly; mention the kind only if it comes naturally.

## no-answer
Project: $project_name ($project_summary)
Technologies: $technologies
Components: $components
Angle: $angle
Facts the notes DO answer (the new question must not be answered by any of them):
$facts

Write a plausible question a team member could ask about this project, from the angle above, that none of those facts answers.

## library
Section of a documentation page, heading path: $heading
Section text:
$text

Write the query a person would type to find this section of the documentation. Describe what they want to know; do not copy a sentence from the section.

## avoid
Earlier attempts shared these words with the target, so do not use them or their forms: $tokens
