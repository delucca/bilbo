---
id: 01KBPTVXMRZ8P4KGXQCK5J11D1
created: 2025-12-05T05:43-03:00
---

# sandbox-image weekly recap

Most of this week on sandbox-image went to making the image build less fragile and figuring out why a few verification runs behaved differently inside the container than on a laptop. Nothing here is final. This is what I remember and what is still open, written fast so the next session doesn't redo the digging.

## What moved

The base layer got cleaned up. We had a few leftover packages in sandbox-image that nobody could explain, so I traced each one back to the test runner or the git tooling and dropped the ones that had no owner. The image is smaller and the build feels quicker, though I haven't measured it properly, so don't quote that anywhere.

I also reordered the Dockerfile so the slow, rarely changing steps come first and the Node dependency install comes later. Cache hits are better on repeat builds in CI. The GitHub Actions workflow that builds the image still rebuilds more than it should when only a small file changes; I suspect the cache key is too broad but haven't confirmed.

The non-root user setup was tidied. Working directories are now created with the right ownership at build time rather than fixed up at start, which removed a class of permission errors when PatchPilot mounts a checked-out repository into the container.

## What broke or surprised us

Targeted tests that rely on a writable home directory failed inside the sandbox while passing outside it. The cause was the container filesystem being read-only in places the package managers expect to write cache data. The workaround was pointing the caches at a scratch location. It works, but I'm not convinced it's the right long-term answer, because it hides how much a given upgrade really touches.

Network access was the other source of confusion. Some upgrades pull in packages with install scripts that try to reach the outside world. In the sandbox those fail, and the failure looks like a test failure in the PR summary. That is misleading for reviewers. The SQLite state that records run outcomes has no way to tell "install blocked" from "test failed", so both show up the same way.

Native modules are still the weak spot. Anything that compiles on install needs build tools that we deliberately kept out of the image, and each case so far has been handled by hand.

## Open items

- Decide whether to keep a second, heavier variant of sandbox-image for native module upgrades, or keep one image and accept that those PRs get flagged as unverified.
- Record install-blocked as its own outcome in the run history so reviewers can tell it from a real failure.
- Confirm whether the workflow cache key is the reason for needless rebuilds.
- Look again at the scratch cache approach and check whether stale cache contents can leak between runs for different repositories. I think each run gets a fresh location, but I haven't verified it.
- Write down which tools in the image are there on purpose, so the next cleanup doesn't have to reverse-engineer it.

## Next week

Start with the outcome classification, since it affects what reviewers see and is the smallest change. Then try the cache key fix in a branch and watch a few runs. Leave the two-image question until there is a clearer picture of how many upgrades actually need native builds. Ask the platform engineers who use PatchPilot what they expect to happen when a sandbox cannot verify something.
