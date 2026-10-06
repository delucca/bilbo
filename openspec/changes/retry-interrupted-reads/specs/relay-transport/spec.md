## ADDED Requirements

### Requirement: Signals during a request
A signal that the bilbo process receives and survives while the transport waits for a relay's answer SHALL NOT fail the request. The transport SHALL keep waiting within the request's time limit, and the request SHALL succeed or fail on what the relay does, as if no signal had arrived. The transport SHALL NOT report a relay unreachable because of a signal.

#### Scenario: Signals while the relay answers late
- **WHEN** the relay holds its answer for a fraction of a second and the bilbo process receives several signals that it handles, or is stopped and continued, before the answer arrives
- **THEN** the request returns the relay's answer, and sync or setup goes on without reporting `relay <url> unreachable`

#### Scenario: Signals while the relay closes the connection
- **WHEN** the relay accepts a request, receives it, and closes the connection without answering while the bilbo process receives signals that it handles
- **THEN** the transport reports `relay <url> unreachable: <reason>`, where the reason describes the closed connection, not the signal

#### Scenario: Signals while the relay never answers
- **WHEN** the relay accepts a request and never answers while the bilbo process keeps receiving signals that it handles
- **THEN** the transport reports `relay <url> unreachable: timed out` once the 300-second request limit has passed, and not later
