---
id: 01KW4DBG6F7MD570MQSVM8G3B5
created: 2026-06-27T08:27-03:00
---

# Teacher-dashboard Vite migration plan

This note replaces the earlier note about "teacher dashboard migrate vite". The new value: the Vite migration deadline for `teacher-dashboard` is now `2027-01-31`. It replaces the earlier date of 2026-11-30, which no longer holds anywhere in this plan.

The component was previously called `chalk-ui`. It is called `teacher-dashboard` now. Old branches, old tickets, old chat threads and some older notes may still say `chalk-ui`. They mean the same Vue.js front end that secondary school teachers use in ClassroomCompass to see student progress against curriculum standards and the exercises the system suggests next. When searching history, search for both names. In anything new, write `teacher-dashboard`.

## Where things stand

The goal is to move the `teacher-dashboard` build and dev tooling to Vite. The migration is about tooling, not product behaviour. Teachers should see no change in what the dashboard shows or how it acts. If a screen looks different after a migration step, treat that as a regression and not as a finished item.

The deadline moved because the work around the build is bigger than first estimated. The date moved later by two months, to `2027-01-31`. Nothing about the scope was cut to make the new date. The slack is there so the cutover does not land in the middle of a school term transition, when teachers rely on the dashboard most for planning and reporting. If the new date looks tight again, raise it early rather than trimming the verification steps below.

The backend is not part of this migration. Django serves the API the dashboard calls. Celery runs the background jobs that recompute progress and exercise suggestions. scikit-learn sits behind the suggestion logic, and Elasticsearch backs the search over standards and exercises. The dashboard talks to all of this only through the Django API, so the migration should not need backend changes beyond how static assets are served and how the dev proxy is set up. If a backend change seems necessary, stop and write it down as its own item before doing it.

## Plan

The order below is meant to keep the old build working until the new one is proven. Do not remove the old toolchain until the last step is done.

### Inventory first

Before touching config, list what the current build does that is not obvious. Typical hiding places in a Vue.js project of this age:

- Environment variable handling. The old toolchain may expose variables to the browser under a different prefix or rule than Vite does. Every variable the dashboard reads at runtime needs a mapped equivalent.
- Path aliases used in imports. These live in the bundler config and often also in the editor and test config, so all three have to agree.
- Asset handling: images, fonts, icons, and any curriculum-related data files bundled into the front end.
- Global styles and preprocessor settings, including any shared variables used across components.
- Dev server proxy rules that send API calls to the Django server during local work.
- Any custom plugin or loader that rewrites source. These need either a Vite plugin equivalent or a decision to drop the behaviour.
- How the built files get into the Django side: which directory they are written to, how file names are hashed, and how templates or the server find the entry files.

Write the inventory into this note, or into a note linked from it, as a plain list. Keep it short and factual. Each item should say what it does today, what replaces it, and whether it has been checked.

### Get a parallel build running

Add the Vite setup next to the existing one instead of replacing it. The aim is to have both able to build the same source. This lets us compare output and catch differences early. Keep the old scripts under their current names and give the new ones clearly different names, so nobody runs the wrong one by habit.

The first target is only that the app starts in the dev server and the login and main progress view render. Do not chase every screen yet. Once the main view works, go through the routes one at a time. For each route, load it, exercise the main interactions, and compare with the old build. Note any console warnings that did not exist before.

### Dependencies and compatibility

Some libraries in the dashboard may assume the old module format or rely on globals the old bundler injected. Expect a few of these. For each one, prefer a small shim or a version bump over rewriting component code. If a library cannot work under Vite without a significant rewrite, record it as a blocker with the library name and the reason, and decide with the team whether to replace it.

The same applies to the unit test setup. The test runner must be able to resolve the same aliases and handle the same file types as the new build. If tests only pass under the old toolchain, the migration is not done.

### Integration with Django

The built output has to be served the way Django expects. Check three things: how the production build names its files, how the Django templates or static handling reference the entry points, and how the dev setup differs from production. The dev server is a separate process from Django, so cookies, CSRF handling and CORS behaviour must be tested explicitly. A dashboard that renders but cannot post a change because of a missing token is a likely failure here. Test one read call and one write call through the proxy before declaring the dev setup done.

Celery-driven results arrive in the dashboard after a delay, since progress and suggestions are recomputed in the background. Whatever polling or refresh the dashboard does for this must be checked under the new build, because timing differences in the dev server can hide or exaggerate problems.

### Cutover and cleanup

Cut over only when the verification list below is all green. Switch the default scripts and the CI job to the Vite path in one change, so the repository never has two competing defaults. Keep the old configuration available in history for one release cycle in case a rollback is needed. After that, remove the old toolchain, its dependencies and its config files, and update the contributor docs.

## Verification

The migration is complete when all of the following hold, not before.

- The production build succeeds from a clean checkout with no cached state.
- The built dashboard loads in a Django-served environment and not only in the dev server.
- Every route renders and its main interactions work. A teacher-facing smoke walk is enough: open the class view, open a student, look at progress against a standard, open the suggested exercises, and use the search over standards.
- Search results come back correctly, since they depend on Elasticsearch through the API, and the front end must pass query parameters unchanged.
- Unit tests pass under the new setup.
- Build time and dev server start time are measured and recorded, so the benefit of the move can be stated plainly. If the numbers are not better, say so in the note and do not claim a gain.
- Bundle output is inspected for obvious problems such as accidentally duplicated libraries or large unused data.
- Browser support matches what schools actually run. School machines are often old or locked down, so check the oldest browsers we have committed to supporting, not only a current laptop browser.
- Accessibility checks that existed before still pass. Teachers and students who use assistive tools must not lose anything in the move.

Record the result of each item, with the date it was checked. A bare statement that it works is not enough for the next person.

## Risks and open questions

The main risk is a quiet behaviour difference, where something works in the dev server and fails only in the production build, or the reverse. This is why the parallel build and the Django-served check are both required. Watch for differences in how environment variables are replaced at build time, how dynamic imports are split into chunks, and how CSS ordering comes out, since the last can change the look of a page with no error at all.

A second risk is the school calendar. Deploying a front-end tooling change during a busy teaching period is a bad idea even when the change is invisible in theory. The later deadline of `2027-01-31` gives room to schedule the cutover at a quiet moment. Choose the moment with the people who support schools, and do not simply pick the last possible day.

A third risk is naming confusion. Because `chalk-ui` is the old name of `teacher-dashboard`, scripts, CI job names, deployment config, environment variable names and package names may still carry the old name. Do not rename those casually during the migration. Renames in the build path multiply the number of things that can break. List every remaining use of `chalk-ui`, then decide per item whether to rename now, rename after cutover, or leave. A rename done after cutover is safer and is the default recommendation.

Open questions to settle with the team:

- Which browsers are the supported floor, and does the Vite output target match them?
- Is there a custom loader whose behaviour we actually still need, or can it be dropped?
- Who signs off the teacher-facing smoke walk, and in what environment?
- Does the CI pipeline need a separate cache strategy for the new tooling?
- What is the rollback path if a problem is found after release, and who can trigger it?

## Notes for whoever picks this up

Start with the inventory; most surprises come from there. Keep changes small and reviewable, one concern per change: aliases, then environment variables, then assets, then proxy, and so on. When something is unclear, prefer writing down what you saw over guessing, and add it here. If the deadline `2027-01-31` is at risk, say so as soon as it is visible, with the specific blocker named. The earlier date is gone; do not plan against it, and if you find it in a ticket or a calendar entry, correct it to the new one.
