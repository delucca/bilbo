---
id: 01KYH6V9VBBSS7RZBBRXQN045P
created: 2026-07-27T04:15-03:00
---

# manifest-publisher: Safari and the CODECS attribute in master playlists

Testing showed that Safari refuses master playlists that lack the CODECS attribute, while other players tolerate the omission. This note records what we saw, why it matters for manifest-publisher, what we think is going on, and what we would check before changing anything. It is research, not a decision: the fix direction is stated at the end, but the details of how to derive the value are still open.

The short version: manifest-publisher writes the master playlist that points at each rendition in the adaptive-bitrate ladder. If a variant entry in that playlist has no CODECS attribute, Safari will not play the stream at all. It does not fall back, it does not guess, it does not play the first variant. Other players we tried were happy with the same file. That asymmetry is exactly why the problem hid for so long: everyone who spot-checked output used a desktop player that was forgiving.

## What we observed

The symptom was a stream that worked everywhere except Apple's browser and the native player on Apple devices. The same master playlist, served from the same bucket path, played fine in the other players we had to hand, and failed in Safari. The failure was immediate and silent from the point of view of the person testing: the player never got to the point of fetching media segments. There was no partial playback and no quality switching glitch, just a player that gave up on the master playlist.

We then compared the failing playlist with one that Safari accepted. The only meaningful difference in the variant entries was the presence of the CODECS attribute. After adding it by hand to a test copy of the playlist, Safari loaded the stream and switched between renditions normally. Removing it again brought the failure back. That is a clean A/B on one variable, repeated enough times that we stopped doubting it.

Things that did not matter in our testing: the order of the variants, whether the bandwidth values were exactly right, and whether the resolution attribute was present. We did not test every combination, so treat that as "did not obviously matter" rather than proven irrelevant.

## Why the other players hide it

Most non-Apple players treat the codec string as a hint. If it is absent, they open the first segment or the initialization data of a rendition, inspect the actual streams, and decide whether they can decode it. That costs a little startup time but works. Some of them also simply attempt playback and let the decoder complain later if something is wrong.

Safari takes the opposite stance. It wants to know up front, from the playlist alone, whether it can play each variant, so that it can pick a compatible set before downloading anything. Without the codec information it has nothing to base that decision on, and it rejects the playlist. We have not found a setting that relaxes this. Our understanding is that this is a deliberate part of how Apple's HLS implementation works, and that the HLS authoring guidance expects the attribute to be present on variant entries. We have not re-read the specification text in this session, so do not quote us on the exact wording.

The practical consequence is that "plays in the player I have open" is not a valid acceptance test for a master playlist. Any change to manifest-publisher that touches variant entries needs a check against Safari specifically.

## Where this sits in the pipeline

ReelForge takes an uploaded video, transcodes it with FFmpeg into a ladder of renditions, and packages them for streaming. The work is orchestrated by AWS Step Functions, and the outputs land in AWS S3. manifest-publisher is the last step of that chain: once every rendition has been transcoded and packaged, it assembles the master playlist and publishes it next to the rendition playlists so that a player has a single entry point.

Because it runs last, it is the component that knows the whole ladder, but it is not the component that knows the codecs. The encoder settings were chosen upstream, and the actual encoded streams were produced by FFmpeg. manifest-publisher is written in Rust, and as far as playlist generation goes it treats each rendition as a record of bandwidth, resolution and a location. Codec information is not currently part of that record in any way we could rely on. That is the gap.

Media operations teams at independent publishers are the users, and they usually open the published stream in whatever browser is on their desk. Many of them are on Apple hardware, so this bug would reach real users quickly once a publisher hit it. It is worth fixing before it shows up as a support ticket.

## What the attribute has to contain

The CODECS attribute carries a quoted, comma-separated list of codec identifiers for everything a player needs to decode in that variant: the video codec and the audio codec when the rendition has both. The identifiers follow the usual conventions for the codec family, and they include profile and level information for video, not just the codec family name. That is the part that makes it awkward. A string that names the right family but the wrong profile or level is not safe to assume correct, and we do not yet know how strictly Safari validates it beyond presence. We only tested presence versus absence, with correct values.

Because the value describes the actual encoded stream, it should come from the encode, not from a hand-maintained table of what we think the presets produce. A table drifts the moment someone adjusts a preset, and the failure mode of a wrong value is probably worse than the failure of a missing one, since it may make a player reject a variant it could have played.

Audio-only renditions, if the ladder ever includes them, would need an entry with only the audio codec. Renditions with video and audio muxed need both. We should not assume every rendition in a ladder has the same codec string, because profile and level typically change with resolution.

## Ways to get the value

There are three plausible sources, and we have not picked one.

First, read it from the encoded output. FFmpeg's probing tools can report the codec, profile and level of each stream in a produced rendition. A step after packaging could probe each rendition and hand the result to manifest-publisher. This is the most accurate option and tracks preset changes automatically. The cost is an extra probing step per rendition and a new contract between that step and manifest-publisher.

Second, have the transcoding step emit the codec string as part of its own output record, since it knows exactly what it asked FFmpeg to produce. This is cheaper at runtime, but it only reflects what was requested, not necessarily what was produced. If the encoder adjusts level automatically to fit the content, the requested value and the real one can differ.

Third, derive it inside manifest-publisher from the rendition settings it already sees. This is the least work and the most fragile. We lean against it for the reasons in the previous section.

Our current lean is the first option, with the second as a cheaper fallback if probing proves slow or awkward inside the Step Functions flow. That lean is a recommendation, nothing has been built.

## Risks and things to watch

A few things could go wrong when this is fixed, and they are worth writing down now.

Existing published streams already have master playlists without the attribute. Fixing the code only helps new publishes. Either those streams get republished, or a one-off job regenerates their master playlists from the rendition playlists that are already in S3. The second is only possible if the codec information can be recovered from the stored renditions, which the probing approach would allow. Someone should decide whether old content matters before the fix ships, because the answer changes how much tooling we need.

Caching is the other trap. Master playlists are small and often cached by a CDN or by the browser. After republishing a corrected playlist, a tester may keep seeing the old failure for a while. When checking a fix in Safari, make sure the playlist being fetched is the new one, and do not conclude the fix failed on the strength of one cached attempt.

Third, a wrong value. If probing yields a string in a form that differs from what players expect, we could convert a working stream in tolerant players into one that fails there too. Test the fix in more than just Safari.

## Test plan and open questions

Acceptance for the fix should include a Safari playback test on a freshly published stream, with the ladder switching renditions under throttled bandwidth, because that exercises the codec decision per variant. It should also include a run in at least one tolerant player, to confirm we did not regress them. A unit-level test in manifest-publisher can check that every variant entry in the generated master playlist carries the attribute, which would catch the omission without needing a browser. That test is cheap and should exist regardless of which source of the value we choose.

Open questions we could not settle from testing alone:

- How strictly does Safari validate the content of the value, as opposed to its presence? We only know presence matters.
- Does Safari behave differently on the desktop browser and on the native player on phones and tablets? Our testing treated them as the same and saw the same failure.
- Should manifest-publisher refuse to publish a master playlist when it lacks the codec information for any variant, instead of silently writing a playlist that one major player rejects? We lean toward failing loudly, since a failed Step Functions execution is much easier to notice than a stream that quietly breaks for one group of viewers.
- If the ladder mixes codec families in the future, how should variants be ordered and grouped? Not a problem today, but the attribute becomes more important then.

Until those are answered, the working rule for anyone touching manifest-publisher is simple: every variant entry in the master playlist gets a CODECS attribute, and nobody calls a playlist change done until Safari has played it.
