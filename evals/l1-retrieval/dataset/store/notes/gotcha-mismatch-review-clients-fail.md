---
id: 01JRRAR6S06F91F1T3MB3SQEA2
created: 2025-04-13T16:41-03:00
---

# mismatch-review-api responses over the gRPC message size limit

Clients of mismatch-review-api fail with `ResourceExhausted: grpc: received message larger than max` when a single response goes over the default limit of `4194304 bytes`. The server builds the response fine. The failure shows up on the client side, which is why it looks like a client bug at first. It is not; it is the default receive cap in gRPC for Go.

## Symptom

A reviewer opens a large settlement batch in a tool that calls mismatch-review-api. The call returns an error instead of data. The status code is ResourceExhausted and the message contains `grpc: received message larger than max`. Small batches work. The same call works again after narrowing the filter. Nothing is logged as an error on the server for the failed call, or at most a normal completed-call line.

## Why it happens

gRPC clients in Go cap the size of a received message. The default is `4194304 bytes`. If one unary response, or one message in a stream, is bigger than that, the client aborts the call. A list of mismatches with full ledger entry details and processor fields for a busy marketplace day can pass that size without much effort.

## Who sees it

- Finance ops tooling that lists mismatches for a whole settlement file in one call.
- Batch export jobs that pull everything for a period.
- Anyone testing with a big fixture file against a local instance.

Services that only ask for one mismatch at a time do not hit it.

## How to confirm

Check the error text on the client first. If it matches the message above, it is this limit. Then compare the size of the response you expect against `4194304 bytes`. Retrying does not help, since the same request produces the same oversized response every time.

```
ResourceExhausted: grpc: received message larger than max
default limit: 4194304 bytes
```

## Fix options

1. Page the results. Ask for fewer mismatches per call and loop with a page token. This is the preferred fix because it also keeps server memory and latency down.
2. Use a server-streaming call where one exists, so each message is small.
3. Raise the client receive limit with the call option for max receive message size. This works but only moves the cliff. Each client has to be changed.
4. Trim the response: drop fields the caller does not need.

## What not to do

Do not only raise the limit on the server side. The send limit and the receive limit are separate, and the failing check is the client's receive limit. Raising the server alone changes nothing for the caller.

Do not raise limits to some very large value and call it done. A response that big will be slow, and it ties up memory in both processes.

## Page size guidance

Pick a page size by the biggest realistic mismatch record, not the average. Mismatch records with many attached ledger lines are the large ones. Leave a lot of room under the cap. Test with a worst-case fixture.

## Things to check when changing it

- Every client of mismatch-review-api needs the same treatment; a Go client, a gateway, and any other language client each have their own default.
- Proxies or gateways in front of the service may have their own message size caps. Check them if the error text differs.
- Kafka consumers that call the service to enrich events can hit this too, and their failures may show up as stuck consumers rather than visible errors.

## Testing

Add a test that requests a response larger than the default and checks that paging returns it in several calls. Keep the fixture generated in the test rather than checked in as a large file.

## Notes for later

If response sizes keep growing, make paging mandatory in the API definition instead of optional, so a caller cannot ask for an unbounded list by accident. Document the cap in the API docs next to the list calls.

## Open questions

- Whether any current client already sets a custom receive limit, and to what.
- Whether the gateway layer, if any, applies its own cap.
