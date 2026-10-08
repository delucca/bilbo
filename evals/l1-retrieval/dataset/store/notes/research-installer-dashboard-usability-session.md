---
id: 01KWV2CTDAF6K17Q2YK1FXQBFT
created: 2026-07-06T03:38-03:00
sources:
  - "doc: Installer usability session notes"
---

# Installer-dashboard usability session: tariff editor confusion

A usability session with 9 installers found that the tariff editor in installer-dashboard was the most frequent point of confusion. This note keeps what that session told us, what we think it means, and what is still open, so the next person who touches the screen does not have to rebuild the picture from memory. The component is installer-dashboard, the Svelte front end that home energy installers use to look at a customer's solar forecast, check the battery schedule, and set up the time-of-use tariff that the scheduler charges against. Internally it is also called sunboard, which matters when you search chat history, old tickets or branch names (see the naming section below).

The short version: installers could read the forecast and the schedule without much trouble. They stumbled when they had to describe a tariff, and they stumbled in the same places, over and over. That is a stronger signal than any single complaint, and it is why the tariff editor goes to the top of the list.

## Naming: sunboard and installer-dashboard

sunboard is the internal codename of installer-dashboard. They are the same thing: one Svelte application, one set of screens, one owner. In this note the component is always written as installer-dashboard, and sunboard appears only when I mean the codename as people say it out loud or as it shows up in older material.

Why this is written down: during the session notes and in some of the earlier discussion, people used sunboard, the dashboard, the installer app and the installer UI interchangeably. A later reader searching for feedback on "sunboard" and finding nothing under "installer-dashboard", or the other way round, might conclude that no feedback exists. It does. If a search on one name comes up empty, try the other before concluding anything.

A few practical consequences:

- Anything an installer sees on screen should not use the codename. Installers know the product by its customer-facing name, and sunboard is internal. If sunboard leaks into a label, an error text or an email template, treat it as a small bug.
- When filing follow-up work about the tariff editor, name the component installer-dashboard in the title and mention sunboard in the body so both searches find it.
- The codename is not a separate service. It does not correspond to a different deployment, a different repository or a different team. If someone talks about a sunboard backend, they almost certainly mean the services installer-dashboard talks to, not something with that name.

## What the session was

The session involved 9 installers. They were people who set up and maintain household solar and battery systems as part of their job, so they know what a tariff is, what a battery schedule is for, and what a customer typically asks them. They were not people who had seen the editor before in its current shape, or at least we did not rely on that. The point was to watch working installers try to do ordinary tasks with installer-dashboard and see where they slowed down, hesitated, backed out or asked a question.

The format was task based. Each installer was given a set of realistic situations, such as a customer on a tariff with a cheap overnight window and a more expensive evening peak, and asked to get the dashboard into a state that matched the situation. They were encouraged to think aloud. An observer took notes on where they paused, what they said, what they clicked, and where they gave up or asked for help. Afterwards each installer was asked a few open questions about what felt clear, what felt unclear and what they would change.

What the session was not: it was not a performance test, it was not a test of the forecast accuracy, and it was not a statistical survey. With 9 installers we can say which problems showed up repeatedly. We cannot put a reliable percentage on how many installers in the field would hit each one, and nobody should quote the session that way. Treat it as a strong qualitative signal about where the design is weak.

## Main finding: the tariff editor

The tariff editor was the most frequent point of confusion. More installers had trouble there than at any other screen or control, and the trouble was not scattered: the same few moments in the editor kept producing the same hesitation. Here is how it showed up.

### Describing a tariff in the editor's own terms

Installers think about a tariff the way the customer's bill describes it: a cheap period, an expensive period, perhaps a shoulder period in between, and some days that differ from others. The editor asks for the tariff in a structure that is closer to how the scheduler consumes it. Several installers had to stop and translate from the bill to the editor's structure before they could type anything, and some translated it wrongly the first time. When people have to translate before they can start, they are already uncertain, and the later mistakes pile on top of that.

