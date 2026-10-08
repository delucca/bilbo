---
id: 01KKRRSGKZJXJFSQ6RTFM0A0VH
created: 2026-03-15T09:50-03:00
---

# telemetry-ingest: partial write field type conflict when inverter firmware sends watts as an integer

telemetry-ingest reports `partial write: field type conflict` when inverter firmware sends watts as an integer instead of a float. That is the whole trap. The device payload looks fine, the message passes through MQTT and Azure IoT Hub without complaint, and telemetry-ingest accepts it. Then the write into InfluxDB comes back as a partial write, and some points are missing from the series that the Julia forecasting code reads later. Nothing crashes. The symptom is quiet holes in the solar output history, followed by forecasts that look oddly flat or stale for the affected installations.

This note is for whoever sees that error in the telemetry-ingest logs, or sees gaps in the production series and has no idea why. Read the first section, then the checks. Most of the rest is background on why this keeps coming back and what we do and do not want to do about it.

## What actually happens

InfluxDB fixes the type of a field the first time it sees it in a given measurement and shard. If the first point for the watts field arrived as a float, every later point for that field in that measurement has to be a float too. A point that carries the same field as an integer is rejected for that field. The server does not reject the whole batch. It writes the points that agree with the existing schema and refuses the ones that do not, and the response says so. telemetry-ingest logs that response as `partial write: field type conflict`.

The reverse also happens. If the first point ever written for a field was an integer, then later floats are the ones that conflict. So the error does not tell you which side is the odd one out. You have to look at what the schema holds now and at what the firmware sends. In practice the schema holds floats, because the first installations we onboarded ran firmware that sent decimals, and the newer or older firmware variants that send whole numbers are the exceptions.

The reason firmware sends an integer is mundane. Some inverter firmware builds format the power reading with no decimal part when the value happens to be a whole number, or always, depending on the build. A reading of a round value is serialized as a bare whole number in the JSON, and the JSON parser on our side turns that into an integer, not a float. The same inverter will send a decimal one minute later when the reading is not round. So one device can alternate between integer and float within a single hour. That is why the failure looks intermittent and why it is easy to mistake for a network problem.

Because the failure is partial, the line protocol that telemetry-ingest builds is still mostly accepted. The batch as a whole does not return a hard error status that would trip a retry. If your retry logic only reacts to hard failures, it will never re-send the rejected points, and re-sending them as-is would produce the same conflict anyway.

## How to recognise it

The fastest signal is the log line itself. Search the telemetry-ingest output for the exact text `partial write: field type conflict`. If it is present, you are in this problem, or at least in a close relative of it. The close relatives are the same message caused by a different field, for example a state of charge field or a temperature field that some firmware sends as a whole number. The fix pattern is the same but the field name differs, so read the rest of the log line to see which field the server complained about.

The second signal is on the data side. Query the production series for a single affected installation and look for points that are missing at regular but irregular-looking intervals. Missing points tend to be the round values. If you see that the low or exactly-zero readings at night are present while some daytime peaks are absent, or the other way round, think of type conflicts before you think of connectivity. Zero is a classic: firmware that sends a bare zero overnight hits the integer path every single time. If the schema expects floats, every night reading is rejected, and the series shows daytime data only. People then assume the inverter is asleep and nobody looks closer.

The third signal is the forecast side. The Julia code that fits the solar output model reads the history and resamples it. Gaps are filled by interpolation or dropped, depending on the settings. With many rejected points the fit quality degrades without any error, and the battery schedule built on that forecast gets worse. If an installer reports that the charging plan for one customer seems off compared with neighbours, check ingest logs for that device before touching the model.

The fourth signal, which is the most reliable for confirmation, is to compare what the device published with what is stored. Subscribe to the device topic on MQTT, or look at the message as it arrived via Azure IoT Hub, and read the raw watts value. If it is printed without a decimal point, it is an integer. If the stored series is missing that timestamp, you have the cause.

## Why it is easy to miss

Several things hide this problem.

First, the write is partial, not failed. Monitoring that counts failed writes will show nothing. The count of successful requests stays healthy. Only a log search or a comparison of points sent against points stored shows the loss.

Second, it depends on the firmware version and sometimes on the value. A device may be fine for weeks and then hit the problem after a firmware update that changes number formatting. Installers update firmware on their own schedule and do not tell us. So a working installation can begin to lose points with no change on our side.

Third, the first write wins. If a brand new measurement or a new shard is created from an integer first, the schema is integer from then on, and the properly formatted floats from all the healthy devices start conflicting. This is the nastiest variant: one odd device that happens to be first can poison the field for everyone writing to that measurement in that time window. If many devices suddenly show the error at once after a quiet period, suspect this variant and look at who wrote first.

