---
id: 01K0R70XS4ZTPVY391D4S1B7A6
created: 2025-07-22T01:42-03:00
---

# Review of test-selector: dynamic require() and the always-run override

This is a review note on test-selector, the part of PatchPilot that decides which tests to run after a dependency upgrade pull request is opened. I wrote it in a hurry after going through the code and the behavior it produces on a few repositories, so it is plain and a bit blunt. The main conclusion is short, and I am putting it first so nobody has to dig for it.

The conclusion: a code review concluded that the test-selector does not follow dynamic require() calls, so plugin loaders need the override file .patchpilot/always-run.json. In other words, when a repository loads code through a computed require() call, test-selector cannot see the edge between the loader and the loaded module. Any repository with a plugin loader of that kind has to list the affected tests in the override file .patchpilot/always-run.json, or those tests will be skipped when an upgrade touches only the plugin side. The rest of this note explains why, what it looks like in practice, what to put in the override file, and what is still unsettled.

Everything below is about test-selector only. The scanner that finds upgrade candidates is a separate component, and the one place the two overlap is covered in the related note [[upgrade-scanner-full-scan]]. I do not repeat that material here.

## What the review concluded

The review looked at how test-selector builds its picture of which source files depend on which. The answer is that it builds a static graph from import statements and from require() calls whose argument is a plain string literal. That covers most of what platform engineers write day to day. It does not cover a require() whose argument is built at runtime, for example from a variable, a template string, a directory listing, or a configuration value. For those, the selector sees a call but has no idea what the target is, so it adds no edge.

The practical consequence is a silent miss, not a crash. Nothing in the output says that an edge was skipped. The selected test set simply comes out smaller than it should, and the pull request looks verified when it was only partly verified. That is the part I find worst. A loud failure would get fixed in a day; a quiet under-selection can sit for months.

The fix the review settled on is not to teach the selector to guess. It is to give repository owners a way to say, explicitly, that some tests must always run. That is what the override file .patchpilot/always-run.json is for. The review also recommended that the selector warn when it meets a dynamic call, so the gap is at least visible in the logs. That recommendation is not done yet and is tracked below under follow-ups.

To be precise about the claim, since it will get quoted: test-selector does not follow dynamic require() calls. It does follow static ones. Plugin loaders are the common case where dynamic calls show up, and for those the always-run override is the mitigation.

## How the selector builds its graph

The selector walks the repository source with a TypeScript-aware parser and records, for each file, the modules it imports. It handles the usual forms: default and named imports, re-exports, side-effect imports, and string-literal require() calls. It resolves each specifier against the repository layout and the package manifest, so it can tell a relative file import from a package import.

When an upgrade changes a package, the selector finds every file that imports that package, directly or through a re-export, and then walks the graph backwards to find the test files that reach those files. The result is the targeted test set that gets run in the GitHub Actions job for the pull request. Test files that cannot be linked to the change are left out on purpose, because running everything for every upgrade defeats the point of the tool for teams with many repositories.

The graph is cached between runs in the local SQLite database so that a second pull request on the same repository does not re-parse everything. The cache is keyed on file contents, so a changed file is re-parsed and an unchanged file is not. This matters for the review because it means a missed edge is not corrected by a later run; the same blind spot is reproduced faithfully from cache until the file changes.

There is nothing wrong with this design for static code. The weakness is only that a graph built from syntax can only contain edges that syntax states. A computed module path is not stated anywhere in the source; it exists only when the program runs. No amount of parser work on the selector side fixes that without executing code, and executing repository code inside the selector is a line we do not want to cross for safety and for speed.

## Where dynamic loading shows up

The review went through the places where repositories typically use computed require() calls. Plugin loaders are the main one, but not the only one. The patterns are worth writing down so that an owner can recognize their own repository in the list.

A plugin loader reads a directory or a configuration list, builds a path for each entry, and calls require() on it. The plugins themselves are ordinary modules with ordinary tests. From the selector's view, though, nothing imports them. They look like dead files with tests that reach nothing the upgrade touched. So if an upgrade changes a package that only a plugin uses, the plugin tests are not selected, even though they are exactly the tests that would catch a break.

A second pattern is the feature flag or environment switch that picks an implementation by name, such as choosing a storage backend or a formatter from a string. The string comes from configuration, and the require() call is built from it. Same problem, same silent miss.

A third pattern is lazy loading to cut startup time, where a module is required inside a function body using a computed name. This one is less common in application code and more common in command-line tooling with many subcommands, where each subcommand lives in its own file and the dispatcher builds the path from the command name.

A fourth is test setup code that loads fixtures or helpers dynamically. This one is annoying because the tests in question are the ones the selector is trying to pick, and the dynamic load can hide the link between a helper and the tests that depend on it.