### Time boundaries

The most common stumbling block was where one period ends and the next begins. Installers were unsure whether an end time was included in the period or was the first moment of the next one. They were also unsure what the editor does with gaps and overlaps between periods. Some assumed a gap meant "no charge" and some assumed it meant "the previous rate continues". A few were not sure whether the editor would warn them if periods overlapped, and one or two entered overlapping periods on purpose to see what would happen, which tells you how little they trusted the editor to tell them.

### Days of the week and seasons

Many tariffs differ on weekdays and weekends, or by season. Installers could not easily tell how the editor wanted that expressed: as separate tariffs, as separate rows within one tariff, or as a setting on each period. Where there was more than one way to do it, they chose differently from each other, and several second-guessed themselves afterwards. This is the sort of thing a good default and a clear preview would fix, and the current editor offers little of either.

### Units and currency

Rates were another source of hesitation. Installers were not always sure what unit a number was expected in, or whether the number should include taxes and fixed charges. Bills often show rates in a different form than the editor expects, and some installers said they would normally keep the bill beside them and do the arithmetic. The editor gave them little help in checking that the number they typed matched the number on the bill.

### Knowing whether the tariff was right

Maybe the most important issue: after entering a tariff, installers did not feel they could tell whether it was correct. There was no view that played the tariff back to them in a form they would recognise from a bill, and the link from the tariff to its effect on the battery schedule was not obvious. People wanted to see "this is what the customer pays at ten in the evening" or "the battery will charge in this window", and they had to infer it instead. The lack of feedback made every earlier ambiguity worse, because a mistake could sit unnoticed.

### Saving and applying

Installers were also unsure what saving does. Does saving change the live schedule straight away, or only the next time the scheduler runs? Is there a draft state? Can a change be undone? Different installers had different assumptions, and a few were visibly nervous about saving on a real customer's system, which is a reasonable instinct that the editor should respect rather than provoke.

## Other friction seen, less often

The tariff editor dominated, but it was not the only thing. These came up less often and are listed so they are not lost. None of them is as well supported as the editor finding.

- Forecast view: installers generally understood the solar output forecast, but some wanted a clearer sense of how uncertain it is on a given day. They asked how much to trust the forecast when the weather is changeable. This is a presentation question more than a modelling one, though the forecasting work in Julia is where any uncertainty information would come from.
- Battery schedule view: the schedule was readable, but a few installers could not tell whether a charge window was driven by the tariff, by the forecast or by a manual setting. Seeing the reason for a decision would help them explain it to customers.
- Device status: some installers wanted to know more quickly whether a site was reporting, since the data reaches the system over MQTT and through Azure IoT Hub and a quiet site can look the same as a site with no activity. They did not use those terms, but they described the symptom: not knowing whether a flat line meant nothing was happening or nothing was arriving.
- History and charts: the time series shown comes from InfluxDB. Installers were mostly happy with the charts, though a few wanted to pick a time range more easily and to compare a day against another day.
- Language and labels: some labels read as internal vocabulary. Installers are used to the language on bills and on inverter and battery datasheets, and the closer our words are to those, the fewer questions we get.
- Navigation: most installers found their way around, but some went to the tariff editor by a roundabout route because they were not sure where tariffs live. That is a small point, and it feeds the larger one: they did not have a clear mental model of where tariff information is kept.

## Interpretation

The reading I would defend is that the tariff editor mixes two jobs that installers see as separate. One is describing what the customer's contract says, which installers do from the bill and in the bill's vocabulary. The other is telling the scheduler how to behave, which is a consequence of the first. The editor presents the second job's structure to someone who is trying to do the first job. That mismatch explains why the confusion repeats at the same points: time boundaries, day types, units and confirmation.

