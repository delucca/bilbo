---
id: 01JYF1EBZE696AHG85TZXWDMFB
created: 2025-06-23T15:40-03:00
---

# iothub-gateway: move to per-device X.509 authentication

The plan is to switch iothub-gateway to per-device X.509 certificate authentication in the 2027 Q1 release. Until then it keeps its current shared-credential approach. This note holds the reasoning, the order of work, the risks we already see, and what is still open. It is a plan, not a finished design, so expect details to move once the first device group is migrated.

The short version: every device that talks through iothub-gateway to Azure IoT Hub gets its own X.509 certificate and presents it when it connects. The gateway stops relying on a credential that many devices share. If one inverter, meter or battery controller is lost, stolen or compromised, we revoke that one device and nothing else is affected.

## Why we are doing this

Today the gateway authenticates with credentials that are effectively shared across a group of devices. That was fine for the early installs, where installers were few and each site was handled by hand. It does not hold up as the installer base grows. The problems, in order of how much they bother us:

- A leaked credential cannot be contained. Because many devices share it, rotating it means touching every device that uses it, and in practice that means waiting for each household's gateway to come online and accept the change.
- We cannot tell devices apart in the audit trail. When something odd shows up in Azure IoT Hub, the identity that connected does not point at one physical site. Support has to cross-check timestamps against telemetry in InfluxDB to guess which household it was.
- Installers hand customers hardware that holds a secret. A per-device certificate with a private key that never leaves the device is a better story to tell installers than a connection string that sits in a config file.
- Revocation is all or nothing. We want to be able to cut off one household without affecting forecasts or battery schedules for anyone else.

The X.509 route fits how Azure IoT Hub already models identity, so we are not inventing anything on the cloud side. The work is mostly on our side: issuing, delivering, storing, renewing and revoking certificates, and making the gateway behave well when a certificate is missing, expired or rejected.

Why the 2027 Q1 release and not sooner: the forecasting and charge-scheduling work in Julia has its own commitments this year, installers need notice before we change onboarding, and we do not want to ship a change to device authentication in the middle of the winter tariff changes when battery scheduling matters most to customers. 2027 Q1 gives us time to pilot first and still land before the next heating season peak.

## Scope

In scope:

- The authentication path between iothub-gateway and Azure IoT Hub for device-to-cloud telemetry and cloud-to-device commands.
- Certificate issuing for new devices at install time, and a migration path for devices already in the field.
- Renewal and revocation, including what the gateway does while a renewal is pending.
- Installer-facing changes in the Svelte dashboard: showing certificate state per device and giving installers a way to re-enroll a device.
- Monitoring and alerting for certificate problems.

Out of scope for this plan:

- Changes to how MQTT topics are laid out between local devices and the gateway. The local side of MQTT stays as it is. If we later want mutual authentication on the local broker as well, that is a separate plan.
- Changes to how telemetry is written into InfluxDB, apart from carrying a device identity field that already exists.
- Any change to the forecasting models or the tariff scheduling logic.

The reason for keeping the local MQTT side out is simple: it is a different trust boundary, with different hardware constraints, and mixing the two would make the rollout much harder to reason about when something fails.

## Approach

The approach is to treat the certificate as the device's identity and make everything else derive from it.

### Certificate hierarchy

We use a private certificate authority that we control, with an intermediate used only for device certificates. The root stays offline. Azure IoT Hub is told to trust that chain, and each device registers with its certificate's identity as its device identity. The intermediate can be rotated without touching the root. We want the device identity string to be derived in a stable way from the hardware serial or installer-assigned site reference, so support can read an identity and know which household it is without a lookup.

An alternative we considered was to use the Azure device provisioning service for enrollment and let it manage the chain. We have not ruled it out. It reduces how much we operate ourselves, but it ties onboarding to another cloud dependency and complicates offline installs, where an installer commissions a device before the household has working internet. This is listed under open questions below.

### Key handling on the device

The private key should be generated on the device where the hardware allows it, and a signing request sent out, so the key is never transmitted. For older hardware without a secure element or a good random source, we fall back to generating the key during a supervised installer session and storing it with restrictive permissions. We will be honest in the docs about which hardware gets which level of protection. Do not store keys in the same place as the application configuration, and do not log certificate contents or key material anywhere, including at debug level.

### Gateway behaviour

The gateway loads the device certificate and key at startup and reloads them when they change on disk, so a renewal does not need a restart. When the connection to Azure IoT Hub is refused because of a certificate problem, the gateway should:

