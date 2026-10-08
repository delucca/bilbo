---
id: 01KP86EW5MAS0Y1F64XSD6RV96
created: 2026-04-15T06:09-03:00
---

# iothub-gateway transport: AMQP over WebSockets on 443

iothub-gateway talks to Azure IoT Hub using AMQP over WebSockets, on port 443. We chose this because customer routers commonly block port 5671, which is the port plain AMQP over TLS needs. Installers kept hitting sites where the gateway could not connect at all, and 443 is almost never filtered because ordinary HTTPS uses it.

## Decision

Use AMQP over WebSockets on port 443 as the transport for iothub-gateway to Azure IoT Hub. Do not make plain AMQP on 5671 the default. The WebSocket variant is the default everywhere, not a fallback that only kicks in after a failure.

## Why

The problem was in the field, not in our lab. At home installs the router is the customer's, often an ISP-supplied box with a locked-down firewall. Outbound traffic on 5671 is dropped on a lot of them, and the installer has no way to change that, or the customer does not want them to. A gateway that works in the office and fails on the roof-install day is the worst outcome for the installer.

Port 443 gets through nearly everywhere. The WebSocket framing adds a bit of overhead per message, but our payloads are small telemetry readings and battery commands, so the cost does not matter in practice.

## Alternatives considered

- Plain AMQP on 5671: lowest overhead, but blocked too often. Rejected as default.
- Try 5671 first, then fall back to WebSockets: sounds safe, but a blocked port often hangs until timeout rather than failing fast. Every start on a bad network would be slow, and the failure path would be hard to test. Rejected.
- MQTT to the hub directly: we already use MQTT locally on the site. Using it toward the cloud as well would mix two concerns, and we did not want to change the cloud path just for this. Not pursued.

## Consequences

- Firewall guidance for installers is simple: outbound HTTPS must work. No special port requests to customers.
- Support should not ask customers to open 5671. If a site cannot connect, look first at proxies or TLS interception on the router, not at port rules.
- Anything that watches connection state (forecast upload, tariff schedule sync, battery charge commands) sees one transport, so there is only one connection path to debug.
- Deployment config should carry the transport and port as explicit settings, so a later change does not mean a code change.

## Config sketch

Illustrative only, the real key names in the gateway config may differ:

```toml
[iothub]
transport = "amqp-websockets"
port = 443
```

## Open points

- If a customer network does inspect or break TLS on 443, we have no plan B yet. Revisit if it shows up in support tickets.
- Check whether keepalive settings need tuning for WebSocket connections through cheap routers that drop idle connections.
