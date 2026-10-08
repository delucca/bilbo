---
id: 01KJHSV0T9CJ8RESE7G4SZNY4G
created: 2026-02-28T06:38-03:00
---

# teacher-dashboard: Vue CLI to Vite migration plan

The teacher-dashboard front end still builds with Vue CLI. The plan is to move it to Vite, and the migration has to be finished by 2026-11-30. This note holds the order of work and the things to watch.

## Goal

Replace the Vue CLI build and dev server of teacher-dashboard with Vite. Behaviour for teachers should not change. The deadline is 2026-11-30.

## Why move

Vue CLI is in maintenance mode and its dev server is slow on the larger pages. Vite starts faster and rebuilds quicker, which matters when working on the progress charts and the exercise suggestion views.

## Deadline

Everything lands by 2026-11-30. Work back from that date and leave the last week for fixes found in staging, not for new migration steps.

## Scope

In scope: the build config, the dev server, environment variable handling, asset handling and the test runner hookup for teacher-dashboard. Out of scope: any change to the Django API, the Celery tasks, the scikit-learn recommendation code or the Elasticsearch indexing.

## Step 1: Inventory

List what the current Vue CLI setup does: custom webpack tweaks, aliases, proxy rules to the Django backend, and any plugins. Anything not on this list will be forgotten and break later.

## Step 2: Add Vite alongside

Install Vite and the Vue plugin next to the existing setup. Get a bare build working before touching the old config. Keep both runnable until the switch.

## Step 3: Config parity

Port aliases, the dev proxy to Django and the public path. Check that the Django templates or static serving still find the built bundle, since the output file names may differ.

## Step 4: Environment variables

Vue CLI and Vite name and expose env variables differently. Go through every place the code reads one and rename or rewire it. This is the most likely source of silent bugs.

## Step 5: Tests and lint

Make sure unit tests and lint still run against the new setup. Fix any imports that relied on webpack-only behaviour.

## Step 6: CI and deploy

Update the CI build step and the deploy script to call the new build command. Confirm the artefact lands where Django expects it.

## Risks

- Webpack-specific loaders or require calls in old components.
- Bundle path changes breaking the Django integration.
- Env variables undefined at runtime without any build error.

## Check commands

```
git switch -c teacher-dashboard-vite
```

## Done when

Dev server, production build, tests and CI all use Vite, the Vue CLI config and dependencies are removed, and a teacher can open the dashboard in staging and see progress and suggested exercises as before. Target: 2026-11-30.
