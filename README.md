# server-rs

Rust port of the active web and printing services in [`server/`](server/).
The Rust app has a Tokio TCP server, bounded HTTP request parsing, safe static
file resolution, configurable MIME mappings, `log`/`env_logger` logging, and
printer operations over IPP using the `ipp` crate. It does not link to
`libcups` or use the CUPS client library.

## Run

```sh
./scripts/run.sh
```

Runtime configuration and MIME mappings live in `local/`. The run script
uses `local/server.conf` by default; pass a different config path when needed.

```sh
./scripts/run.sh path/to/server.conf
```

To check formatting and compile all targets, run:

```sh
./scripts/check.sh
```

## Configuration

The config format matches the C version: one `key = value` pair per line,
with blank lines and `#` comments allowed. Invalid or unknown options stop
startup with the file and line number.

| Setting | Default | Valid values |
| --- | --- | --- |
| `host` | `0.0.0.0` | Host name or IP address |
| `port` | `8080` | `1`–`65535` |
| `worker_threads` | `4` | `1`–`1024` Tokio runtime threads |
| `document_root` | `public` | Existing directory |
| `index_file` | `index.html` | Safe relative file path |
| `not_found_page` | `404.html` | Safe relative file path |
| `mime_types_file` | `local/mime.types` | Extension and media type pairs |
| `default_mime_type` | `application/octet-stream` | Valid MIME type |
| `max_static_file_size_mb` | `64` | `1`–`1024` |
| `log_level` | `info` | `debug`, `info`, `warn`, `error` |
| `log_console` | `true` | `true`, `false`, `1`, `0` |
| `log_file` | `true` | `true`, `false`, `1`, `0` |
| `log_directory` | `local/logs` | Non-empty path |
| `cups_server` | `localhost:631` | Optional IPP server host and port |
| `cups_encryption_required` | `false` | Use `ipps` (TLS) instead of `ipp` |

The `cups_*` option names are retained for compatibility with the original C
configuration. They configure the IPP endpoint and transport.

The MIME file accepts one `extension media/type` mapping per line. Extension
matching is case insensitive; unmapped extensions use `default_mime_type`.

## Layout

- `src/config.rs`: validated server and IPP endpoint settings
- `src/server.rs`: Tokio listener, graceful shutdown, and connection handling
- `src/http/`: bounded request parser and HTTP response writer
- `src/static_files.rs`: document-root checks, path decoding, and MIME lookup
- `src/printing/`: IPP service and public printer/job types
- `src/logging.rs`: `log` facade with env_logger output to console and file
- `local/`: runtime server config and MIME mappings
- `scripts/`: run and compile-check helper scripts
- `docs/`: implementation roadmap and deployment notes
- `src/tls/`, `src/auth/`, `src/api/`, `src/middleware/`: empty placeholders
  for planned features; they are not compiled or active yet
- `public/`: sample site

The C project’s active listener is serial and serves GET requests only. The
Rust port uses the configured worker count for concurrent connections, while
preserving the GET-only behavior, the 4,095-byte request limit, configured
404 page, static size limit, and document-root traversal protections.

## Printing

`server_rs::printing::PrintService` is an IPP client implemented with the
`ipp` crate; it does not use `libcups`. It reads queues from the configured IPP
server, refreshes printer state and job counts, chooses an eligible queue, and
submits, lists, or cancels jobs. Queue enumeration and device discovery use
CUPS-defined IPP extensions, so those operations require a compatible server.
Printing remains a library service; the C project does not expose it as an HTTP
route.

The authenticated IPP server identity owns submitted jobs. `PrintJob.owner` is
metadata returned with jobs; it is not sent as an IPP impersonation attribute.

The C project’s `router`, TLS, event loop, buffer, and worker pool source files
are empty scaffolding and are not linked into its executable. They therefore
do not have runtime behavior to port. The C print-job source is also partial;
the Rust library provides complete IPP operations for the corresponding API.

## Next steps

The service currently provides IPP operations as a Rust library; the HTTP
server only serves static GET requests. The print service refreshes printer
snapshots concurrently with a limit of eight requests, and reads queue depth
from printer attributes during selection to avoid a second round trip. The
implementation plan, including security and deployment requirements, is in
[`docs/ROADMAP.md`](docs/ROADMAP.md).
