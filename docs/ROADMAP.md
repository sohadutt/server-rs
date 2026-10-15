# Implementation roadmap

This document tracks work needed before exposing printing or deploying the
server for untrusted users. The TLS, auth, API, and middleware directories
under `src/` are empty scaffolding; they are intentionally not declared in
`lib.rs` and do not affect the running application.

## Recommended order

1. **Lock down behavior with tests.** Add unit tests for configuration and IPP
   attribute parsing. Add HTTP integration coverage for malformed and oversized
   requests, methods, 404 responses, path traversal, symlinks, and file-size
   limits. Use a mock IPP endpoint for queue, job, timeout, and error cases.
2. **Verify IPP interoperability.** Check queue enumeration, printer
   attributes, queue depth, job submission, job listing, cancellation, device
   discovery, authentication, and IPPS against the target IPP server. Queue
   enumeration and device discovery currently use CUPS-defined IPP extensions;
   choose an implementation that supports those operations or replace them
   with standard IPP behavior where possible.
3. **Define the print API.** Specify routes, request and response formats,
   document size limits, supported formats, print options, job ownership, and
   authorization rules before adding handlers under `src/api/routes/`.
4. **Add authentication and authorization.** Identify users securely, map each
   HTTP identity to permitted printers and jobs, and enforce access on submit,
   list, and cancel operations. Do not trust a client-supplied `PrintJob.owner`.
5. **Add TLS.** Put TLS configuration and certificate loading in `src/tls/`;
   support certificate/key paths, startup validation, secure protocol defaults,
   and certificate rotation. Keep plaintext HTTP an explicit development or
   deployment choice once TLS is available.
6. **Expand print-job modeling.** Add multi-document jobs and IPP job
   attributes, validate requested options against printer capabilities, and
   enforce configurable document size and format limits. Keep document paths
   constrained to an approved spool or document directory.
7. **Harden server operations.** Bound concurrent HTTP connections and print
   requests, use request deadlines, drain active work on shutdown, and add
   health/readiness reporting. Define log retention and avoid logging document
   contents or credentials.
8. **Package and deploy.** Document OS dependencies, service account and file
   permissions, configuration, certificates, IPP endpoint credentials,
   firewall requirements, backups, upgrades, and rollback.

## Current print-service limits

- Print operations are library calls; there are no HTTP printing routes.
- A job submits one local document. Per-job IPP options and multi-document
  jobs are not implemented.
- Queue discovery uses CUPS-defined IPP extensions, although transport and
  printer/job operations use the IPP protocol and the Rust `ipp` crate rather
  than `libcups`.
- Printer refresh and job listing use batches of up to eight concurrent IPP
  requests; this limit is currently a code constant, not a config setting.
- Document size validation is available on `PrintDocument`, but the service
  does not yet apply a configured limit during submission.
- `PrintJob.owner` is returned metadata. The IPP server's authenticated
  identity controls job ownership.
- The HTTP server currently serves plain HTTP and has no authentication layer.

## Scaffold directories

- `src/tls/`: HTTPS transport and certificate configuration
- `src/auth/`: identity and authorization
- `src/api/routes/`: future application and print endpoints
- `src/middleware/`: shared request concerns such as authentication,
  request IDs, and rate limits