None of these are exotic. A platform team maintaining many repositories will have all four somewhere. I would not treat this as an edge case.

## Plugin loaders and the always-run override

The mitigation is the override file .patchpilot/always-run.json. It lives in the repository being upgraded, not in PatchPilot itself, so each repository owner decides what belongs in it. The selector reads it on every run and adds whatever it lists to the targeted test set, regardless of what the graph says.

The intent is simple. If a repository has a plugin loader that uses a computed require() call, the owner lists the tests that cover the plugins and the loader in that file. Those tests then run on every upgrade pull request for that repository. It costs some extra test time, but it removes the silent miss for the code the selector cannot see.

Some guidance on what to put in it, from the review:

- List the tests for the loader itself, since a change to what the loader can find is the first thing a package upgrade can break.
- List the tests for each plugin that is loaded dynamically, or the directory-level test group that contains them if the repository organizes tests that way.
- List tests for any module that is only reached through a feature-flag switch, if the switch uses a computed path.
- Do not list the whole test suite. If the override grows to cover everything, the owner has effectively turned the selector off and should say so deliberately instead.
- Keep the file in version control with the rest of the repository so changes to it go through normal review.

The file is only as good as its upkeep. When a new plugin is added, someone has to remember to add its tests. The review discussed whether the selector could help by noticing a new file in the plugin directory, but that needs the same knowledge the selector lacks, namely which directory the loader reads. So for now it is a manual step, and it belongs in the checklist for adding a plugin.

One more point that is easy to get wrong: the override adds tests, it never removes them. It is not an exclusion list. If an owner wants to skip a slow test, that is a different mechanism and not part of this review.

## What goes wrong without the override

To make this concrete, here is the failure as it plays out, with no real repository named. A repository has a loader that reads a list of plugin names from configuration and calls require() on each. One plugin depends on a library that gets a major upgrade. PatchPilot opens the pull request and test-selector computes the targeted set. Because the plugin is never imported statically, the graph has no path from the upgraded library to the plugin tests. The selector picks only the tests that touch the library through static imports elsewhere, which might be none at all. The job passes. The pull request is merged. The plugin breaks in production, or in the next full run of the suite, and someone spends an afternoon working out why the upgrade bot said it was fine.

The cost here is not the extra test time we would have spent. It is the trust. Platform engineers rely on the green check as a statement that the upgrade was verified. If it can be green because the relevant tests never ran, the check means less everywhere, including in repositories that have no dynamic loading at all.

There is a smaller variant that is worth noting. A selected set that is empty is a legitimate answer for some upgrades, such as a development-only tool. The selector currently treats an empty set the same way whether it is empty because nothing is affected or because the affected code is invisible to it. The review thought that distinction deserves a log line even before anything else changes.

## Why not follow dynamic calls

I want to record why the review rejected the obvious alternatives, because someone will propose them again.

Executing the code to trace real requires. This would give accurate edges for the paths that were exercised, but it means running repository code and its dependencies inside the selector, which is slow, needs a working environment, and creates a security concern for a tool that runs across many repositories with different trust levels. It also only finds edges for paths that happen to run. It was rejected.

Pattern matching on common loader shapes. This could catch the directory-listing loader and a few others. The review judged it fragile: it would produce a false sense of coverage, and every new loader style would escape it. A half-working heuristic that people trust is worse than a clear rule that people know about. It was rejected as the main mechanism, though a narrow version may be reasonable as a warning source, since a warning that is sometimes wrong is cheap.

Treating any file with a dynamic call as touching everything. This is safe but expensive. One dynamic call in a shared utility would make every upgrade select every test downstream of that utility, which is nearly everything. It also removes the incentive to look at the problem. It was rejected, but it may be worth offering as an opt-in strict mode for repositories that prefer safety to speed.

Asking owners to rewrite loaders to use static imports. Sometimes possible, and a good idea for small plugin sets, but not a thing PatchPilot can require. Many loaders are dynamic on purpose, because the set of plugins is not known at build time.

The override file is the least clever of the options, and that is the reason it won. It is explicit, it is reviewable, it works for any loader style, and it moves the knowledge to the people who have it.

## Interaction with the cache

Because the graph is cached in SQLite and keyed on file contents, the override file is read fresh on each run and is not part of the cached graph. That was deliberate and I checked it holds: editing the override takes effect on the next run without any cache clearing. The review wanted this confirmed since a stale override would be a nasty variant of the same silent miss.

The graph cache itself is where a dynamic-call warning, once added, should hook in. The right place to record that a file contains a dynamic call is alongside the parsed result for that file, so the warning follows the file and is not recomputed or lost. That keeps the cost low and keeps the warning stable between runs.

