-- Photovault: settings changed with the admin CLI (e.g. `photovault admin signups on`).
CREATE TABLE photovault_settings (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);
