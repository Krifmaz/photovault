# LAN discovery

The server announces itself on the local network with mDNS/DNS-SD, so the
Photovault apps can list it on the *Connect to your server* screen without
anyone typing an address.

## What is announced

Service type `_photovault._tcp`. The instance name is
`PHOTOVAULT_SERVER_NAME` (default `Photovault`). The port is the port of
`PHOTOVAULT_HOME_URL` when that is set, otherwise the port the server
listens on.

| TXT key | Value |
| --- | --- |
| `name` | `PHOTOVAULT_SERVER_NAME` |
| `version` | server version |
| `public` | `PHOTOVAULT_PUBLIC_URL` |
| `home` | `PHOTOVAULT_HOME_URL` (only when set) |
| `info` | `/photovault/info` |

An app that finds the service reads `/photovault/info` from it before it
offers the server.

Check from another machine on the LAN:

```sh
avahi-browse -rt _photovault._tcp      # Linux
dns-sd -B _photovault._tcp             # macOS
```

Turn it off with `PHOTOVAULT_MDNS=false`. If announcing fails at start the
server logs a warning and carries on.

## Docker

Multicast does not leave Docker's default bridge network. With the usual
`ports: - "8095:8080"` the server's own announcement never reaches the LAN.
Pick one of these:

1. **The `photovault-mdns` sidecar** (recommended). A second service from the
   same image runs `photovault announce` on the host network. Every minute it
   reads `/photovault/info` from the server and announces what it finds, so a
   new name or address is picked up without a restart. It is in
   `deploy/compose.yaml`, commented out:

   ```yaml
   photovault-mdns:
     image: ghcr.io/krifmaz/photovault:edge
     restart: unless-stopped
     command: ["announce", "--server", "http://127.0.0.1:8095"]
     network_mode: host
     healthcheck: {disable: true}
     read_only: true
     security_opt: [no-new-privileges:true]
     cap_drop: [ALL]
   ```

   `--server` is the server as the sidecar sees it: the published port on
   the host. `--port` overrides the announced port (default: the port of
   `homeUrl`, else of `--server`). The healthcheck is disabled because the
   image's built-in check probes port 8080, which is not on the host. You
   can then set `PHOTOVAULT_MDNS=false` on the main service.

2. **Host networking for the server.** `network_mode: host` on the
   `photovault` service. Drop `ports:`, set `PHOTOVAULT_LISTEN` to the port
   you want (e.g. `0.0.0.0:8095`) and `PHOTOVAULT_HEALTHCHECK_URL` to match
   (`http://127.0.0.1:8095/healthz`). Simple, but the container shares the
   host's network.

3. **An mDNS reflector.** If the host already runs Avahi, set
   `enable-reflector=yes` in `/etc/avahi/avahi-daemon.conf` so it repeats
   announcements between the Docker bridge and the LAN.

## Proxmox LXC

mDNS works inside an LXC on a bridged network (`vmbr0`). Use the sidecar or
host networking inside the LXC as above. VLANs or Wi-Fi client isolation
between phone and server stop multicast; then enter the address by hand or
scan the QR code from the web app's settings.
