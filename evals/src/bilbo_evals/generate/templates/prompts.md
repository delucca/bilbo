## common
Batch: $item

You write messages that a developer sends to a coding agent. A tool decides from each message alone whether to attach notes from the developer's memory; you are producing test messages for it.
Write the messages in $lang_name. Reply with JSON only: {"prompts": ["...", ...]} with exactly $n strings, each different, none numbered.

## positive
Project: $project_name ($project_summary)
Below are $n notes (numbered), each described by the facts it holds. For each one, in order, write a task request the developer would send while working on the project, one that this note bears on. The request must not quote the fact; it asks for work (fix, add, check, explain, change) in the developer's own voice, 1 to 3 sentences.
$notes

## noise
Write short conversational turns that carry no topic of their own: acknowledgements, "go on", "yes do that", "ok thanks", "try again", a one-line reaction. 1 to 8 words each. Vary tone and wording across the batch. Style hint: $hint

## off-topic
Write task requests to a coding agent that have nothing to do with the project below: other languages, other domains, general questions, small scripts, writing help, shell one-liners. 1 to 3 sentences each. Do not mention any software project of the sender's.
Theme hint: $hint

## near-miss
Project: $project_name ($project_summary)
Technologies: $technologies
Components: $components
Write task requests that sound like work on this project, using its technologies and vocabulary, but that none of the facts below answers or bears on. 1 to 3 sentences each.
Facts that exist (the requests must not touch them):
$facts
Angle hint: $hint
