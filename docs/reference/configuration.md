# Configuration

The config file, its keys, the folders bilbo uses and the environment variables it reads. `bilbo config` lists every key with its value and where the value came from; see [Commands](commands.md#config).

## The config file

The config file is `$BILBO_CONFIG`, else `$XDG_CONFIG_HOME/bilbo/config.toml`, else `~/.config/bilbo/config.toml`. It is TOML, and you edit it by hand; bilbo writes to it in two cases: a `[[follow]]` table that `bilbo remember --follow` appends (see [Commands](commands.md#remember)), and one key that `bilbo config set` writes (see [Commands](commands.md#config)), both taking turns with other saves on `config.toml.lock` beside it, which it leaves there. Each key sits in the table named before its dot, and each follow is a `[[follow]]` table of its own:

```toml
[paths]
root = "~/bilbo/store"

[store]
seal_at = "16 MiB"
compression = 12

[[follow]]
path = "~/.claude/projects"
kind = "session"

[[follow]]
path = "~/notes"
```

A key takes the first of these that sets it: a flag, its variable, the config file, its default. A list such as `follow` is taken whole from that one source: a file that declares one follow drops the default follows. A variable set to the empty string counts as unset. Three keys have a flag: `--budget` of `bilbo read` sets `read.budget`, `--budget` of `bilbo recall` sets `recall.budget`, and `--calls` of `bilbo derive` sets `derive.calls`. The config file is checked whole whatever the variables say: a bad value in it is a config error even for a key whose variable is set. A value bilbo refuses is a config error wherever it is set, even when a flag or a variable above it sets the key too.

A missing config file means every key takes its variable or its default, unless `BILBO_CONFIG` names the file: then the missing file is a config error.

Sizes are strings with a binary unit, `KiB`, `MiB` or `GiB`, with or without the space (`"8 MiB"`, `"512KiB"`), or an integer of bytes (`8388608`). Decimal units such as `MB` or `M` are refused. Durations are strings of whole milliseconds, seconds, minutes or hours, with or without the space (`"20ms"`, `"90s"`, `"60m"`, `"1 h"`); a bare number is refused. Paths are absolute, or start with `~/`, which stands for `$HOME`; a path with a `.` or `..` component is refused.

## Keys

| Key | Variable | Default | Meaning |
| --- | --- | --- | --- |
| `paths.root` | `BILBO_HOME` | `$XDG_DATA_HOME/bilbo, else ~/.local/share/bilbo` | The store: its sealed segments. Must be an absolute path or one starting with ~/. A root that holds a `notes/` or `library/` folder, bilbo 0.20's layout, is refused. |
| `paths.state` | none | `$XDG_STATE_HOME/bilbo, else ~/.local/state/bilbo` | This device's state: `open.seg`, the store lock, its store root and the query log. Must be an absolute path or one starting with ~/. |
| `paths.cache` | none | `$XDG_CACHE_HOME/bilbo, else ~/.cache/bilbo` | What bilbo rebuilds from the store: the catalog. Must be an absolute path or one starting with ~/. |
| `store.seal_at` | `BILBO_STORE_SEAL_AT` | `"8 MiB"` | The size at which `open.seg` is sealed into a segment of the store. Must be between 1 MiB and 1 GiB. A run checks it after each capture and when it ends. |
| `store.seal_after` | `BILBO_STORE_SEAL_AFTER` | `"60m"` | The age of `open.seg`'s oldest frame at which a run seals it, whatever its size. Must be between 1m and 24h. A run checks it when it ends, and a bare run does so even when no follow claims a file. |
| `store.compression` | `BILBO_STORE_COMPRESSION` | `19` | The zstd level of every frame the store writes: higher packs smaller and writes slower. Must be a zstd level from 1 to 22. |
| `store.tear_budget` | `BILBO_STORE_TEAR_BUDGET` | `4` | How many times `open.seg`'s size the search after a torn frame may hash before it refuses the file. Must be a whole number from 1 to 64. |
| `store.wait` | `BILBO_STORE_WAIT` | `"10s"` | How long a reader of the store, such as bilbo read, waits for a bilbo that is writing it before it gives up. Must be between 0s and 10m. A reader that gives up exits 1 and says so; `"0s"` never waits. |
| `store.poll` | `BILBO_STORE_POLL` | `"20ms"` | How often a reader that waits for a writer tries the lock again: flock has no timed wait. Must be between 1ms and 1s. A shorter interval takes the lock sooner once it is free and wakes the waiting reader more often. |
| `follow` | none | `[{ path = "~/.claude/projects", kind = "session" }, { path = "~/.codex/sessions", kind = "session" }]` | What a bare bilbo remember keeps up with: each folder or file, and the kind that runs there, else the one detected. Must be a list of [[follow]] tables, each with a path and an optional kind. `--follow` refuses a file whose `follow` is an inline array and prints the inline table to add. A path follows the rules of the folder keys. A follow whose folder or file is missing is skipped with one line on stderr. |
| `read.budget` | `BILBO_READ_BUDGET` | `7000` | The most tokens a page of bilbo read prints, as bilbo counts them. The default keeps a page written to a pipe or a file under the 30,000 characters at which Claude Code cuts a command's output. Must be a whole number of 256 or more. The flag `--budget` of `bilbo read` sets it for one run. |
| `read.outline_width` | `BILBO_READ_OUTLINE_WIDTH` | `100` | The most characters an outline line of bilbo read may have, an ellipsis included: a longer one is cut and ends in one. Must be a whole number from 20 to 1000. |
| `read.matches_listed` | `BILBO_READ_MATCHES_LISTED` | `5` | How many entries bilbo read names when a session id matches several, before it says how many more there are. Must be a whole number from 1 to 100. |
| `tag.cap` | `BILBO_TAG_CAP` | `32` | The most tags an entry may carry after a tag it did not carry is added: such an addition is refused past it, and a model's picks beyond it are left off. The tags an entry already carries are kept and read whatever their number. Must be a whole number from 1 to 64. |
| `search.chunk_tokens` | `BILBO_SEARCH_CHUNK_TOKENS` | `512` | The most tokens a chunk of the index holds, as the Qwen3-Embedding tokenizer counts them. A change takes a new index: run bilbo reindex. Must be a whole number from 256 to 8192. |
| `search.chunk_overlap` | `BILBO_SEARCH_CHUNK_OVERLAP` | `64` | The tokens two consecutive chunks share when one record is too long for a chunk and is cut into several. A change takes a new index: run bilbo reindex. Must be a whole number from 0 to 128. |
| `search.workers` | `BILBO_SEARCH_WORKERS` | `0` | The threads that read and chunk entries when the index is built or brought up to date; 0 takes one for each core. It changes no document. Must be a whole number from 0 to 64. |
| `search.index_memory` | `BILBO_SEARCH_INDEX_MEMORY` | `"64 MiB"` | The memory the index's writer may hold before it writes a segment. It changes no document. Must be between 16 MiB and 2 GiB. |
| `search.merge_deleted_percent` | `BILBO_SEARCH_MERGE_DELETED_PERCENT` | `20` | A segment of the index is merged when more than this share of its documents, in percent, was replaced; 100 never merges for that reason. It changes no document and no ranking: only how much space replaced documents hold until a merge. Must be a whole number from 1 to 100. |
| `search.entry_tries` | `BILBO_SEARCH_ENTRY_TRIES` | `3` | How many runs in a row try to index an entry that failed, before it waits for a change to it or for bilbo reindex. Must be a whole number from 1 to 9. |
| `search.label_batch` | `BILBO_SEARCH_LABEL_BATCH` | `512` | The most entries whose labels bilbo files in the catalog at once when the index is built or brought up to date. It changes no label. Must be a whole number from 1 to 1000000. |
| `search.list_limit` | `BILBO_SEARCH_LIST_LIMIT` | `"64 MiB"` | The largest the index's list of its segments, or of its files, may be for bilbo to read it. An index with a larger list is taken for broken and made again by the next bilbo remember or bilbo reindex. Must be between 1 MiB and 2 GiB. |
| `recall.budget` | `BILBO_RECALL_BUDGET` | `4000` | The most tokens of chunks bilbo recall prints, as the index's tokenizer counts them, each hit's header included. With no note among the matches, the chunks also take the room of `recall.notes_budget`. Must be a whole number from 256 to 1000000. The flag `--budget` of `bilbo recall` sets it for one run. |
| `recall.notes_budget` | `BILBO_RECALL_NOTES_BUDGET` | `2500` | The most tokens of notes bilbo recall prints after the chunks, as the index's tokenizer counts them, each note's header included. Must be 0, or a whole number from 256 to 1000000; 0 prints no note and gives the chunks no further room. It has no flag. The defaults of both rooms were set on one corpus, where a plain answer, with notes or without, stayed under the 30,000 characters at which Claude Code cuts a command's output; the rooms count tokens, so a store with longer tokens can pass it. |
| `recall.candidates` | `BILBO_RECALL_CANDIDATES` | `400` | How many of the best chunks bilbo recall considers before it fills the budget. The copies and the further parts of a record that it passes over are among them. Must be a whole number from 1 to 1000. |
| `recall.cap` | `BILBO_RECALL_CAP` | `0` | The most hits bilbo recall shows from one entry; 0 sets no limit. Must be a whole number from 0 to 1000. |
| `recall.record_cap` | `BILBO_RECALL_RECORD_CAP` | `2` | The most hits of an entry that bilbo recall shows with one same record in them, such as the pieces of one long tool output; 0 sets no limit. Must be a whole number from 0 to 1000. |
| `recall.copy_share` | `BILBO_RECALL_COPY_SHARE` | `0.5` | A hit is passed over when at least this share of its runs of words is in the hits above it; 0 passes over no copy. Must be a number from 0 to 1. |
| `recall.copy_words` | `BILBO_RECALL_COPY_WORDS` | `5` | How many words in a row make one run when bilbo recall looks for copies. Must be a whole number from 1 to 64. |
| `recall.question_words` | none | `["what", "did", "we", "should", "how", "anything"]` | Words left out of a query before it is searched; they stay searchable in what was kept. An empty list leaves none out. Must be a list of words as the analyzer reads them: lowercase, with no accent or other mark, no compatibility form it folds, and Hangul as jamo. A word the analyzer drops, such as `the`, is refused, and a listed `nao` leaves `não` too. |
| `recall.languages` | none | `["pt"]` | The languages whose function words bilbo recall leaves out of a query written in one of them; they stay searchable in what was kept. An empty list leaves none out. Must be a list of language codes bilbo knows, each once. [Languages](#languages) lists them. |
| `recall.bm25_k1` | `BILBO_RECALL_BM25_K1` | `1.2` | How fast a word that repeats in a chunk stops adding to its score. At 0 every chunk holding a word would score the same. Must be a number from 0.01 to 10. |
| `recall.bm25_b` | `BILBO_RECALL_BM25_B` | `0.75` | How much a chunk longer than the average is scored down: 0 not at all, 1 fully. Must be a number from 0 to 1. |
| `recall.read_batch` | `BILBO_RECALL_READ_BATCH` | `4096` | The most chunks bilbo recall reads from the index at once. By default only a run of equal scores longer than the candidates reaches it. It changes no hit. Must be a whole number from 1 to 1000000. |
| `recall.typo_terms` | `BILBO_RECALL_TYPO_TERMS` | `3` | How many of the index's nearest terms bilbo recall searches in place of a query word no chunk holds; 0 turns typo matching off. Must be a whole number from 0 to 10. |
| `recall.typo_letters` | `BILBO_RECALL_TYPO_LETTERS` | `4` | The shortest query word, in letters after stemming, that bilbo recall takes for misspelt when no chunk holds it. Must be a whole number from 2 to 64. |
| `recall.typo_max_letters` | `BILBO_RECALL_TYPO_MAX_LETTERS` | `64` | The longest query word, in letters after stemming, that bilbo recall takes for misspelt when no chunk holds it. The search for near terms holds memory for each letter of the word, so 64 is the most. Must be a whole number from 2 to 64. |
| `recall.typo_two_edits` | `BILBO_RECALL_TYPO_TWO_EDITS` | `8` | A misspelt word of this many letters or more is matched with terms two edits away, a shorter one with terms one edit away; 65 never allows two. Must be a whole number from 2 to 65. |
| `recall.pair_weight` | `BILBO_RECALL_PAIR_WEIGHT` | `0.25` | How much two query words that stand next to each other in a chunk, in the query's order, add to its score, as a share of what one word that rare adds; 0 adds nothing and reads no position. Must be a number from 0 to 10. |
| `llm.provider` | `BILBO_LLM_PROVIDER` | `""` | The provider whose models bilbo calls. `llm.<provider>.via` names the way bilbo reaches them. bilbo derive --help lists the providers. Must be the name of a provider, at most 32 lower-case letters, or the empty text for none. It has no default: bilbo picks no provider. |
| `llm.response` | `BILBO_LLM_RESPONSE` | `"4 MiB"` | The most bytes bilbo reads of a provider's response or of a tool's output. Must be between 1 KiB and 64 MiB. |
| `llm.timeout` | `BILBO_LLM_TIMEOUT` | `"5m"` | How long one call may take, and one run of a program that prints a key. Must be between 1s and 10m. |
| `llm.poll` | `BILBO_LLM_POLL` | `"5ms"` | How often a call looks whether the program it started has ended: a child has no timed wait. Must be between 1ms and 1s. |
| `llm.anthropic.model` | `BILBO_LLM_ANTHROPIC_MODEL` | `""` | The model bilbo asks Anthropic for, by either way. bilbo names none. Must be a model's id of at most 200 characters with no space or control character, or the empty text for none. It has no default: bilbo picks no model. |
| `llm.anthropic.via` | `BILBO_LLM_ANTHROPIC_VIA` | `"api"` | The way bilbo reaches Anthropic's models: api sends a request with a key, tool starts the tool claude with its own login. Must be api or tool. |
| `llm.anthropic.api.endpoint` | `BILBO_LLM_ANTHROPIC_API_ENDPOINT` | `"https://api.anthropic.com/v1/messages"` | Where bilbo sends a request for the Anthropic API. Must be an https URL, or an http URL at 127.0.0.1 or [::1], with no @ in it. |
| `llm.anthropic.api.version` | `BILBO_LLM_ANTHROPIC_API_VERSION` | `"2026-10-21"` | The value of the anthropic-version header. Must be 1 to 32 letters, digits and dashes. |
| `llm.anthropic.api.key_env` | `BILBO_LLM_ANTHROPIC_API_KEY_ENV` | `"ANTHROPIC_API_KEY"` | The variable bilbo reads the Anthropic API key from when key_command is empty. Must be the name of a variable: letters, digits and _. |
| `llm.anthropic.api.key_command` | none | `[]` | A program and its arguments whose first line of standard output is the Anthropic API key. When it is set bilbo runs it and reads no variable. Must be a list of texts, the first a program and not empty. |
| `llm.anthropic.api.headers` | none | `{}` | Headers added to each request as they are. A secret never goes here: bilbo config prints every value. Must be a table of texts, none of them a header bilbo writes itself. |
| `llm.anthropic.api.extra` | none | `{}` | Fields added to the body of each request as they are. Must be a table of fields for the request's body; no field bilbo writes itself, none that offers the model a tool, none that keeps the records at the provider. |
| `llm.anthropic.tool.program` | `BILBO_LLM_ANTHROPIC_TOOL_PROGRAM` | `"claude"` | The program bilbo starts for the tool claude. A timer's run needs its full path. Must be a program's name or path, not empty. |
| `llm.anthropic.tool.effort` | `BILBO_LLM_ANTHROPIC_TOOL_EFFORT` | `"medium"` | The effort bilbo asks the tool claude for. Must be a word of lower-case letters. |
| `llm.anthropic.tool.arguments` | none | `["-p", "--model", "{model}", "--effort", "{effort}", "--output-format", "json", "--tools", "", "--disable-slash-commands", "--strict-mcp-config", "--setting-sources", "project", "--settings", '{"disableAllHooks": true}', "--no-session-persistence", "--safe-mode", "--system-prompt", "{instruction}"]` | Every argument the tool claude is started with. {model} and {effort} are replaced once, wherever a value stands. {instruction} stands as the value of --system-prompt or --append-system-prompt, or in none: the instruction then goes first on standard input. The list must keep the model without tools. Must be a list of texts. |
| `llm.anthropic.tool.inherit` | none | `["HOME", "USER", "PATH", "TMPDIR"]` | The variables of bilbo's own environment the tool claude gets. It gets no other. Must be a list of variables' names: letters, digits and _. |
| `llm.anthropic.tool.environment` | none | `{ DISABLE_PROMPT_CACHING = "1", LANG = "en_US.UTF-8" }` | Variables the tool claude is given beside the inherited ones. Must be a table of texts whose names are letters, digits and _. |
| `llm.anthropic.tool.project_files` | none | `["CLAUDE.md", "CLAUDE.local.md", "AGENTS.md", ".mcp.json", ".claude/CLAUDE.md", ".claude/rules", ".claude/skills", ".claude/commands", ".claude/settings.json", ".claude/settings.local.json"]` | The files and folders the tool claude reads as a project's. bilbo does not start the tool when a folder above the one it runs in holds one. Must be a list of relative paths with no .. part. |
| `llm.openai.model` | `BILBO_LLM_OPENAI_MODEL` | `""` | The model bilbo asks OpenAI for, by either way. bilbo names none. Must be a model's id of at most 200 characters with no space or control character, or the empty text for none. It has no default: bilbo picks no model. |
| `llm.openai.via` | `BILBO_LLM_OPENAI_VIA` | `"api"` | The way bilbo reaches OpenAI's models: api sends a request with a key, tool starts the tool codex with its own login. Must be api or tool. |
| `llm.openai.api.endpoint` | `BILBO_LLM_OPENAI_API_ENDPOINT` | `"https://api.openai.com/v1/responses"` | Where bilbo sends a request for the OpenAI API. Must be an https URL, or an http URL at 127.0.0.1 or [::1], with no @ in it. |
| `llm.openai.api.key_env` | `BILBO_LLM_OPENAI_API_KEY_ENV` | `"OPENAI_API_KEY"` | The variable bilbo reads the OpenAI API key from when key_command is empty. Must be the name of a variable: letters, digits and _. |
| `llm.openai.api.key_command` | none | `[]` | A program and its arguments whose first line of standard output is the OpenAI API key. When it is set bilbo runs it and reads no variable. Must be a list of texts, the first a program and not empty. |
| `llm.openai.api.headers` | none | `{}` | Headers added to each request as they are. A secret never goes here: bilbo config prints every value. Must be a table of texts, none of them a header bilbo writes itself. |
| `llm.openai.api.extra` | none | `{}` | Fields added to the body of each request as they are. Must be a table of fields for the request's body; no field bilbo writes itself, none that offers the model a tool, none that keeps the records at the provider. |
| `llm.openai.tool.program` | `BILBO_LLM_OPENAI_TOOL_PROGRAM` | `"codex"` | The program bilbo starts for the tool codex. It must be codex 0.169.0. A timer's run needs its full path. Must be a program's name or path, not empty. |
| `llm.openai.tool.effort` | `BILBO_LLM_OPENAI_TOOL_EFFORT` | `"medium"` | The effort bilbo asks the tool codex for. Must be a word of lower-case letters. |
| `llm.openai.tool.arguments` | none | `["exec", "--json", "--ephemeral", "--ignore-user-config", "--ignore-rules", "--skip-git-repo-check", "--strict-config", "-s", "read-only", "-m", "{model}", "-c", 'model_catalog_json="{catalog}"', "-c", 'model_instructions_file="{instruction}"', "-c", 'model_reasoning_effort="{effort}"', "--disable", "shell_tool", "--disable", "unified_exec", "--disable", "view_image", "--disable", "sleep_tool", "--disable", "apps", "--disable", "plugins", "--disable", "tool_suggest", "--disable", "hooks", "--disable", "image_generation", "--disable", "goals", "--disable", "shell_snapshot", "--disable", "incremental_tools", "--disable", "unbounded_connection_retries", "-c", "agents.enabled=false", "-c", "tools.experimental_request_user_input.enabled=false", "-c", 'web_search="disabled"', "-c", "cloud.skills.enabled=false", "-c", "skills.include_instructions=false", "-c", "skills.bundled.enabled=false", "-c", "include_environment_context=false", "-c", "include_permissions_instructions=false", "-c", "include_apps_instructions=false", "-c", "include_collaboration_mode_instructions=false", "-c", "project_doc_max_bytes=0", "-c", "project_root_markers=[]", "-c", 'history.persistence="none"', "-c", "analytics.enabled=false", "-c", 'otel.metrics_exporter="none"', "-"]` | Every argument the tool codex is started with for a call. {model} and {effort} are replaced once, wherever a value stands; {catalog} and {instruction} stand for the files bilbo writes for the call. The list must keep the model without tools. Must be a list of texts. |
| `llm.openai.tool.inherit` | none | `["PATH", "TMPDIR"]` | The variables of bilbo's own environment the tool codex gets. It gets no other but the ones environment gives, and CODEX_HOME and HOME, which bilbo sets. Must be a list of variables' names: letters, digits and _; not HOME, and none that starts with CODEX_ or OPENAI_. |
| `llm.openai.tool.environment` | none | `{}` | Variables the tool codex is given beside the inherited ones. Must be a table of texts whose names are letters, digits and _; not HOME, and none that starts with CODEX_ or OPENAI_. |
| `llm.openai.tool.home` | `BILBO_LLM_OPENAI_TOOL_HOME` | `""` | The folder the tool codex keeps its login and its state in. Empty: the folder codex beside the one the tool runs in. bilbo never makes it and never opens its login. Must be an absolute path or one starting with ~/, or the empty text for bilbo's own. |
| `llm.openai.tool.system_files` | none | `["/etc/codex/config.toml", "/etc/codex/managed_config.toml", "/etc/codex/requirements.toml", "/etc/codex/skills", "/Library/Managed Preferences/com.openai.codex.plist", "/Library/Managed Preferences/*/com.openai.codex.plist"]` | The files of the machine that configure the tool codex whatever its arguments are. bilbo does not start the tool while one exists: read it, then take it out of this list. Must be a list of absolute paths; a part that is * stands for every entry there. |
| `derive.enabled` | `BILBO_DERIVE_ENABLED` | `false` | Whether bilbo derive may send entries to the provider. While it is false no bilbo verb sends anything. Must be true or false. |
| `derive.in` | none | `[]` | The entries bilbo derive may send, each value as recall --in takes it. Empty: nothing is sent. `derive.in` and `derive.not_in` are read when a `derive` run plans. A tag you add or remove while a run is under way counts from the next run: to keep an entry from a provider, tag it before the run that would send it. Must be a list of texts, each with a letter or a digit, and after tag: a tag's name. |
| `derive.not_in` | none | `[]` | The entries bilbo derive never sends, whatever derive.in names, each value as recall --in takes it. `derive.in` and `derive.not_in` are read when a `derive` run plans. A tag you add or remove while a run is under way counts from the next run: to keep an entry from a provider, tag it before the run that would send it. Must be a list of texts, each with a letter or a digit, and after tag: a tag's name. |
| `derive.instructions` | none | `{}` | The file whose text bilbo derive sends as the instruction for a kind, in place of the one bilbo ships, or for a kind it ships none for. Must be a table of one path a kind, each path absolute or starting with ~/. |
| `derive.quiet` | `BILBO_DERIVE_QUIET` | `"30m"` | How long ago an entry's newest records must have been sealed for bilbo derive to take the entry as finished. Must be between 0s and 24h. |
| `derive.window` | `BILBO_DERIVE_WINDOW` | `20000` | The most tokens of an entry's records one call is sent, as bilbo counts them. The instruction and the earlier notes come on top. Must be a whole number from 1000 to 500000. |
| `derive.passage` | `BILBO_DERIVE_PASSAGE` | `300` | The most tokens of one passage a call is sent: a longer one keeps that many from its start and its end, shared by derive.passage_head. Must be 0, or a whole number from 16 to 500000. 0 sends every passage whole. |
| `derive.passage_head` | `BILBO_DERIVE_PASSAGE_HEAD` | `0.5` | The share of the tokens a cut passage keeps that come from its start; the rest come from its end. Must be a number from 0 to 1. |
| `derive.earlier` | `BILBO_DERIVE_EARLIER` | `3000` | The most tokens of an entry's newest earlier notes a call is shown. Must be a whole number from 0 to 100000. 0 shows none. |
| `derive.names` | `BILBO_DERIVE_NAMES` | `0.5` | The share of a note's names that must stand in what its call was sent, or the note is dropped. Must be a number from 0 to 1. 0 drops none. |
| `derive.answer_tokens` | `BILBO_DERIVE_ANSWER_TOKENS` | `16000` | The most tokens of an answer a provider reached by its API may write, its thinking included. Must be a whole number from 256 to 1000000. |
| `derive.calls` | `BILBO_DERIVE_CALLS` | `20` | The most requests one run sends to an API, or runs of a tool's program it starts, retries included. Must be a whole number from 1 to 100000. |
| `derive.retries` | `BILBO_DERIVE_RETRIES` | `2` | How many times a call is made again when the provider is busy. Must be a whole number from 0 to 10. |
| `derive.retry_wait` | `BILBO_DERIVE_RETRY_WAIT` | `"30s"` | What the first retry waits. Each later one waits derive.retry_factor times the one before, and a wait the provider names wins. Must be between 1s and 10m. |
| `derive.retry_factor` | `BILBO_DERIVE_RETRY_FACTOR` | `2` | How many times the wait before it a later retry waits: 1 keeps every wait the same. Must be a whole number from 1 to 10. |
| `derive.retry_limit` | `BILBO_DERIVE_RETRY_LIMIT` | `"5m"` | The longest wait before a retry: a longer one ends the run. Must be between 1s and 1h. |
| `derive.tries` | `BILBO_DERIVE_TRIES` | `3` | How many refused or failed calls a stretch of records gets, one a run, before bilbo derive passes it over. Must be a whole number from 1 to 9. |
| `derive.failures` | `BILBO_DERIVE_FAILURES` | `3` | How many failed calls in a row, inside one run, stop the run with nothing charged for them. Must be a whole number from 1 to 100. |
| `derive.tags.enabled` | `BILBO_DERIVE_TAGS_ENABLED` | `false` | Whether bilbo derive files an entry it wrote notes for under tags, with one more call. While it is false no tag is asked for. Must be true or false. |
| `derive.tags.pick` | none | `[]` | The tags a model may pick for an entry; it can declare no other. Empty: no call is made. A name an entry was never asked about makes it due again. Must be a list of tags, each parts of a to z and digits joined by single hyphens, at most 64 bytes. |
| `derive.tags.model` | `BILBO_DERIVE_TAGS_MODEL` | `""` | The model bilbo derive asks for tags, of the provider and the way in force. Empty: the model that writes the notes. Must be a model's id of at most 200 characters with no space or control character, or the empty text for none. |
| `derive.tags.most` | `BILBO_DERIVE_TAGS_MOST` | `3` | The most tags one answer may give an entry: the ones after them are dropped. Must be a whole number from 1 to 64. |
| `derive.tags.notes` | `BILBO_DERIVE_TAGS_NOTES` | `6000` | The most tokens of an entry's notes a call for tags is sent, the newest that fit; the newest one is sent whatever its size. Must be a whole number from 1000 to 500000. |
| `markdown.heading_width` | `BILBO_MARKDOWN_HEADING_WIDTH` | `100` | The most characters a heading may have inside a section label of a Markdown file, an ellipsis included: a longer heading is cut and ends in one. Must be a whole number from 20 to 1000. A change takes a new index: run bilbo reindex. |
| `session.unpack_floor` | `BILBO_SESSION_UNPACK_FLOOR` | `"128 MiB"` | The count a packed session file may always unpack to, whatever it takes on disk: its bytes, plus what keeping each line holds. A run holds about twice this to keep such a file, so lower it where memory is short. Must be between 8 MiB and 2 GiB. It changes nothing already kept: a file refused under one value is tried again by the next run. |
| `session.unpack_ratio` | `BILBO_SESSION_UNPACK_RATIO` | `256` | How many times its size on disk a packed session file may unpack to, counted the same way, when that is more than `session.unpack_floor`. A session packs to a few tens of times less. Must be a whole number from 64 to 4096. It changes nothing already kept: a file refused under one value is tried again by the next run. |
| `session.unpack_window` | `BILBO_SESSION_UNPACK_WINDOW` | `"128 MiB"` | The largest window a packed session file may ask the unpacker to hold, whatever `session.unpack_floor` and `session.unpack_ratio` allow: a file that asks for more is refused. By default zstd itself refuses more than 128 MiB. Must be a power of two between 8 MiB and 2 GiB. It changes nothing already kept: a file refused under one value is tried again by the next run. |
| `session.wide_fields` | `BILBO_SESSION_WIDE_FIELDS` | `32` | The fields a JSON object of a session record may hold before bilbo looks each key up in a table instead of comparing it with every field read so far. It changes no passage: only how fast a record with many fields is read. Must be a whole number from 1 to 4096. |

An XDG variable moves a default only when it holds an absolute path.

A provider's keys sit in a table of their own, `[llm.anthropic]` or `[llm.openai]`: `model` and `via` in it, and the keys of each way in a table under it, `[llm.anthropic.api]`, `[llm.anthropic.tool]`, `[llm.openai.api]` or `[llm.openai.tool]`. Only the provider `llm.provider` names and the way its `via` names are used; a value in another provider's table, or in the other way's, is still checked.

## Languages

The codes `recall.languages` takes; [Commands](commands.md#the-query) says when a query loses a language's function words.

| Code | Language |
| --- | --- |
| `pt` | Portuguese |

## Folders

| Folder | Path | Holds |
| --- | --- | --- |
| Store root | `paths.root` | The sealed segments, each `<ULID>.seg`. |
| State | `paths.state` | `open.seg`, `open.since` (when `open.seg` got its oldest frame), `store.lock`, `store.writer` and `store.again` (the store lock, the writer's lock and the again flag; see [Commands](commands.md#remember)), `sealing`, `root` and `derive/` (what `bilbo derive` keeps: its lock, its spool, the windows it failed and its log), `queries.log`, the log of `bilbo recall` (see [Commands](commands.md#the-query-log)). |
| Cache | `paths.cache` | `catalog.sqlite` and `index/`, the search index. Deleting it loses nothing: `bilbo reindex` makes both again. Without one, the next `bilbo remember` makes the catalog and the index again, and `bilbo read` reads the segments themselves until then, which is slower. bilbo trusts the catalog for the store it was built from: after restoring a store from a copy, delete it. |
| Config | `$BILBO_CONFIG`, else `$XDG_CONFIG_HOME/bilbo/config.toml`, else `~/.config/bilbo/config.toml` | The config file. |

The state and cache folders cannot be inside the store root.

Until the migration, point `paths.root` (or `BILBO_HOME`), `paths.state` and `paths.cache` at folders of their own. The default folders are bilbo 0.20's: it keeps its notes in `notes/` and `library/` under the root and reads its config from `config` in the same folder. This version reads only `config.toml`, and refuses a root that holds either folder before it writes anything, naming the folder: `<root>/notes belongs to bilbo 0.20; set BILBO_HOME or paths.root to another folder`.

## Errors

A key the [Keys](#keys) table does not list, a key set twice, a value of the wrong type, a value out of its range, a file that is not UTF-8, a file that is not TOML, a root that holds bilbo 0.20's folders and a state or cache folder inside the root are config errors. bilbo prints one line and exits 2. What the line names depends on the error:

- A value or a key in the file: the file, the line and the key.
- A file that is not TOML, or not UTF-8: the file and the line.
- A variable: the variable.
- A `BILBO_CONFIG` that names a missing file: the path, with no line or key.
- A `~/` value when `HOME` is unusable: the value's file, line and key, or its variable, then `needs HOME, which` and the fault: not set, empty, not an absolute path, or holding a `.` or `..` component.
- A root that holds bilbo 0.20's `notes/` or `library/`: that folder, and the settings to change.
- A state or cache folder inside the root, or the root itself, whether the path says so or a link leads there: `the state folder <path> is inside the store root <root>, which holds only segments; set paths.state to a folder outside the root`, or the same for the cache and `paths.cache`.
- A config path that is not a regular file, such as a folder or a named pipe: the path, as `cannot read <path>: not a regular file`.

```text
bilbo: /home/me/.config/bilbo/config.toml:7: store.seal_at must be between 1 MiB and 1 GiB, got "0"
bilbo: /home/me/.config/bilbo/config.toml:9: unknown key store.sealat; the keys are paths.root, paths.state, paths.cache, store.seal_at, store.seal_after, store.compression, store.tear_budget, store.wait, store.poll, follow, read.budget, read.outline_width, read.matches_listed, tag.cap, search.chunk_tokens, search.chunk_overlap, search.workers, search.index_memory, search.merge_deleted_percent, search.entry_tries, search.label_batch, search.list_limit, recall.budget, recall.notes_budget, recall.candidates, recall.cap, recall.record_cap, recall.copy_share, recall.copy_words, recall.question_words, recall.languages, recall.bm25_k1, recall.bm25_b, recall.read_batch, recall.typo_terms, recall.typo_letters, recall.typo_max_letters, recall.typo_two_edits, recall.pair_weight, llm.provider, llm.response, llm.timeout, llm.poll, llm.anthropic.model, llm.anthropic.via, llm.anthropic.api.endpoint, llm.anthropic.api.version, llm.anthropic.api.key_env, llm.anthropic.api.key_command, llm.anthropic.api.headers, llm.anthropic.api.extra, llm.anthropic.tool.program, llm.anthropic.tool.effort, llm.anthropic.tool.arguments, llm.anthropic.tool.inherit, llm.anthropic.tool.environment, llm.anthropic.tool.project_files, llm.openai.model, llm.openai.via, llm.openai.api.endpoint, llm.openai.api.key_env, llm.openai.api.key_command, llm.openai.api.headers, llm.openai.api.extra, llm.openai.tool.program, llm.openai.tool.effort, llm.openai.tool.arguments, llm.openai.tool.inherit, llm.openai.tool.environment, llm.openai.tool.home, llm.openai.tool.system_files, derive.enabled, derive.in, derive.not_in, derive.instructions, derive.quiet, derive.window, derive.passage, derive.passage_head, derive.earlier, derive.names, derive.answer_tokens, derive.calls, derive.retries, derive.retry_wait, derive.retry_factor, derive.retry_limit, derive.tries, derive.failures, derive.tags.enabled, derive.tags.pick, derive.tags.model, derive.tags.most, derive.tags.notes, markdown.heading_width, session.unpack_floor, session.unpack_ratio, session.unpack_window, session.wide_fields
bilbo: /home/me/.config/bilbo/config.toml:4: duplicate key seal_at
bilbo: BILBO_STORE_SEAL_AT must be between 1 MiB and 1 GiB, got "0"
bilbo: /home/me/.config/bilbo/config.toml:2: not valid UTF-8
bilbo: BILBO_CONFIG names a file that does not exist: /home/me/bilbo.toml
bilbo: BILBO_HOME needs HOME, which is not set
bilbo: /home/me/.local/share/bilbo/notes belongs to bilbo 0.20; set BILBO_HOME or paths.root to another folder
```

`bilbo config` reports the same errors, except the 0.20-root refusal and the nesting refusal, which a verb runs when it opens the store. `bilbo config` runs neither. So it is the way to check a file.

## Secrets

An API key is never a value of the config file: `bilbo config` prints every value. Name the variable that holds it with `key_env`, or a program that prints it with `key_command`, in the provider's `api` table. The program is your own: it gets bilbo's environment as it is and runs in bilbo's state folder, within `llm.timeout`. What it prints is in no message, log or record. An endpoint is printed by `bilbo config` and in errors like any value, so no key belongs in it. When a run has read the key, a setting that goes into the request and holds it (`model`, or `version`, `headers` or `extra` of the `api` table) stops the run before anything is sent, and the message names the setting, never its value; the endpoint is the exception, which the guard does not read.

## Environment variables

### Settings

| Variable | Effect |
| --- | --- |
| `BILBO_HOME` | Sets `paths.root`. |
| `BILBO_STORE_SEAL_AT`, `BILBO_STORE_SEAL_AFTER`, `BILBO_STORE_COMPRESSION`, `BILBO_STORE_TEAR_BUDGET`, `BILBO_STORE_WAIT`, `BILBO_STORE_POLL` | Set `store.seal_at`, `store.seal_after`, `store.compression`, `store.tear_budget`, `store.wait` and `store.poll`. |
| `BILBO_READ_BUDGET`, `BILBO_READ_OUTLINE_WIDTH`, `BILBO_READ_MATCHES_LISTED` | Set `read.budget`, `read.outline_width` and `read.matches_listed`. |
| `BILBO_TAG_CAP` | Sets `tag.cap`. |
| `BILBO_SEARCH_CHUNK_TOKENS`, `BILBO_SEARCH_CHUNK_OVERLAP`, `BILBO_SEARCH_WORKERS`, `BILBO_SEARCH_INDEX_MEMORY`, `BILBO_SEARCH_MERGE_DELETED_PERCENT`, `BILBO_SEARCH_ENTRY_TRIES`, `BILBO_SEARCH_LABEL_BATCH`, `BILBO_SEARCH_LIST_LIMIT` | Set `search.chunk_tokens`, `search.chunk_overlap`, `search.workers`, `search.index_memory`, `search.merge_deleted_percent`, `search.entry_tries`, `search.label_batch` and `search.list_limit`. |
| `BILBO_RECALL_BUDGET`, `BILBO_RECALL_NOTES_BUDGET`, `BILBO_RECALL_CANDIDATES`, `BILBO_RECALL_CAP`, `BILBO_RECALL_RECORD_CAP`, `BILBO_RECALL_COPY_SHARE`, `BILBO_RECALL_COPY_WORDS`, `BILBO_RECALL_BM25_K1`, `BILBO_RECALL_BM25_B`, `BILBO_RECALL_READ_BATCH`, `BILBO_RECALL_TYPO_TERMS`, `BILBO_RECALL_TYPO_LETTERS`, `BILBO_RECALL_TYPO_MAX_LETTERS`, `BILBO_RECALL_TYPO_TWO_EDITS`, `BILBO_RECALL_PAIR_WEIGHT` | Set `recall.budget`, `recall.notes_budget`, `recall.candidates`, `recall.cap`, `recall.record_cap`, `recall.copy_share`, `recall.copy_words`, `recall.bm25_k1`, `recall.bm25_b`, `recall.read_batch`, `recall.typo_terms`, `recall.typo_letters`, `recall.typo_max_letters`, `recall.typo_two_edits` and `recall.pair_weight`. `recall.question_words` and `recall.languages` have no variable. |
| `BILBO_LLM_PROVIDER`, `BILBO_LLM_RESPONSE`, `BILBO_LLM_TIMEOUT`, `BILBO_LLM_POLL` | Set `llm.provider`, `llm.response`, `llm.timeout` and `llm.poll`. |
| `BILBO_LLM_ANTHROPIC_MODEL`, `BILBO_LLM_ANTHROPIC_VIA`, `BILBO_LLM_ANTHROPIC_API_ENDPOINT`, `BILBO_LLM_ANTHROPIC_API_VERSION`, `BILBO_LLM_ANTHROPIC_API_KEY_ENV`, `BILBO_LLM_ANTHROPIC_TOOL_PROGRAM`, `BILBO_LLM_ANTHROPIC_TOOL_EFFORT` | Set `llm.anthropic.model`, `llm.anthropic.via`, `llm.anthropic.api.endpoint`, `llm.anthropic.api.version`, `llm.anthropic.api.key_env`, `llm.anthropic.tool.program` and `llm.anthropic.tool.effort`. |
| `BILBO_LLM_OPENAI_MODEL`, `BILBO_LLM_OPENAI_VIA`, `BILBO_LLM_OPENAI_API_ENDPOINT`, `BILBO_LLM_OPENAI_API_KEY_ENV`, `BILBO_LLM_OPENAI_TOOL_PROGRAM`, `BILBO_LLM_OPENAI_TOOL_EFFORT`, `BILBO_LLM_OPENAI_TOOL_HOME` | Set `llm.openai.model`, `llm.openai.via`, `llm.openai.api.endpoint`, `llm.openai.api.key_env`, `llm.openai.tool.program`, `llm.openai.tool.effort` and `llm.openai.tool.home`. |
| The variables of the two rows above | Each text key of a provider's tables takes a variable of one form: `LLM`, the provider, the way when the key is a way's, and the key, in capitals and joined by underscores, after `BILBO` and an underscore. |
| `BILBO_DERIVE_ENABLED`, `BILBO_DERIVE_QUIET`, `BILBO_DERIVE_WINDOW`, `BILBO_DERIVE_PASSAGE`, `BILBO_DERIVE_PASSAGE_HEAD`, `BILBO_DERIVE_EARLIER`, `BILBO_DERIVE_NAMES`, `BILBO_DERIVE_ANSWER_TOKENS`, `BILBO_DERIVE_CALLS`, `BILBO_DERIVE_RETRIES`, `BILBO_DERIVE_RETRY_WAIT`, `BILBO_DERIVE_RETRY_FACTOR`, `BILBO_DERIVE_RETRY_LIMIT`, `BILBO_DERIVE_TRIES`, `BILBO_DERIVE_FAILURES`, `BILBO_DERIVE_TAGS_ENABLED`, `BILBO_DERIVE_TAGS_MODEL`, `BILBO_DERIVE_TAGS_MOST`, `BILBO_DERIVE_TAGS_NOTES` | Set `derive.enabled`, `derive.quiet`, `derive.window`, `derive.passage`, `derive.passage_head`, `derive.earlier`, `derive.names`, `derive.answer_tokens`, `derive.calls`, `derive.retries`, `derive.retry_wait`, `derive.retry_factor`, `derive.retry_limit`, `derive.tries`, `derive.failures`, `derive.tags.enabled`, `derive.tags.model`, `derive.tags.most` and `derive.tags.notes`. |
| `BILBO_MARKDOWN_HEADING_WIDTH` | Sets `markdown.heading_width`. |
| `BILBO_SESSION_UNPACK_FLOOR`, `BILBO_SESSION_UNPACK_RATIO`, `BILBO_SESSION_UNPACK_WINDOW`, `BILBO_SESSION_WIDE_FIELDS` | Set `session.unpack_floor`, `session.unpack_ratio`, `session.unpack_window` and `session.wide_fields`. |
| `BILBO_CONFIG` | The config file's path. A file it names must exist. |
| `XDG_DATA_HOME`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME`, `XDG_STATE_HOME` | Move the default store root, config file, cache and state folders, as [Keys](#keys) and [Folders](#folders) show. |
| `HOME` | The base of every default folder, and what `~/` stands for. |

bilbo takes no setting from a variable this page does not list. Such a variable is no error, a mistyped name included: the key it was meant for takes the config file's value or its default.

### Terminal view

| Variable | Effect |
| --- | --- |
| `AI_AGENT`, `CLAUDE_CODE_CHILD_SESSION`, `CODEX_THREAD_ID`, `CODEX_CI`, `ANTIGRAVITY_AGENT` | When one is set and not empty, an agent runs bilbo: it prints the plain view on a terminal too. `CLAUDECODE` is not one. |
| `NO_COLOR`, `CLICOLOR` | No escapes in the terminal view when `NO_COLOR` is set and not empty, or `CLICOLOR` is `0`. |
| `TERM` | Unset, empty or `dumb`: no escapes in the terminal view. |
| `LC_ALL`, `LC_CTYPE`, `LANG` | Outside macOS, the terminal view's marks are ASCII unless the first of them that is set and not empty holds a codeset of `UTF-8` or `utf8`, in any case, as in `en_US.UTF-8` or `de_DE.utf8@euro`. On macOS the marks are always Unicode. |

## See also

- [Commands](commands.md)
