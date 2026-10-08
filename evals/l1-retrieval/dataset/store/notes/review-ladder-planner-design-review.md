---
id: 01KQS45C8ZXHQ4FZZZG4YQ38W7
created: 2026-05-04T06:12-03:00
sources:
  - "doc: Ladder design review"
---

# ladder-planner design review: rung spacing

Design review of the ladder-planner, written up so the next session does not have to reconstruct it. The main conclusion is short: adjacent rungs should differ by at least `1.5x` in bitrate to avoid redundant renditions. Everything below is the reasoning around that, what it means for the planner code and the Step Functions flow, and what is still open. This is a review note, not a spec. Where I say the planner should do something, that is the outcome of the review and not necessarily what the code does today.

The ladder-planner is the part of ReelForge that decides which renditions a given upload gets. It takes what we know about the source (resolution, frame rate, measured complexity, audio layout) and the publisher's profile, and returns a list of rungs. Each rung is a target resolution plus a target bitrate plus codec settings. The transcode workers then turn that list into FFmpeg invocations, and the packaging step writes HLS playlists that point at the results in S3. The ladder-planner itself does no encoding. It is pure planning, which is why it is cheap to review and cheap to change, and also why mistakes in it get multiplied downstream: every extra rung is a full encode, storage, and a line in the master playlist.

## What the review concluded

The rule: adjacent rungs should differ by at least `1.5x` in bitrate. Sort the ladder by bitrate, take any two neighbours, and the higher one must be at least `1.5x` the lower one. If it is not, one of them is redundant and the ladder-planner should drop or merge it.

Things the review agreed on:

- The ratio is measured on the target bitrate the ladder-planner assigns, not on the bitrate the encoder later produces. Encoders overshoot and undershoot, and the planner cannot know the outcome ahead of time. Using the target keeps the rule deterministic and testable.
- The ratio applies to neighbours in the final sorted ladder, not to every pair. If neighbours satisfy `1.5x`, then every non-adjacent pair is farther apart still, so checking neighbours is enough.
- The rule is a floor on spacing, not a target. A ladder with wider gaps is allowed. Wide gaps cost viewers some quality at switch points, but they are not redundant. The review did not set a ceiling, and nobody argued for one strongly enough to put it in.
- The rule applies per codec family within a ladder. Different codec families are different ladders as far as spacing goes, because a player picks one family and then switches only inside it. Comparing an H.264 rung against an HEVC rung at similar bitrate is not a spacing question.
- The rule applies to the video renditions only. Audio-only renditions are planned separately and the review did not touch them.

What the review did not decide: whether `1.5x` should be a hard constant or a profile setting. The leaning was to keep it as a named default in the ladder-planner, overridable per publisher profile, but only toward a larger ratio. Going below `1.5x` reintroduces the redundancy the rule exists to remove, so an override that lowers it should be rejected at profile validation time rather than silently accepted. This was a leaning, not a vote. Check with whoever owns the profile schema before wiring it.

## Why redundant renditions are a problem

The reasoning, since the number alone will get questioned later.

An adaptive player picks a rendition by estimating throughput and choosing the highest rung that fits under it with some safety margin. Two rungs close in bitrate give the player almost no new choice. At a given throughput estimate it will land on one or the other, and the viewer-visible difference between them is small, often below what anyone can see. Meanwhile each rung costs us real things:

- Compute. Every rung is a separate encode. For long sources this is the dominant cost of the whole pipeline, and independent publishers feel it directly.
- Storage. Every rung is a full set of segments in S3, kept for as long as the asset lives. Near-duplicate rungs double up storage for no gain.
- Playlist size and player behaviour. The master playlist grows, some players probe more variants than they need to, and switching logic can start to oscillate between two nearly identical choices when the throughput estimate is noisy. Oscillation is worse than a stable slightly-lower rung, because every switch risks a rebuffer or a visible quality step at the segment boundary.
- Cache fragmentation at the CDN. Viewers spread across more variants means each variant has a lower hit rate. With near-duplicates this fragmentation buys nothing.
- Operator confusion. Media ops people read these ladders when debugging complaints. A ladder with rungs that are practically the same makes it hard to say which one a viewer was on and whether it mattered.

The flip side was raised in the review and is worth keeping: gaps that are too large hurt too. If neighbours are very far apart, a throughput dip forces the player down a long way and the viewer sees a big quality drop where a middle rung would have softened it. That is why the rule is phrased as a minimum ratio and why the ladder-planner should still try to fill space sensibly when the gap is large. The review settled on `1.5x` as a reasonable floor because it is large enough that two rungs are distinguishable in practice and small enough that it does not force sparse ladders. It is a judgment call and we said so. If real viewing data later shows the floor is too tight or too loose, change it with data, not with taste.

## How the planner should apply it

