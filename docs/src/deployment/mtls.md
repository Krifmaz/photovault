# Optional mTLS device certificates

Photovault's own security does not depend on the network: everything is
end-to-end encrypted and every request needs a session token. Requiring a
client certificate at the proxy adds a second gate: only devices holding a
certificate you issued can even reach the login endpoints.

Photovault itself needs no changes for this; it is configured entirely at
the reverse proxy.

## Know what the stock apps can do

| Client | Presents a client certificate? |
| --- | --- |
| Web app in a browser | Yes: install the certificate in the OS/browser store; the browser prompts once. |
| Desktop app (Electron) | Usually: Electron picks a matching certificate from the OS store. |
| Mobile apps (Flutter) | **No.** The stock apps do not load client certificates. |

So the practical pattern is **a second hostname**:

* `photos.example.com`: reachable only on your LAN or VPN (WireGuard,
  Tailscale), no mTLS; the phones use this.
* `photos-mtls.example.com`: published to the internet, mTLS required; browsers
  and desktops use this away from home.

If you enable mTLS on the storage hostname as well, every signed
upload/download URL also needs the certificate. Browsers send it
automatically for the same certificate authority; give the mTLS API hostname
its own storage hostname (or the same one) consistently.

## Create a small CA and a device certificate

```sh
# CA (keep ca.key offline)
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes \
  -keyout ca.key -out ca.crt -days 3650 -subj "/CN=Photovault devices"

# One certificate per device
openssl req -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes \
  -keyout laptop.key -out laptop.csr -subj "/CN=laptop"
openssl x509 -req -in laptop.csr -CA ca.crt -CAkey ca.key -CAcreateserial \
  -days 825 -out laptop.crt -extfile <(printf 'extendedKeyUsage=clientAuth')
openssl pkcs12 -export -inkey laptop.key -in laptop.crt -out laptop.p12
```

Import `laptop.p12` into the device's certificate store. To revoke a device,
publish a CRL and point the proxy at it, or rotate the CA.

## Caddy

```caddy
photos-mtls.example.com {
    tls {
        client_auth {
            mode require_and_verify
            trust_pool file /etc/caddy/photovault-ca.crt
        }
    }
    reverse_proxy photovault:8080
}
```

## Nginx / Nginx Proxy Manager (Advanced tab)

```nginx
ssl_client_certificate /data/custom_ssl/photovault-ca.crt;
ssl_verify_client on;
ssl_verify_depth 1;
```

## Traefik

```yaml
# dynamic configuration
tls:
  options:
    photovault-mtls:
      clientAuth:
        caFiles: [/certs/photovault-ca.crt]
        clientAuthType: RequireAndVerifyClientCert
```

and on the router: `traefik.http.routers.photovault-mtls.tls.options=photovault-mtls@file`.

## CORS preflights

Browsers send CORS preflight (`OPTIONS`) requests **without** a client
certificate on some platforms. If uploads from the web app fail only with
mTLS on, use `mode verify_if_given` (Caddy) / `ssl_verify_client optional`
(nginx) and reject requests without a verified certificate except `OPTIONS`:

```nginx
ssl_verify_client optional;
set $mtls_ok 0;
if ($request_method = OPTIONS) { set $mtls_ok 1; }
if ($ssl_client_verify = SUCCESS) { set $mtls_ok 1; }
if ($mtls_ok != 1) { return 403; }
```
