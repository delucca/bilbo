---
id: 01M35K0ECKM4KD649NHJEDWFB0
created: 2026-09-22T19:15-03:00
---

# teacher-dashboard crashes on classes with no enrolled students

The teacher-dashboard breaks on any class that has no enrolled students. The browser console shows `Uncaught TypeError: Cannot read properties of undefined (reading 'standards')` and the panel stays blank or half drawn. Optional chaining avoids it. Written down so nobody burns an afternoon on the backend again.

## Symptom

A teacher opens a class that was just created, or one whose roster was emptied, and the teacher-dashboard does not render the progress view. The console error is `Uncaught TypeError: Cannot read properties of undefined (reading 'standards')`. Other classes on the same page work fine, which makes it look random at first.

## Trigger

The only trigger found so far is a class with no enrolled students. Typical cases are a new term before rosters are imported, a class a teacher made for planning, and a class where every student was moved out.

## Cause

The Vue code in teacher-dashboard reads the standards off a per-student or aggregate object as if it always exists. With no students, nothing builds that object, so the value is undefined and the property read on it throws. The data is not corrupt. The API simply has nothing to return for that class, and the front end assumed it would.

## Why it looks like a backend bug

The first instinct is to blame Django or Celery, because the progress numbers come from there. The response for an empty class is valid, just empty. Do not spend time on the Celery side or on the scikit-learn suggestions for this one. The failure is in the browser.

## Fix

Use optional chaining wherever the dashboard reaches into the possibly missing object before reading standards. Pair it with a sensible fallback, such as an empty list, so the template still loops over something. Optional chaining alone stops the crash. The fallback keeps the layout from collapsing.

## Better UX

An empty class should show a plain message saying no students are enrolled yet, not an empty grid of standards. That is a nice-to-have. The crash fix does not depend on it.

## What not to do

Do not paper over it by having the backend invent a placeholder student. That pollutes progress aggregates and the exercise suggestions. Do not wrap the whole component in a try/catch either, since that hides real errors elsewhere.

## How to reproduce

Create a class with no students, or remove all students from an existing one. Open it in the teacher-dashboard with the browser console visible. The error appears on load. Re-adding one student makes it go away, which confirms the trigger.

## Checking the fix

Load an empty class and confirm the console is clean and the page renders. Then load a class with students and confirm nothing changed. Also try a class that had students and had them all removed, since cached state can differ from a fresh class.

## Related spots to audit

Other components that read standards, progress or suggestions from class-level data may have the same assumption. Worth a quick look wherever a deep property chain starts from data that comes from the API. Elasticsearch-backed search views are less likely to be affected, but check anything that builds on the same class object.

## Tests

Add a front-end test that mounts the dashboard with an empty class payload and asserts it renders without throwing. Without it, someone will remove the optional chaining during a cleanup and the bug returns.

## Notes for later sessions

If the error text differs, for example a different property name in the parentheses, it is a different bug of the same family: a missing object read without a guard. The same approach applies.

## Status

The cause and the fix are understood. Keep the guard in place when refactoring the dashboard.
