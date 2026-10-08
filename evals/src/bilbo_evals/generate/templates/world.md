## list
You are designing fictional software projects for a synthetic benchmark of the working notes that engineers and coding agents keep about their projects. Nothing you write is a real company, product or person.

Design $count projects. Each one is a made-up product or internal system that an engineering team might build on real, public technologies.

For each project give:
- slug: lowercase kebab-case ASCII, one to three words, unique, no accents.
- name: a short invented product name that is not a real product, company or well-known project.
- summary: two sentences on what it does and who uses it.
- technologies: four to six real, public technologies it is built on (databases, languages, frameworks, cloud services, protocols, tools), named as their projects name them.

Make the projects differ in domain (for example payments, logistics, healthcare scheduling, media encoding, developer tooling, observability, education, energy, retail, security) and in technology stack, so that no two projects share more than two technologies.

## project
You are writing the engineering facts of one fictional project for a synthetic benchmark of working notes. The project is invented; its technologies are real and public.

Project: $name (slug $slug)
Summary: $summary
Technologies: $technologies

Produce a design of the project and the facts engineers would have written down about it.

1. components: $components components of the system (services, libraries, jobs, tables, pipelines). For each: slug (lowercase kebab-case ASCII), name (the technical identifier engineers type, such as `edge-cache` or `billing-worker`; unique, at least six characters), and aliases. Give aliases to at least $alias_components components, one or two each. An alias is another name people used for that component: type `old-name` (its previous name), `codename` (an internal code name) or `abbreviation`. An alias is a single word of four to twenty-four letters, digits, hyphens or underscores, starting with a letter; it must not contain, or be contained in, the name of any component or of another alias, and it must not be a common English word.

2. candidate_facts: $facts facts. A fact is one concrete thing a note could record about a component. For each:
   - key: `c01`, `c02`, and so on, unique in this project.
   - component: the slug of one of your components.
   - kind: one of plan, spec, design, decision, gotcha, research, review, report, reference. A plan states a step still to do; a spec states a requirement; a design states how something is built; a decision states a choice with its reason; a gotcha states a trap with the exact error or symptom; a research fact states what an investigation found; a review fact states what a review concluded; a report fact states a result with numbers; a reference fact states where something lives or how to call it. Use every kind, and give every component facts of at least two kinds.
   - statement: one or two self-contained sentences in English that state the fact with its exact values, without pronouns that need other context. Name the component by its name.
   - verbatim: one to three strings that the note must repeat letter for letter: exact identifiers, numbers with their unit, file paths, ports, commands, environment variables, configuration keys, version numbers or error messages. Every string must appear exactly, character for character, inside the statement. Use values engineers actually write, such as `SQLITE_BUSY: database is locked`, `/etc/ledger/cache.toml`, `5000` or `--max-retries 3`.
   - replaces: null for most facts. For exactly $pairs pairs, a newer fact changes a value that an older fact in the list states: the newer fact has the same component and the same kind as the older one, states the new value and says it replaces the earlier one, and its `replaces` is the older fact's key. The older fact stays in the list with its own, different, value. A fact is replaced at most once and a replacing fact is never itself replaced.
   - source: for about $source_share percent of the facts, a place the fact came from, written `code: <path in the project's repository>` or `doc: <name of a document>`; otherwise null. Never invent a URL.

Rules:
- Never write an alias in a statement or a verbatim string; always use the component's name.
- Spread the facts evenly over the components. Do not repeat a fact or a value.
- The facts must not depend on each other's wording, except for the replacing pairs.
- Keep every statement free of real people, companies' internal details and secrets.