A second reading is about trust. Because there was no playback and no clear statement of what saving does, installers could not close the loop. Confusion that can be resolved in a moment by looking at the result tends to be forgotten; confusion that stays unresolved gets remembered and reported. Some of the strength of the finding comes from that. Even if the editor's structure were perfect, the lack of feedback would still make it feel unclear.

A third point is about who the users are. Installers are practical and time-limited. They do this at a customer's home or between jobs, often on a phone or a laptop in a cramped space. The editor should assume partial attention. Anything that needs the user to hold several pieces of state in their head, such as which period they are editing and what the neighbouring periods are, will be a problem even for a motivated person.

What the session does not tell us: it does not tell us whether the scheduler's tariff model itself is wrong or too limited. Some installers described tariffs that did not fit neatly into any structure, and we cannot tell from the session how common such tariffs are. It also does not tell us how customers, who also use the product, would fare with the editor. Customers are likely to be less expert than installers, so if installers struggle, customers will most likely struggle at least as much, but that is an inference, not something observed.

## What to do about it

These are recommendations in rough priority order. They are my suggestions after the session, not decisions the team has made.

1. Add a plain-language playback of the tariff next to the editor, written in the way a bill would describe it, so that an installer can check what they entered against the document in front of them. This addresses the trust problem and is probably the highest value single change.
2. Make time boundaries explicit in the control itself: show clearly whether a period includes its end, show gaps and overlaps as they are created, and refuse or warn on the ones that cannot be right. A visual timeline of a day, with the periods drawn on it, would likely do more than any amount of help text.
3. Give weekday, weekend and seasonal differences one obvious home in the editor, and make the default the common case. Then test it again with installers to see whether they pick the same structure as each other.
4. State units and what is included in a rate directly beside each rate field, and where it is practical let installers enter the rate in the form the bill uses.
5. Say what saving does. Either introduce a draft state with an explicit apply step, or state clearly that changes take effect at the next scheduling run, and offer a way back. Installers should be able to try something on a live customer system without fear.
6. Show, from the tariff editor, the effect on the battery schedule, even in a rough form, so the connection between the tariff and the charging behaviour is visible.
7. Review labels across installer-dashboard for internal vocabulary, and bring them closer to what bills and datasheets say. This is cheap and can be done alongside the other work.

The smaller items under the less frequent friction section can wait until the editor work is underway, except that the device-status question may deserve its own look, since it touches the data path and not just the screen.

## Open questions and follow-up

- Which tariffs do not fit the current structure? We should collect real examples from installers, not invent them, and decide whether the scheduler's model needs to grow or the editor should say honestly that a tariff is unsupported.
- Does the confusion drop when the changes above are made? The honest way to find out is a second session with a similar group of installers and similar tasks, ideally including a few who did not take part the first time, so we are not measuring familiarity.
- How should customers see the tariff? The customer view might need a lighter version of the same playback, without the editing controls. That is a separate decision and needs its own note once someone looks at it.
- Is there a reason the editor was built in its current shape that this note does not know about? Before restructuring it, talk to whoever built it and to whoever owns the scheduler's tariff handling, since the editor's structure may be tied to constraints on the Julia side that are not visible from the front end.
- Where should these findings live for people outside engineering? This note is for the people building installer-dashboard. If product or support need a version, it should be written for them separately.

## Caveats on the evidence

Nine participants is a small group, and they may not be typical of all installers. They were people willing to take part in a session, which already selects for a certain kind of person. The tasks were set by us, so they reflect what we thought was important. The observers interpreted what they saw, and a different observer might have put the emphasis differently. For these reasons, the ranking of the tariff editor as the most frequent point of confusion is solid as a statement about this session, and it is a good basis for prioritising work, but it should not be turned into a claim about the whole installer population.

It is also worth saying what would change my mind. If a later session shows installers sailing through the editor while struggling somewhere else, the priority order here should be rewritten. If the scheduler's tariff model changes, several of the points above about structure may stop applying. And if sunboard, the codename, ends up being replaced by a new name, update the naming section so searches keep working.