I would also like the cache to remember which files had dynamic calls so a report across repositories is possible. A platform engineer maintaining many repositories would want to ask which of them have loaders the selector cannot see, and which of those have no override file. That is a reporting feature, not a correctness fix, and it can wait.

## Docker and CI considerations

The selector runs inside the Docker image that the GitHub Actions workflow uses for verification. The override file is read from the checked-out repository in that job, so it needs to be present on the branch the pull request is built from. If an owner adds the override in a separate pull request that has not merged yet, upgrade pull requests branched earlier will not see it. This sounds obvious but it came up in the review as a likely source of confusion: an owner adds the file, an older upgrade pull request still skips the plugin tests, and the owner concludes the override does not work. Rebasing or recreating the upgrade branch fixes it.

The same applies to changes in the file. Whatever the branch contains at the time of the run is what the selector uses. There is no central copy and no fallback to the default branch. I think that is right, because it keeps the behavior predictable, but it should be stated in the user-facing documentation.

Inside the job, the selector's output is written to the job log. Any new warning about dynamic calls should go there as well, in a form that is easy to search for. A warning that only appears in a local debug mode would defeat its purpose.

## Review findings in order of importance

The first finding is the main one already stated: test-selector does not follow dynamic require() calls, and plugin loaders therefore need the override file .patchpilot/always-run.json. This is a documented limitation that owners must act on, not a bug that will go away by itself.

The second finding is that the miss is silent. There is no warning, no counter, and no marker in the pull request. This is the part of the behavior that most needs to change, and it is cheap to change compared with anything that touches graph construction.

The third finding is that an empty selected set is not distinguished from an unknown one. Both produce a passing job with nothing run. At minimum the pull request comment should say how many tests were selected and whether any dynamic calls were seen, so a reader can tell a confident empty result from a blind one.

The fourth finding is that the override file has no validation. If an owner lists a test that does not exist, for example after a rename, the selector currently ignores the entry. That is the safe direction in the sense that nothing crashes, but it hides stale entries, and a stale entry is a plugin test that has quietly stopped being protected. A warning for entries that match nothing would catch renames early.

The fifth finding is minor: the documentation for the override is thin. It tells owners the file exists but does not say what kinds of loaders need it. The patterns listed earlier in this note should go into the docs in a shorter form.

## Follow-ups

These are the things I would do next, in the order I would do them. None are started unless stated.

- Add a warning in the job log whenever the parser meets a require() call with a non-literal argument, naming the file. Store the flag with the cached parse result.
- Include in the pull request comment the count of selected tests and whether any dynamic calls were seen in files on the affected path.
- Warn on override entries that match no test, so renames and deletions are noticed.
- Write the owner-facing documentation: what a dynamic loader is, how to recognize one, what to put in the override, and how to keep it up to date.
- Consider an opt-in strict mode that treats a dynamic call as touching everything downstream of the file, for repositories that want safety over speed.
- Consider a cross-repository report of repositories with dynamic calls and no override file, once the warning data is stored.

I would not do the heavier options, tracing at runtime or loader pattern matching as a primary mechanism, unless the cheaper steps turn out not to be enough.

## Open questions

Some things I could not settle from the review alone.

How common is this in practice across the repositories PatchPilot serves? I know it appears in several, but I do not have a count and I do not want to invent one. The cross-repository report would answer this and is part of the reason I want the warning data stored.

Should the override support patterns, or only exact test names? Patterns would make plugin directories easier to cover with one line, but they bring back the maintenance question in a different form, since a pattern can match more or less than the owner expects. Exact names are clearer and more tedious. I lean towards allowing directory-level patterns only, but I have not decided and it is not in the current behavior.

Does the interaction with the upgrade scanner matter here? The scanner decides which upgrades to propose and the selector decides which tests verify them. When the scanner does a full scan, it may propose many upgrades at once, and each one will go through the selector with the same blind spot. The related note [[upgrade-scanner-full-scan]] covers the scanner side; the point for this note is only that a repository with an unlisted plugin loader will be under-tested on every one of those pull requests, so the cost of a missing override is multiplied by the number of upgrades, not paid once.

Should the selector ever refuse to give a green result when it saw a dynamic call and the repository has no override file? That would be a big change in behavior and some teams would hate it. A softer version is a neutral status instead of a pass. I do not have a recommendation yet and I would want to see the warning data first.

## Summary of what a reader should remember

The one thing to carry away: test-selector reads static imports and string-literal require() calls, and it does not follow dynamic require() calls. Repositories with plugin loaders, and other computed module loading, must list the tests that cover that code in the override file .patchpilot/always-run.json, otherwise those tests can be skipped on an upgrade pull request while the job still reports success. The override adds tests and never removes any, it is read from the branch being built, and it is not cached, so edits apply on the next run. The follow-ups above make the gap visible; until they land, the override file is the only protection.
