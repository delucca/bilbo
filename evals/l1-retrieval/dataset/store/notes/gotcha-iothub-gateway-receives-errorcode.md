---
id: 01KVRVPJ5M9GYR860CWHFR0N38
created: 2026-06-22T20:47-03:00
---

# iothub-gateway: SAS token expiry causes unauthorized errors

iothub-gateway receives ErrorCode:IotHubUnauthorizedAccess once the SAS token expires, which happens after 3600 seconds. That is the whole trap: the gateway connects fine, publishes telemetry for an hour, then every send and every cloud-to-device receive is refused with that error until the token is renewed or the connection is rebuilt. Nothing else about the setup changes at that moment, so the failure looks random if you do not know the token lifetime.

## Symptom

The gateway runs normally after start. Forecast inputs and battery telemetry flow from the MQTT side through iothub-gateway into Azure IoT Hub, and on to InfluxDB. Then, 3600 seconds after the token was issued, the logs fill with ErrorCode:IotHubUnauthorizedAccess. Restarting the process makes it work again, for another 3600 seconds. That restart-fixes-it pattern is the giveaway.

What a reader should remember: the error is not a network fault and not a bad key. The credentials were valid when the token was minted. The token simply ran out. The lifetime is 3600 seconds, counted from when the token was generated, not from the last message sent.

## Why it happens

iothub-gateway authenticates to Azure IoT Hub with a shared access signature (SAS) token derived from the device key. The token carries an expiry. After 3600 seconds IoT Hub rejects anything signed with it. If the gateway generates the token once at startup and reuses it, it will always hit this wall at the one-hour mark.

An open connection does not save you. IoT Hub checks token validity and drops or refuses the session when the token lapses. Reconnecting with the same old token fails with the same error, so a naive retry loop just spins on ErrorCode:IotHubUnauthorizedAccess.

## What it does to GridHaven

The downstream effect is quiet. The Julia forecasting side keeps running on whatever data it already has, and the battery scheduler keeps using the last known state against the time-of-use tariffs. So installers and customers do not see an outage; they see stale numbers. Gaps show up in InfluxDB at the one-hour boundary, which is the easiest place to confirm the cause after the fact. The Svelte dashboard just shows flat or missing recent data.

Charging schedules built from stale state can be wrong during a tariff window change, which is when it costs the customer money. Treat a gateway stuck on this error as urgent even though nothing crashes.

## How to confirm

- Look at the time between gateway start (or last successful reconnect) and the first ErrorCode:IotHubUnauthorizedAccess line. If it is about 3600 seconds, this is the cause.
- Check that other devices with fresh tokens still connect to the same hub. If they do, the hub and the network are fine.
- Check for gaps in InfluxDB that start at the same offset after each gateway restart.
- Compare against a restart: if a restart clears it, the key is good and only the token expired.

## Fix direction

Do not extend the lifetime as the main fix. Regenerate the token before it expires, well ahead of the 3600 seconds mark, and reconnect or update the credential on the live session using the new token. Schedule renewal from the token's issue time, with a safety margin, and renew on a timer rather than waiting for a failure.

As a second layer, treat ErrorCode:IotHubUnauthorizedAccess as a signal to mint a new token and then retry, instead of retrying with the old one. Retrying with the same token is pointless. Cap the retry rate so a truly revoked key does not hammer the hub.

Alternatively, if the SDK in use supports token refresh natively, rely on that and make sure it is enabled, not just that the code compiles against it.

## Things to watch for

- Clock skew on the gateway host changes when the hub thinks the token ends. A fast or slow clock makes the failure arrive earlier or later than 3600 seconds, which can mislead the diagnosis.
- Tests that run for less than an hour never reproduce this. A test needs a short-lived token or a faked clock to show it.
- A monitoring alert on this specific error string is cheap and catches the problem before customers notice stale data.
- Do not confuse this with a revoked or wrong device key. That fails at the first connect, not after an hour.

## Open items

Check whether the current renewal path covers both the telemetry send and the receive side, and whether the MQTT bridge reconnects cleanly after a renewal without losing buffered messages. Add the alert on the error string and a test with a short token lifetime so this does not regress.
