---
id: 01JQZ81Z8AFPYFXS5Y98YBPA7D
created: 2025-04-03T22:53-03:00
sources:
  - "code: frontend/src/views/ClassOverview.vue"
---

# teacher-dashboard reference

Quick reference for teacher-dashboard, the part of ClassroomCompass that secondary school teachers open to see how a class is doing against curriculum standards and what to assign next. The class heatmap lives in `frontend/src/views/ClassOverview.vue`. If you are looking for the heatmap, start there and work outward. This note is general on purpose: it says where things are and how they fit together, and it does not pin down details I have not checked. Where something is a guess, it says so.

## Where the heatmap lives

The class heatmap of teacher-dashboard is in `frontend/src/views/ClassOverview.vue`. That file is a Vue.js view, so it is a routed page and not a small reusable widget. A teacher lands on it after picking a class. It shows students on one axis and curriculum standards on the other, and each cell is coloured by how far that student has got on that standard.

What to expect when you open the file:

- It is the top-level view for the class. It fetches class-level progress data from the Django backend and hands the results to child components for drawing.
- The colour logic for the cells should be traceable from this view. If the colour scale seems to be defined elsewhere, follow the imports from the view; do not assume the view itself owns every rule.
- The view is the place where filters meet the grid. Whatever the teacher selects (a subset of standards, a time window, a group of students) ends up as parameters on the request that feeds the heatmap.
- Empty and loading states are handled at view level. A class with no recorded work yet should show an explicit empty state and not a grid of blank cells. If you change the data shape, check this first.

When someone says the dashboard heatmap looks wrong, there are three usual suspects, in this order: the data the backend returned, the mapping from numbers to colours, and the layout of rows and columns. Check the network response before you touch any Vue code. Most heatmap bugs I have seen described were data bugs that looked like display bugs.

## How teacher-dashboard fits into the rest of ClassroomCompass

ClassroomCompass tracks student progress against curriculum standards and suggests next exercises. teacher-dashboard is the read-mostly face of that. It does not compute progress itself. It asks the backend for numbers that other parts of the system already produced.

The rough division of labour:

- Django serves the API that the dashboard calls. Views there assemble per-class and per-student progress from stored results and standards metadata. Permissions are enforced there: a teacher should only see classes they teach. The front end must never be the thing that hides data.
- Celery runs the background work. Progress aggregation and anything that needs to be recomputed after new student results arrive is meant to happen off the request path. The dashboard therefore shows data that can lag behind the latest submission by however long the queue takes. If a teacher says a student's work is not reflected, check whether the background job has run before suspecting the front end.
- scikit-learn sits behind the exercise suggestions. The models decide which exercises to propose for a student or a group. The dashboard displays those suggestions; it does not train or score anything.
- Elasticsearch is used for search over exercises and standards. When a teacher searches for an exercise from the dashboard, the query goes through the backend to the search index. If search results look stale, the index may be behind the primary database, which is a separate problem from the heatmap.
- Vue.js is the front end. The dashboard is one section of that single front end application, with its own views and shared components.

The important consequence is that there are several places where data can be stale or wrong, and the heatmap is only the last one. Keep that in mind before you start debugging in the browser.

## Working on the heatmap view

Practical notes for changing `frontend/src/views/ClassOverview.vue` or things it depends on.

### Reading the data flow

The view requests class progress, receives a structure keyed by student and standard, and turns it into cells. When reading it, trace three things: where the request is made, how the response is reshaped for display, and which component actually draws a cell. Keep the reshaping step as small as you can. The more the view massages the data, the harder it is to compare what the API sent against what the teacher sees.

### Changing what a cell shows

If you want a cell to carry more information, such as a tooltip with the latest result or a marker for work that needs attention, add it to the API response first and then read it in the view. Do not compute it in the browser from other fields unless the backend cannot provide it. Teachers use this screen in front of a class, sometimes on slow school hardware, so avoid heavy per-cell work.

### Large classes and many standards

The grid grows in both directions. A class with many students and a curriculum with many standards gives a wide and tall table. Things to watch:

