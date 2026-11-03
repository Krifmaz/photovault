# Summary

[Introduction](introduction.md)

# Running Photovault

- [Deployment](deployment.md)
  - [Docker on Proxmox LXC with Portainer](deployment/proxmox-portainer.md)
  - [HTTPS behind a reverse proxy](deployment/reverse-proxy.md)
  - [Optional mTLS device certificates](deployment/mtls.md)
- [Configuration](configuration.md)
- [Storage backends](storage.md)
- [Observability](observability.md)
- [Admin CLI](admin-cli.md)

# Plugins

- [Overview](plugins/overview.md)
  - [Writing a plugin](plugins/writing.md)
  - [The manifest](plugins/manifest.md)
  - [Permissions and sandbox](plugins/security.md)
  - [Host/guest ABI](plugins/abi.md)
  - [Running plugins in an app](plugins/host-integration.md)

# Internals

- [Architecture](architecture.md)
- [API compatibility matrix](api-compatibility.md)
- [Keeping the app forks up to date](upstream.md)
