# HTTPS behind a reverse proxy

Photovault speaks plain HTTP on port 8080 and expects a reverse proxy to
terminate TLS. The apps refuse plain HTTP for anything but local testing.

## Hostnames

| Hostname | Points to | Used for |
| --- | --- | --- |
| `photos.example.com` (`public_url`) | `photovault:8080` | API: what you enter in the apps. |
| `storage.photos.example.com` (`storage.public_url`, optional) | `photovault:8080` | Signed upload/download URLs (`/storage/…`) when using the local backend. |

Both may be the same name. A separate storage hostname is useful when the API
name goes through something with upload limits (Cloudflare's proxy caps
request bodies at 100 MB on free plans) or when you want different proxy
settings for bulk transfers. With the S3 backend the apps talk to the
bucket's endpoint (`public_endpoint`) instead.

The web app (if you host it) is a third hostname serving static files; it
calls the API cross-origin, which Photovault allows (CORS mirrors the origin,
accepts any request header, exposes `ETag`).

## What the proxy must do

* Pass the `Host` header through and add `X-Forwarded-For`/`-Proto`.
* **No body size limit** on `/storage/` (parts are tens of MB, a
  non-multipart upload can be larger) and do not buffer request bodies.
* Long timeouts for `/storage/` (uploads from slow links): 1 hour is safe.
* Keep `Range` requests intact (video seeking).
* Do not cache `/storage/` responses: each URL is signed and expires.
* Keep `/metrics` internal.

## Nginx Proxy Manager

Proxy host `photos.example.com` → `http://photovault:8080`, *Block Common
Exploits* off, *Websockets* off, SSL with Let's Encrypt and *Force SSL*. In
*Advanced*:

```nginx
client_max_body_size 0;
proxy_request_buffering off;
proxy_buffering off;
proxy_read_timeout 3600s;
proxy_send_timeout 3600s;
location = /metrics { return 404; }
```

Repeat for `storage.photos.example.com` if you use it.

## Caddy

```caddy
photos.example.com, storage.photos.example.com {
    @metrics path /metrics
    respond @metrics 404
    reverse_proxy photovault:8080 {
        flush_interval -1
        transport http {
            read_timeout 1h
            write_timeout 1h
        }
    }
}
```

Caddy has no request body limit by default.

## Traefik (labels on the photovault service)

```yaml
labels:
  - traefik.enable=true
  - traefik.docker.network=proxy_net
  - traefik.http.routers.photovault.rule=Host(`photos.example.com`) || Host(`storage.photos.example.com`)
  - traefik.http.routers.photovault.entrypoints=websecure
  - traefik.http.routers.photovault.tls.certresolver=letsencrypt
  - traefik.http.services.photovault.loadbalancer.server.port=8080
  - traefik.http.routers.photovault-metrics.rule=Path(`/metrics`)
  - traefik.http.routers.photovault-metrics.priority=1000
  - traefik.http.routers.photovault-metrics.middlewares=photovault-deny
  - traefik.http.middlewares.photovault-deny.ipallowlist.sourcerange=127.0.0.1/32
```

Traefik does not limit body size unless you add the `buffering` middleware;
set `respondingTimeouts.readTimeout` on the entrypoint to `1h` (its default is
60 s since v3, which aborts slow uploads).

## Checking

```sh
curl -fsS https://photos.example.com/ping
curl -fsS https://photos.example.com/healthz
```