- keep buffering telemetry locally within its existing limits, rather than dropping readings;
- keep serving local MQTT clients so on-site behaviour, including battery charging against the tariff schedule, keeps running from the last known plan;
- back off its reconnect attempts with jitter, so a fleet-wide problem does not turn into a reconnect storm against the hub;
- surface a clear, distinct state for "certificate rejected" versus "certificate expired" versus "network down", because support needs to tell these apart quickly.

The last point matters more than it looks. In the old setup a bad credential and a bad network looked about the same from the outside. With certificates there are several new failure modes, and lumping them together will cost us support time.

### Renewal and revocation

Certificates get a lifetime short enough that a lost device ages out on its own, but long enough that a household offline for a few weeks does not fall off the cloud. Renewal starts well before expiry, using the current valid certificate to authenticate the renewal request. If a certificate has already expired, the device cannot renew by itself, and the installer or the customer has to trigger a re-enrollment. We need that path to be simple enough for a customer to follow with the installer on the phone.

Revocation means disabling or removing that device identity in Azure IoT Hub and adding the certificate to our own deny list. Both steps must happen together; doing only one leaves a gap. We will write this as a single operation in our admin tooling so nobody has to remember the pair.

## Rollout order

The order of work, roughly:

- First, build the certificate authority and issuing flow in a test environment, and enroll a handful of internal test devices that mimic the real hardware mix.
- Second, change iothub-gateway so it supports both the existing authentication and X.509, selected per device by configuration. This dual mode is the key safety measure. It lets us migrate device by device and roll back a single device without a release.
- Third, add the Svelte dashboard changes so installers can see certificate state and trigger re-enrollment. These need to be in place before any real customer device is migrated, not after.
- Fourth, pilot with a small number of friendly installers and their households. Watch for connection failures, renewal behaviour, and time-to-recover after a deliberate expiry test.
- Fifth, ship the 2027 Q1 release with X.509 as the default for all new installs, and begin migrating existing devices in batches by installer.
- Sixth, after the field has migrated and we have gone a full renewal cycle without incident, remove the old shared-credential path and revoke the old credentials.

The sixth step is deliberately not tied to the release. Removing the old path early is the single riskiest thing in this plan, because it turns any missed device into an offline device. We would rather carry the old code for a while than force it.

## Risks and things to watch

- Clock drift. Certificate validation depends on the device clock. Some household devices have poor or unset clocks right after power loss. A device with a wrong clock will see good certificates as not yet valid or already expired. The gateway needs a sanity check on time before it decides the certificate is at fault, and the error state should say when time looks wrong.
- Devices that are offline across a renewal window. These will need the manual re-enrollment path. We should measure how many households this affects during the pilot rather than guess.
- Installer workflow friction. If enrolling a device takes noticeably longer than it does today, installers will complain, and they are our main channel. The commissioning flow has to stay short.
- Operational load of running a certificate authority. Someone has to own it: backups of the intermediate, an expiry calendar for the intermediate itself, and a tested procedure for replacing it. Letting the intermediate expire unnoticed would take down every new enrollment at once.
- Hub-side limits. Azure IoT Hub has quotas on registered identities and on operations. Check that our batch migration does not trip throttling, and pace it.
- Data continuity. Telemetry in InfluxDB is tagged by device identity. If the identity string changes during migration, dashboards and the forecasting inputs could split a household's history in two. Keep the identity string stable across the switch, or add a mapping before anything is migrated.
- Partial outages mid-migration. Because both modes will exist at once, monitoring must report them separately, otherwise a healthy-looking aggregate can hide a failing group.

## Open questions

- Do we use the Azure device provisioning service or run our own enrollment? Leaning toward our own for the first release because of the offline-install case, but this should be revisited after the pilot.
- What lifetime do we choose for device certificates? Needs input from support about how long households are typically offline, and from whoever owns the authority about operational cost.
- Which older hardware cannot generate keys on the device, and what is our position on supporting it? One option is to stop supporting X.509 migration for the oldest units and keep them on the old path until they are replaced, but that conflicts with removing the old path.
- How does an installer prove they are allowed to re-enroll a given household's device? Probably tied to the existing installer account and site assignment, but not designed yet.
- Do we need customer-visible messaging about the change? Probably only if a device falls offline, but the wording should be agreed before the pilot.

## Next steps

Write a short design note for the certificate hierarchy and identity string, since the data continuity risk depends on getting that right first. Then start on dual-mode support in iothub-gateway, because everything else depends on being able to switch per device. Revisit this plan after the pilot and update the dates and the open questions; the commitment that stays fixed is X.509 for iothub-gateway in the 2027 Q1 release.