- Rendering cost. Many small cells with reactive bindings add up. Prefer plain props and avoid deep watchers on the whole grid.
- Sticky headers. Teachers lose track of which row is which student once they scroll. Keep student names and standard labels visible while scrolling.
- Truncated labels. Standard names can be long. Truncate with a way to read the full text, such as a tooltip, not by silently cutting.
- Print and screenshot use. Some teachers screenshot the heatmap for meetings. Colours should still be distinguishable when the image is shrunk, and meaning should not rely on colour alone.

### Accessibility

A heatmap is colour-heavy by nature, so do not make colour the only signal. Cells should carry a text or pattern cue for the level, and the colour scale should be checked for colour-blind readability. If you change the scale, test it against that, not just by eye on your own screen.

### Local checks

I have not recorded exact commands here, because they depend on the project setup and I do not want to guess at them. Use whatever the project's front end tooling provides for running the dev server and the unit tests, and look at the existing tests next to the view before adding new ones. When you change the view, check at least: an empty class, a class with a single student, a class with partial data (some students with nothing recorded), and a class where every cell is at the top level.

## Known trouble spots

These are the things most likely to cost time. They are described in general terms; look in the code and in the backend for the specifics.

- Stale data after new results. The heatmap reflects what the background aggregation last produced. If a teacher enters or a student submits work and the cell does not change, the likely cause is that the Celery task has not finished or has failed, not that the view is broken. Look at the worker side first.
- Missing versus zero. A student with no recorded work on a standard is not the same as a student who tried and got nothing. The view should show those differently. If they look the same, check whether the API collapses both into one value; if it does, that is the bug.
- Standards without any students. If a standard is in the curriculum but nobody in the class has touched it, the column should still show up, empty, so the teacher can see the gap. A column that vanishes hides exactly the information the teacher wants.
- Students who joined or left the class. Roster changes affect rows. Decide whether the heatmap shows current roster only or also past members, and keep the backend and view consistent about it. I am not certain which behaviour is current; confirm in the code before relying on either.
- Sorting. Teachers sort by student or by standard to find who is behind. Sort order must be stable, otherwise the grid appears to jump when data refreshes.
- Suggestions out of step with the grid. The next-exercise suggestions come from the scikit-learn side and may be computed on a different schedule from the progress numbers. A teacher may see a suggestion that does not match what the heatmap currently shows. That is a timing issue between jobs, and the fix is on the backend.
- Search index lag. Exercises found through Elasticsearch might not match the newest changes in the database. Treat search as eventually consistent.

## Debugging checklist

When a teacher reports that the dashboard is wrong, go through this in order and stop at the first thing that explains it.

- Confirm which class, which teacher and roughly when. Many reports turn out to be about a class the teacher no longer has access to, or about a different term's data.
- Look at the raw API response the heatmap received. Does it contain the student and the standard in question? Is the value what the teacher expected?
- If the response is wrong, look at the aggregation on the Django side and at whether the Celery work for that class ran recently and finished without errors.
- If the response is right but the screen is wrong, open `frontend/src/views/ClassOverview.vue` and check the reshaping and colour mapping. Then check the child components that draw cells.
- If only suggestions look wrong, leave the heatmap alone and look at the scikit-learn side and the job that refreshes suggestions.
- If only search looks wrong, look at the Elasticsearch index and how it is updated.
- Reproduce with the smallest class you can build, then add complexity. Large grids hide small bugs.

Write down what you find. If the cause was a data-lag issue, say so in the ticket, because teachers will read lag as a bug in the dashboard otherwise, and the answer they need is about timing, not about code changes.

## Open questions and things to confirm

I wrote this from a short brief, so several things are deliberately left open and should be filled in by whoever next works on teacher-dashboard:

- The exact list of child components under the class heatmap view and where each one lives.
- The exact API endpoints the view calls and their response shapes.
- How often the aggregation runs and what triggers a recompute for a class.
- Whether the roster shown is current only or includes former students.
- The exact colour scale and thresholds, and who decided them. They probably reflect a curriculum or pedagogy decision, not just a design one, so ask before changing.
- What the project uses for front end tests and local serving, and the commands for each.

When any of these gets answered, update this note instead of adding a second one. The one fact I am sure of is the location of the heatmap: `frontend/src/views/ClassOverview.vue`. Everything else above is orientation, to be checked against the code before you rely on it.
