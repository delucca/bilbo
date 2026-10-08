---
id: 01JVM7CWKJ6AYR4W1HRVAAEBCF
created: 2025-05-19T09:13-03:00
---

# teacher-dashboard design

teacher-dashboard is the front end teachers open to see how a class is doing against curriculum standards and what ClassroomCompass suggests they set next. It is a Vue.js 3 single-page app. All class state lives in one Pinia store, `useClassStore`. This note says what the app is for, how the pieces fit, and where the traps are. It is written from the design as it stands, so check the code before relying on a detail.

## Purpose

A secondary school teacher has several classes and a lot of students. They need to see, per class and per student, which curriculum standards are met, which are shaky and which have not been touched. The dashboard shows that and puts the suggested next exercises beside it. It does not compute progress or suggestions. It displays what the backend gives it and lets the teacher act on it.

## Shape of the app

teacher-dashboard is a single-page app. There is one shell, a router, a set of views and a set of shared components. Views read from the store and dispatch actions to it. Components get data through props and send events upward. Almost nothing outside the store keeps data that outlives a view.

## Where class state lives

Class state sits in the Pinia store `useClassStore`. That means the selected class, the student list for it, the progress per standard, and the current exercise suggestions. If a view needs any of these, it reads them from `useClassStore` and does not fetch on its own. This was chosen so two views on screen at once cannot disagree about which class is selected.

```js
useClassStore()
```

## Why one store

Earlier thinking had each view load its own data. That produced flicker when the teacher switched class, because one panel updated before another. A single store gives one switch point: change the selected class once, and everything derived from it follows. The cost is that the store grows. Keep it to class state and resist putting every bit of UI state in it.

## What belongs in the store and what does not

In the store: the selected class, students, progress data, suggestions, and the loading and error flags for those. Not in the store: whether a dropdown is open, the text in a search box before it is submitted, scroll position, a tooltip. Those stay local to the component. If a piece of state is only meaningful to one component, it is local.

## Data flow

The teacher picks a class. A store action sets the selection and then asks the backend for the students, progress and suggestions of that class. Responses fill the store. Views react. Errors set a flag in the store and the views show a plain message. There is no optimistic update for progress, since progress is derived on the server and the dashboard should not guess it.

## Talking to the backend

The Django backend serves the data. The dashboard calls it over HTTP and treats it as the source of truth. Progress comes from what students have done, suggestions come from the recommendation side that uses scikit-learn, and long jobs run in Celery. The dashboard never knows about those internals. It sees finished results, or a state that says they are not ready.

## Search

Searching for a student, a standard or an exercise goes through the backend, which uses Elasticsearch. The dashboard sends the typed text and shows what comes back. Do not filter large lists in the browser to imitate search; results would differ from what the backend returns elsewhere.

## Progress view

The progress view is the main screen. It lays standards against students and colours each cell by how well the standard is met. The colours must be readable without relying on hue alone, because teachers project this in classrooms and some students or colleagues see colour differently. Each cell also carries text or a pattern.

## Suggestions panel

The suggestions panel lists next exercises, for the class or for a single student. A teacher can accept one, which assigns it, or dismiss it. Dismissal should be remembered so the same suggestion does not return at once. The panel only presents; ranking is the backend's job. If suggestions are not ready, show that, not an empty list that looks like there is nothing to suggest.

## Switching class

Switching class is the riskiest moment for stale state. When the selection changes, the old class data must be cleared or marked stale before the new data arrives. A late response for the previous class must not overwrite the new one. The store should check that a response still matches the current selection before applying it.

## Loading and empty states

Three states are easy to confuse: loading, loaded but empty, and failed. A new class with no student activity is loaded and empty, and that is normal early in a term. Each state gets its own wording. Do not show a blank grid for any of them.

## Errors

Errors are shown in the view that needed the data, in plain language, with a retry. The store keeps the error flag. Do not log student names into the console or into any error report. This is student data about minors, so treat it with care in every debugging aid.

## Testing

Store actions are the best place for tests: select a class, feed a fake response, check the state. Test the stale-response case on purpose. Component tests stay light and check that each of the loading, empty, failed and loaded states renders something distinct. Keep fixtures free of real student data.

## Gotchas

- Reading class state from anywhere except `useClassStore` creates a second copy that will drift.
- Destructuring store state into plain variables drops reactivity. Use the store's getters, or convert refs properly.
- A late response after a class switch can overwrite newer data if the check is missing.
- An empty suggestions list can mean not ready, not none. Do not read it as none.

## Open questions

- Whether the store should split into smaller stores once more than class state needs sharing.
- How much of the progress grid to load at once for very large classes, and whether to page it.
- Whether dismissed suggestions should be remembered per teacher or per class.

## Pointers for later work

Before changing how state is shared, read how `useClassStore` is used across the views and list every reader. Before adding a new data type for a class, decide whether it is class state; if it is, put it in `useClassStore` and follow the same pattern for loading, empty and failed.
