# HTTPS behind a reverse proxy

Photovault speaks plain HTTP on port 8080. For access from outside your home,
put a reverse proxy in front of it to add HTTPS.
[`deploy/compose.domain.yaml`](https://github.com/Krifmaz/photovault/blob/main/deploy/compose.domain.yaml)
is the matching compose file.

## Two hostnames

| Hostname | Setting | Points to | Used for |
| --- | --- | --- | --- |
| `photos.example.com` | `PHOTOVAULT_PUBLIC_URL` | `photovault:8080` | API and web app. What you enter in the apps. |
| `storage.photos.example.com` | `PHOTOVAULT_STORAGE_PUBLIC_URL` | `photovault:8080` | Signed upload and download URLs (`/storage/…`). |

Both go to the same container. One hostname also works (leave
`PHOTOVAULT_STORAGE_PUBLIC_URL` unset), but a separate storage hostname lets
you give bulk transfers their own proxy settings and keep them away from a
CDN. With the S3 backend the apps talk to the bucket's endpoint
(`PHOTOVAULT_S3_PUBLIC_ENDPOINT`) instead, and the storage hostname is not
used.

Set `PHOTOVAULT_HOME_URL` to the LAN address (for example
`http://192.168.1.10:8095`) so the apps can skip the proxy at home.

## Cloudflare

If your DNS is on Cloudflare, set the **storage hostname to DNS only**
(grey cloud). Cloudflare's proxy limits request bodies to 100 MB on most
plans, and the apps upload large videos in parts bigger than that, so those
uploads fail. The API hostname can stay proxied (orange cloud): its requests
are small.

## What the proxy must do

* Pass the `Host` header through and add `X-Forwarded-For` and
  `X-Forwarded-Proto`.
* **No request body limit** on the storage hostname, and no request
  buffering. A single upload part is tens of MB; an upload without parts
  can be larger.
* Long timeouts for the storage hostname: 1 hour is safe for slow uplinks.
* Keep `Range` requests intact (video seeking).
* Do not cache `/storage/` responses: each URL is signed and expires.
* Keep `/metrics` internal.

## Nginx and Nginx Proxy Manager

Proxy host `photos.example.com` → `http://photovault:8080`, *Block Common
Exploits* off, SSL with Let's Encrypt and *Force SSL*. In *Advanced*, or in
the `server` block of plain nginx:

```nginx
client_max_body_size 0;          # no body limit (nginx's default is 1 MB)
proxy_request_buffering off;     # stream uploads instead of spooling them to disk
proxy_buffering off;
proxy_read_timeout 3600s;
proxy_send_timeout 3600s;
location = /metrics { return 404; }
```

Add the same proxy host for `storage.photos.example.com`.

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

Caddy has no request body limit unless you add a `request_body` directive
with `max_size`; do not add one for these hostnames. `flush_interval -1`
streams responses without buffering.

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

Traefik does not limit body size unless you add the `buffering` middleware.
Set `respondingTimeouts.readTimeout` on the entrypoint to `1h`: its default
of 60 s aborts slow uploads.

## Checking

```sh
curl -fsS https://photos.example.com/photovault/info
curl -fsS https://photos.example.com/healthz
```

`/photovault/info` should show your `publicUrl` and, if set, `homeUrl`.