The order of operations matters, because the spacing check is easy to get subtly wrong if it runs at the wrong point.

First, the ladder-planner builds a candidate set of rungs from the source and the profile. Candidates come from the resolution steps the profile allows, capped by the source: never upscale beyond the source resolution, and treat a source with a low frame rate as a reason to avoid rungs that assume high frame rate. For each candidate it assigns a target bitrate from the profile's bitrate model, adjusted for measured complexity of the content. Simple content such as talking heads and slides gets lower targets, busy content such as sports gets higher ones.

Second, sort candidates by target bitrate, ascending.

Third, walk the sorted list and enforce the spacing rule. Start from the lowest rung, which always stays. For each next candidate, compare its bitrate against the last rung that was kept, not against the previous candidate. If the ratio is at least `1.5x`, keep it. If not, it is a conflict and must be resolved. This detail matters: comparing against the previous candidate instead of the last kept rung lets a chain of slightly increasing candidates all survive, each passing against its neighbour that was itself dropped. The check has to be against what is actually in the output.

Fourth, conflict resolution. When a candidate is too close to the last kept rung, the review preferred these choices in order:

- Drop the candidate with the lower value to the viewer. In practice this usually means prefer to keep the one whose resolution is a standard step in the profile, and drop the one that is an odd in-between size.
- If both are standard, keep the higher bitrate one when it is the top of the ladder, because the top rung defines the best quality we offer, and keep the lower one otherwise, because lower rungs protect viewers on weak connections.
- If neither rule decides, drop the later one and log which rule fired. A deterministic tiebreak is better than a clever one.

A wrinkle the review spent time on: dropping a rung can open a gap larger than intended elsewhere. The ladder-planner should not then add something back to compensate, because that is how you get loops. The rule is one pass, sorted, against last kept. The result may be a bit uneven and that is acceptable.

Fifth, validate the result. After the pass, run an independent check over the final ladder that asserts every adjacent pair satisfies the ratio. This looks redundant, and it is, on purpose: the check is written separately from the pass so a bug in one does not hide a bug in the other. If the check fails, the ladder-planner returns an error instead of a ladder. A planner that emits a ladder violating its own rule is worse than one that fails loudly, because the bad ladder will be encoded and stored before anybody sees it.

Edge cases that came up:

- A very small source. After capping by source resolution there may be only a handful of candidates, possibly one. A single-rung ladder is valid. The spacing rule has nothing to say about it. Do not pad it to look like a ladder.
- Two candidates with the same target bitrate but different resolutions. The ratio is one, which violates the rule, so one gets dropped. This is the cleanest case of redundancy and the tiebreak above handles it.
- Profiles that explicitly list rungs. If a publisher profile pins a fixed list of rungs, the ladder-planner should still run the check. If the pinned list violates the rule, the profile is invalid, and the error should say which pair is too close. Do not silently thin a ladder the publisher wrote by hand. Fail at validation, tell them which two rungs, and let them fix the profile.
- Variable frame rate sources. Complexity measurement can be noisy on them, which moves target bitrates around between runs. Because the spacing rule works on assigned targets, noise in the input can change which rung is dropped. This is a determinism concern, listed below under open questions.
- Bitrate models that scale by a multiplier. If a profile's model happens to produce ratios just under the floor between standard resolution steps, whole sets of rungs will be dropped on every upload and the publisher will wonder why their ladder is thin. That is a profile design problem, but the ladder-planner log should make it obvious, not leave it to be discovered by counting renditions.

## Where this touches the rest of ReelForge

The ladder-planner sits at the start of the Step Functions workflow that handles an upload. Upstream, the ingest side hands over a source object in S3 and some probe results. Downstream, the state machine fans out one transcode task per planned rung and then joins them for packaging. Because the fan-out is driven by the planner output, the spacing rule directly controls how wide that fan-out is. Fewer redundant rungs means fewer parallel tasks, lower cost, and a shorter tail on the slowest task.

A few consequences worth writing down:

