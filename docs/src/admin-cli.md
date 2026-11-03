# Admin CLI

`photovault admin …` runs administrative commands against the configured
database (same config file and environment as the server). In Docker:

```sh
docker exec -it photovault-photovault-1 photovault admin list-users
```

| Command | Description |
| --- | --- |
| `admin list-users` | List user ids and storage quotas. |

<!-- The users workstream documents account creation, quotas, OTP and
     sign-up management here. -->

> This page is a placeholder; the full command reference is added together
> with the account-management commands.

Other subcommands of the binary:

| Command | Description |
| --- | --- |
| `photovault serve` | Run the API server (default). |
| `photovault migrate` | Apply database migrations and exit. |
| `photovault healthcheck [--url URL]` | Probe `/healthz`; exit non-zero unless it answers 2xx (Docker `HEALTHCHECK`). |
