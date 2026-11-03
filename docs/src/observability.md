# Observability

## Health

`GET /healthz` answers `200 {"status":"ok"}` after a database round trip, or
`503 {"status":"db_unreachable"}`. The image's `HEALTHCHECK` runs
`photovault healthcheck`, a tiny built-in HTTP probe of that endpoint (no
shell or curl in the image). `GET /ping` is museum's liveness endpoint used
by the apps.

## Metrics

`GET /metrics` serves OpenMetrics text. It has no authentication: block it at
the reverse proxy and scrape it from inside the Docker network
(`http://photovault:8080/metrics`).

| Metric | Labels | Meaning |
| --- | --- | --- |
| `photovault_http_requests_total` | `method`, `route`, `status` | Requests served. `route` is the route **template** (`/files/download/{file_id}`), never the raw path. |
| `photovault_http_request_duration_seconds` | same | Latency histogram. |
| `photovault_storage_operations_total` | `backend`, `op`, `result` | Object-store calls (`presign_put`, `create_multipart`, `head`, `copy`, `delete`, …); `result` is `ok`, `not_found` or `error`. |
| `photovault_storage_operation_duration_seconds` | same | Latency histogram (for S3 this includes the network round trip). |
| `photovault_storage_transfer_bytes_total` | `direction` | Bytes uploaded/downloaded through local-backend signed URLs. |
| `photovault_db_pool_connections` | – | Open connections in the pool (sampled at scrape). |
| `photovault_db_pool_idle_connections` | – | Idle connections. |
| `photovault_db_pool_max_connections` | – | Pool size limit (16 Postgres, 4 SQLite). |
| `photovault_db_ping_duration_seconds` | – | Database round trip measured at each scrape. |

## Logs

Logs are JSON lines on stdout (`log_format = "pretty"` for humans), filtered
with `RUST_LOG` (default `info`). Each request produces one `access` event
with method, route template, status, latency and `req_id` (also returned as
the `X-Request-Id` response header).

What is **never** logged: auth tokens, email addresses, one-time codes once
SMTP is configured, raw request paths or query strings (signed URLs carry
their signature in the query), S3 credentials, and object contents.
Storage-backend errors are reduced to the operation and S3 error code before
they reach the log.