- The planner output is the contract between planning and encoding. If a rung is in the list, a worker will encode it and packaging will expect it. There is no later stage that prunes. So the spacing rule must be enforced in the ladder-planner and nowhere else. Do not add a second pruning step in the state machine or in packaging, it would hide planner bugs and create two sources of truth for what the ladder is.
- HLS master playlist ordering. Packaging writes variants into the master playlist, and the planner's sorted order is the natural source for that ordering. Keep the planner's output sorted by bitrate ascending and let packaging rely on it, or sort in one place only. The review noted that two places sorting differently is a classic way to get a playlist that players handle inconsistently.
- Retries and re-planning. If a workflow is retried, the ladder-planner may be invoked again for the same asset. The result should be identical for the same inputs, otherwise a retry can produce a different ladder than the first attempt and leave orphaned renditions in S3 from the earlier plan. The planner therefore has to be a pure function of its inputs: source facts, profile, and planner version. No clock, no randomness, no reading of mutable external state.
- Planner versioning. If the spacing rule or its default changes, ladders for new uploads change. Existing assets keep the ladder they were encoded with. Re-encoding old assets is a separate, deliberate action, and the review agreed that a change to the default floor should never trigger it implicitly. Record the planner version alongside the plan so a later reader can tell which rules produced a given ladder.
- Upload-side validation. There is a related note about what the upload gateway must do when a source is revised after it has been accepted: [[upload-gateway-must-reject-revised]]. It matters here because if a source can change after planning, the plan can be stale, and the spacing check would have run against facts that no longer hold. The ladder-planner assumes the source it was given is the source that will be encoded. Read that note before changing how probe results are passed in.
- FFmpeg settings. The spacing rule says nothing about how a rung is encoded, only about what bitrate it targets. Rate control mode, buffer size and keyframe alignment across rungs are decided elsewhere. One thing to keep in mind is that rungs must stay segment-aligned for switching to work, and nothing in the spacing rule interferes with that. If someone proposes to meet the ratio by tweaking encoder settings rather than dropping a rung, say no: the rule is about the plan.

On observability: each planning run should log the candidate list, the kept list, every dropped rung with the reason, and the planner version. When a publisher asks why their ladder has fewer renditions than expected, this log should answer the question without anyone reading code. The reason strings for spacing drops should name the two rungs involved and say that the spacing floor was the cause, in the same words each time so they can be searched.

## Open questions and follow-ups

Things not settled, in rough order of importance.

Determinism under noisy complexity input. The target bitrates depend on a complexity measure, and the spacing decision depends on the targets. If the measure is not stable between runs on the same file, the same upload could get different ladders. The review suggested quantizing the complexity measure into coarse buckets before it feeds the bitrate model, so tiny differences do not flip a spacing decision. Not done. Someone should check how stable the measure really is before adding complexity to fix a problem that may be small.

Profile override semantics. As said above, overrides should be allowed only to raise the floor. Need to confirm with the profile schema owner and decide where validation lives. If it lives in the ladder-planner, it needs to reject a lowered floor with a clear message. If it lives in the profile loader, the planner can trust its input but should still keep its own final check.

The top of the ladder. The tiebreak prefers keeping the higher rung when the conflict is at the top. A reviewer asked whether that could leave the top rung bumping up against the source's own bitrate, making the top rung nearly a passthrough. Not a spacing question as such, but the two interact: if the top is capped by the source and the next rung down is within the floor, we drop one of them and end up with a different top than the profile intended. Decide whether the cap should be applied before or after spacing. Current thinking is before, so the spacing pass sees the real candidate set, but this was not confirmed.

Multiple codec families in one upload. Spacing is per family. Make sure the ladder-planner groups by family before the pass, and make sure the final validation also groups, or it will report false violations when two families have similar bitrates. Easy to get wrong in tests, so write a test with two families whose bitrates interleave.

Audio. Audio renditions were out of scope. If someone wants a spacing rule for audio, it needs its own review; audio bitrates are small and the ratio logic may not carry over, because the perceptual steps are different.

Measurement after the change. The review wanted evidence that the rule helps rather than only reasoning that it should. Things worth measuring once it ships: average number of rungs per asset before and after, total encode time and storage per asset, switch counts per viewing session, and rebuffer rate. If switch counts drop and rebuffers do not rise, the rule is doing its job. If rebuffers rise, suspect that dropped rungs opened gaps that are too large and revisit whether a ceiling is needed after all.

Tests to write, so the rule stays honest:

- A ladder where every neighbour is exactly at the floor. This tests the boundary and the comparison operator. Greater-or-equal is the intent, so exactly at the floor is kept.
- A ladder with a chain of slowly increasing candidates, each close to the one before. This is the case that catches comparing against the previous candidate instead of the last kept one.
- Two candidates with identical bitrate. One must be dropped, deterministically, and the same one every run.
- A source small enough that only one rung survives capping. Result must be valid.
- A pinned profile that violates the floor. Must fail validation and name the pair.
- Same inputs twice. Output must be byte-identical.
- Interleaved codec families. No false violations.
- Shuffled candidate order on input. Output must not depend on it.

A property test is a good fit here: generate random candidate sets, run the ladder-planner, and assert both that the output satisfies the floor and that nothing was dropped without a recorded reason. The second assertion is the one people forget.

Last point, for whoever picks this up. Resist the urge to make the ladder-planner smarter in the same change that adds the spacing rule. The point of the rule is that it is simple and checkable. Add it, add the independent validation, add the log lines, ship it, then look at the data. The review explicitly preferred a plain rule that operators can explain to a publisher in one sentence over a tuned heuristic that nobody can predict.