Fourth, tests with hand-written sample messages almost always use decimals, because the person writing the sample copies a realistic number like a value with a fractional part. The integer case simply is not covered unless somebody thinks of it.

Fifth, JSON has no integer-versus-float distinction in the spec. A value written as a whole number and the same value written with a trailing decimal zero are the same number to many tools. Only the receiving parser decides. So the data looks identical in a viewer and differs only in the type the code infers.

## What to do when you hit it

Start by establishing which field and which direction. Read the full log line and, if needed, ask the database what type it holds for that field in the affected measurement. Then decide whether the schema or the input is the odd one out. In nearly every case the schema is correct and the input should be coerced.

The right fix lives in telemetry-ingest, before the line protocol is built. When the code decodes the device message, it should coerce every field that the schema defines as a float into a float, regardless of how the JSON delivered it. For watts that means that a whole number is converted to a float before it is formatted into the write. In line protocol terms, a float is written without the integer suffix, and an integer is written with it, so the formatter has to know the intended type per field and not guess from the parsed value. Keep a small, explicit table of the expected type for each telemetry field and apply it on the way in. Do not rely on the parser's guess.

Do not change the database schema to integer to fit the firmware. Floats are needed for fractional readings, and the forecasting code assumes float values. Switching the type would also require a new measurement or a migration, since the stored type cannot be changed in place.

Do not drop the conflicting points as noise. They are real readings, and for round values like an exactly-zero night reading they matter for the daily totals.

If you must unblock quickly before a proper fix, the least harmful step is a coercion at the point where the message is decoded, even if it is crude, plus a log line when a coercion actually happened so you can see which devices and firmware need attention. Then do the proper table-driven version afterwards.

For points that were already rejected and lost, there is usually nothing to recover from the ingest side, because we do not keep a copy of the rejected batch. If the device or Azure IoT Hub still holds the raw messages inside its retention window, you can replay from there after the coercion is in place. Replay carefully: writing the same timestamp twice with the same field simply overwrites the point, which is fine, but make sure the replay goes through the fixed path and not through the old one, otherwise you get the same conflict again. Tell the installer that history before the fix has gaps, so they do not read the dips as a hardware fault.

## Preventing it from returning

A few habits would have caught this earlier.

Write tests for telemetry-ingest that feed it messages where numeric fields are whole numbers, including zero, and assert that the produced line protocol marks them as floats. Add the reverse case too, a field that is meant to be an integer arriving as a decimal, and decide deliberately what should happen there: coerce, or reject with a clear message that names the field.

Add a counter for partial writes in telemetry-ingest and surface it in the dashboards next to the success count. A rising partial-write counter with a flat failure counter is exactly the signature of this bug, and it should be visible without grepping logs. Alert on a sustained non-zero rate, not on a single occurrence, since an occasional odd device is expected.

When onboarding a new inverter model or accepting a firmware change from an installer, send a few sample messages through a staging instance with a separate measurement and look at the stored types before pointing real traffic at the production measurement. Staging must use its own measurement, not the production one, because the first write defines the type and a test with the wrong type can poison the real thing.

Keep the per-field type table in one place and have the Julia side and the Svelte dashboard read the same definitions where practical, so that nobody assumes a different type downstream. If the dashboard shows the value with formatting that depends on the type, a coerced float will render the same as before, which is the point.

When a firmware vendor documents number formatting, record it near the table. When they do not, assume nothing and coerce.

## Related behaviour worth remembering

The same rule applies to every field, not only watts. Any telemetry field whose first stored type differs from what a device later sends will produce the same message. Fields that commonly hit this are those that are whole numbers much of the time: charge level, grid import and export, and counters. Treat each as float or integer on purpose.

Tags are strings in InfluxDB, so a device identifier sent as a number does not conflict, but it does create a separate series from the same identifier sent as text if formatting differs. That is a different problem with a similar look: split series instead of missing points. Worth a glance when you are already in the data.

Timestamps are not part of the type conflict, but if a device sends them in a unit different from what telemetry-ingest assumes, points land far in the past or future and look missing too. Rule out time problems before blaming types if the log has no mention of `partial write: field type conflict`.

If the error text appears and the affected field is not one you recognise, someone may have added a new field to a device payload. Check whether the new field was ever written with another type earlier, for example during an experiment, since the old type will stick for that measurement in that time range.

Finally, remember that the message arrives with the server's wording and the exact phrase may be extended with further detail after it. Search for the stable part, `partial write: field type conflict`, and read what follows it for the field name and the types involved.

## Short checklist

See the error in the telemetry-ingest log. Identify the field and the stored type. Look at the raw device message and confirm the value arrives as a whole number. Coerce to float in telemetry-ingest before building the write, driven by a per-field type table. Replay recoverable raw messages through the fixed path if the history matters. Add the partial-write counter and the whole-number tests so the next firmware change does not repeat this quietly.
