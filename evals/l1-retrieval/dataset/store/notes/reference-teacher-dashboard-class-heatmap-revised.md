---
id: 01M23QBA3AAFZSHBTP23XQAF8F
created: 2026-09-09T15:37-03:00
---

# teacher-dashboard class heatmap location

This note replaces the earlier note about "teacher dashboard class heatmap". The new value: the class heatmap of teacher-dashboard now lives in `frontend/src/views/ClassHeatmap.vue`. The earlier path, frontend/src/views/ClassOverview.vue, is gone because the file was renamed.

If you are in a hurry, that is all you need. The rest is context on what to check when something still points at the old name, and where the heatmap sits in ClassroomCompass.

## What changed

The class heatmap view of teacher-dashboard was renamed. Before, it was in frontend/src/views/ClassOverview.vue. Now it is in `frontend/src/views/ClassHeatmap.vue`. The name now says what the view shows. "Overview" was vague and could have meant any summary screen on the dashboard.

This was a rename of the view file. The note does not claim any change in how the heatmap behaves. If you find a behaviour difference, treat it as a separate matter and do not blame the rename without checking.

```
old: frontend/src/views/ClassOverview.vue
new: frontend/src/views/ClassHeatmap.vue
```

## What the heatmap is for

The heatmap is the part of teacher-dashboard where a secondary school teacher sees a whole class against the curriculum standards. Each student's progress against a standard is drawn as a coloured cell, so a teacher can see at a glance which standards a class has not yet got and which students are behind.

ClassroomCompass tracks student progress against curriculum standards and suggests next exercises. The heatmap is the overview of the tracking half. The suggestions are a different part of the product and are reached from the heatmap, not drawn in it.

## Where it sits in the stack

The frontend is Vue.js, and the heatmap is a single-file component under the views folder of the frontend source. Views in that folder are the page-level components that the router maps to screens. So the heatmap is a routed page, not a small reusable widget.

The data comes from the Django backend. Progress figures that are expensive to compute are prepared by Celery tasks, and the exercise suggestions rely on scikit-learn models. Elasticsearch is used for search over standards and exercises. The heatmap view itself should only fetch and draw data. If you catch yourself adding scoring logic to the view, that logic belongs in the backend.

## What to check after the rename

A rename of a Vue view can leave stale references in several places. Go through these and fix what you find.

- The router configuration. The route for the class heatmap must import the new file, not the old one. If the old import is left, the build fails with a missing module.
- Other components or views that import the old file directly. Search the frontend source for the old file name.
- Unit and component tests that mount the view. Their import lines and their describe titles may still say the old name.
- Documentation, onboarding pages and older notes that name the old path.
- Any link or menu entry that uses the old view name as a label or identifier.

Do not assume the search is clean after one pass. Search for the base name without the extension too, since a component can be registered under its name as well as imported by path.

## Component name inside the file

Renaming the file does not rename the component declared inside it. Check whether the component has an explicit name, and whether other code refers to it by that name, for example in keep-alive includes, in devtools filters, or in tests that find components by name. If the name inside the file still says the old thing, decide whether to align it. Aligning it is cheap, but only do it together with a search for every usage of the old component name.

## Git history

A plain rename keeps history if the content stays close to the original, and git will normally detect it. When reading blame or log for the heatmap, follow the file across the rename, otherwise the history seems to start at the rename commit. Old commits and old review comments refer to the previous path, so keep that in mind when you read them.

## Naming rule for this note and others

Always call the component `teacher-dashboard` in notes, tickets and commit messages. Do not use a nickname such as "the teacher screen" or "main dashboard". Searches over the notes depend on the exact name, and a different spelling hides the note from the person who needs it.

For the file, use the full path written above. A short name like "the heatmap view" is fine in talk, but in a note write the path so that a search finds it.

## Things this note does not cover

It does not describe the data format that the backend sends to the heatmap. It does not describe the colour scale or the thresholds. It does not cover the Celery tasks that prepare the progress figures, or how the suggestions are produced. Those belong in their own notes. If one does not exist yet, write it, and link it from here rather than expanding this note.

I also did not verify, while writing this, every place in the repository that might still mention the old path. The list above is what to check, not a record of what was checked.

## Open points

- Whether any external link, such as a bookmark in a teacher guide or a help page, used the old view name. Those would be outside the repository and need a separate look.
- Whether the component's inner name was aligned with the file name. Check the file.
- Whether the tests were renamed with the view. If not, rename them so the test file and the view stay easy to pair.

## Quick answers

Where is the class heatmap of teacher-dashboard? In `frontend/src/views/ClassHeatmap.vue`.

What was it called before? It was in frontend/src/views/ClassOverview.vue, and that path no longer exists.

Is this a behaviour change? Not as far as this note knows. It is a rename of the view file.

What breaks if a reference is missed? Most likely a build error about a module that cannot be found, or a test that cannot import the view.

Which earlier note does this one replace? The note about the teacher dashboard class heatmap. Use this one instead, and treat the old path in that note as out of date.
