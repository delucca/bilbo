---
id: 01M35VTCEP5R4KRFJR7S4EABRF
created: 2026-09-22T21:49-03:00
---

# iothub-gateway X.509 switch plan

This note replaces the earlier note about "iothub gateway switch device". The new value is that the switch of iothub-gateway to X.509 authentication now ships in the `2027 Q2` release. That replaces the earlier schedule, which should be treated as dead. If you find the old date in a ticket, a roadmap page, an installer email or a comment in the code, it is stale and should be corrected to `2027 Q2`.

The rest of this note is the working plan for getting there: what changes, in what order, what can go wrong, and what we still do not know. It is written quickly and will need edits as the work firms up.

## What is being switched

iothub-gateway sits between the site devices and the cloud side of GridHaven. Inverters, meters and battery controllers publish readings over MQTT. The gateway takes those messages, talks to Azure IoT Hub on behalf of the device or the site, and passes telemetry on so that it ends up in InfluxDB. The Julia forecasting and scheduling code reads from there, and the Svelte dashboards that installers and customers use sit on top of that.

Today the gateway authenticates to Azure IoT Hub with the existing shared-secret style of credential. The change is to move to X.509 certificates, so each identity the gateway presents to the hub is proven by a certificate and private key pair rather than a symmetric key or token derived from one. In plain terms: the thing that proves who we are changes, and everything that stores, rotates, hands out or checks that proof changes with it.

What is not changing: the MQTT topics the site devices publish to, the shape of the telemetry, the InfluxDB schema, and the forecast and tariff scheduling logic. If a change in this work seems to need one of those, stop and write it down as a separate item rather than folding it in.

## Why it moved to 2027 Q2

The release slot moved, and the earlier schedule no longer holds. The practical reasons, as far as they are known to the people working on it:

- The certificate lifecycle side is bigger than first thought. Issuing, renewing and revoking certificates for a fleet of home installations is an operational system, not a config flag.
- Installers need time and clear instructions. Their field process for commissioning a site has to change, and they cannot absorb that change in the middle of a busy install season.
- We want a long overlap period where both credential types work, so that a site can be moved without a truck roll if possible, and so that a bad rollout can be backed out.

None of this is a reason to stop preparing. Most of the preparatory work can land well before the release and should, so that the release itself is mostly a flip of a default plus documentation.

## Plan of work

The order below is the intended order. Each step should be mergeable on its own and should leave the current shared-secret path working.

### Step one: make credentials pluggable inside the gateway

Right now the code that builds the connection to Azure IoT Hub assumes one kind of credential. Pull that behind a small interface so the gateway can be handed either kind, chosen by configuration. No behavior change for existing sites. The test for this step is that the existing integration tests pass unchanged and a new test shows the credential kind being picked from configuration.

Keep the choice per site or per device identity, not global for the whole process, because during the overlap period one gateway process may serve sites in different states.

### Step two: certificate handling in the gateway

Add loading of a certificate and private key, with a clear failure when either is missing, unreadable, expired or does not match the other. Error messages need to say which file or which identity, because the person reading them will be an installer on a phone, not a developer.

Decisions to make here, and not yet made:

- Where the private key lives on the device and what file permissions it needs.
- Whether the key is generated on the device and never leaves it, which is the preferred answer, or is provisioned from outside.
- How the gateway notices a renewed certificate on disk and picks it up without dropping MQTT traffic from the site devices.

The hot reload point matters. A gateway that has to restart to pick up a new certificate will drop readings, and a gap in readings degrades the solar forecast and can push the battery schedule onto stale data during a tariff window. Prefer reloading in place, and test what happens to buffered MQTT messages while the reload happens.

### Step three: enrollment and identity in Azure IoT Hub

Decide between registering each identity individually in the hub and using an enrollment approach where identities are created when a device first presents a certificate chained to a trusted authority. The second scales better for an installer base, but it adds a certificate authority we have to run or buy, and it adds trust decisions we have to document. This is the single biggest open design question and should be settled before anything else in this step is built.

Whatever is chosen, the mapping between a hub identity and a GridHaven site must stay the same as it is now, so that existing InfluxDB series keep their tags and history stays continuous. Check this explicitly with a before and after comparison on a test site.

### Step four: the overlap period

Both credential types are accepted for a long stretch. During it:

- The gateway reports, in its own logs and in a status field the dashboard can read, which credential kind each identity is using.
- We can list the sites still on the old kind, so support can chase them.
- Moving a single site over and moving it back are both supported and both tested.

Do not remove the old path in the same release that makes X.509 the default. Removal is a later release and gets its own note.

### Step five: installer and customer facing material

Write the commissioning instructions for installers, the troubleshooting page for the common failures, and a short customer-facing statement that says nothing changes in how the dashboard looks or behaves. Get a couple of friendly installers to follow the instructions cold before anything is published, and note where they stumble.

## Testing approach

Use a test hub, not the production one, for everything up to the final rehearsal. The cases that matter:

- Fresh site commissioned with a certificate from the start.
- Existing site moved from the old credential to a certificate while devices keep publishing over MQTT.
- Certificate expires while running: the gateway should log a clear message, keep buffering what it can, and recover when a new certificate appears.
- Certificate revoked or identity disabled in the hub: the gateway should stop cleanly and report, not retry in a tight loop.
- Clock skew on a site with a bad time source. Certificate validation depends on time, and home hardware sometimes boots with a wrong clock. This is a likely source of field support calls, so treat it as a first-class case.
- Network loss and return, with the connection re-established using the certificate and no manual action.

For each, check the end result in InfluxDB: no duplicated points, no silent gaps beyond what the buffering design allows, tags unchanged.

## Risks

The main risks, roughly in order of how much they worry me:

- Clock problems on site hardware causing valid certificates to be rejected. Mitigation is a clear error and a fallback path during the overlap period.
- Certificate renewal failing silently and the site going dark weeks later. Mitigation is renewal well ahead of expiry, plus an alert when a certificate is getting close to expiry and has not been renewed.
- Private key handling mistakes: keys in logs, in support bundles, or in backups. Mitigation is review of every place the gateway writes diagnostics, and a rule that key material is never included in any output.
- Installer confusion during commissioning, producing a pile of half-enrolled sites. Mitigation is the dry-run with real installers and a status view that shows the enrollment state plainly.
- Schedule drift. The date has already moved once. The preparatory steps should be kept small and merged early so that a further slip, if it happens, costs a documentation change and not a rewrite.

## Open questions

- Who runs the certificate authority, and what is its own failure plan?
- How long should certificates live? Short lifetimes are safer but make renewal failures bite sooner.
- Does the Svelte dashboard need any change beyond showing credential state for support users? Current answer is probably not for customers.
- Do any older gateway builds in the field lack what is needed to hold a key and certificate safely, and if so what is the upgrade path for them before the release?
- What is the cutoff, after the release, when the old credential path stops being accepted? Not decided and deliberately not in this note.

## Housekeeping

Anything that still refers to the previous schedule should be updated to say `2027 Q2`. When a part of this plan is finished or changes, edit this note in place rather than adding a second note on the same subject. If the release date moves again, change it here first, then fix the references elsewhere.
