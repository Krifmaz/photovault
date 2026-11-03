//! `[accounts]` config section: URLs of the companion web apps and the WebAuthn relying
//! party (museum's `apps.*` and `webauthn.*` keys).

use serde::Deserialize;

/// Account-related settings.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AccountsSettings {
    /// Accounts web app (passkeys, family invites, account recovery). Museum `apps.accounts`.
    pub accounts_url: String,
    /// Family management web app. Museum `apps.family`.
    pub family_url: String,
    /// Cast receiver web app. Museum `apps.cast`.
    pub cast_url: String,
    /// Embeddable public albums. Museum `apps.embed-albums`.
    pub embed_albums_url: String,
    /// CNAME target users point custom domains at. Museum `apps.custom-domain.cname`.
    pub custom_domain_cname: String,
    /// WebAuthn relying-party id (a registrable domain of the accounts app). Museum `webauthn.rpid`.
    pub webauthn_rp_id: String,
    /// Origins allowed in WebAuthn client data. Museum `webauthn.rporigins`.
    pub webauthn_rp_origins: Vec<String>,
    /// Relying-party display name shown by authenticators.
    pub webauthn_rp_name: String,
    /// Origin of the public paste web app; `/paste/consume` rejects other origins when set.
    /// Museum `apps.public-paste`.
    pub paste_url: String,
    /// Public albums web app used in album and file share links (empty: `public_url`).
    /// Museum `apps.public-albums`.
    pub public_albums_url: String,
    /// Public Locker share web app used in Locker links (empty: `public_url`).
    /// Museum `apps.public-locker`.
    pub public_locker_url: String,
    /// Public memories web app used in memory share links (empty: `public_url`).
    /// Museum `apps.public-memories`.
    pub public_memories_url: String,
}

impl Default for AccountsSettings {
    fn default() -> Self {
        // Same defaults as museum's configurations/local.yaml.
        Self {
            accounts_url: "http://localhost:3001".into(),
            family_url: "http://localhost:3003".into(),
            cast_url: String::new(),
            embed_albums_url: String::new(),
            custom_domain_cname: String::new(),
            webauthn_rp_id: "localhost".into(),
            webauthn_rp_origins: vec!["http://localhost:3001".into()],
            webauthn_rp_name: "Photovault".into(),
            paste_url: String::new(),
            public_albums_url: String::new(),
            public_locker_url: String::new(),
            public_memories_url: String::new(),
        }
    }
}

impl AccountsSettings {
    /// `PHOTOVAULT_ACCOUNTS_URL`, `PHOTOVAULT_FAMILY_URL`, `PHOTOVAULT_CAST_URL`,
    /// `PHOTOVAULT_EMBED_ALBUMS_URL`, `PHOTOVAULT_CUSTOM_DOMAIN_CNAME`,
    /// `PHOTOVAULT_PASTE_URL`, `PHOTOVAULT_ALBUMS_URL`, `PHOTOVAULT_LOCKER_SHARE_URL`,
    /// `PHOTOVAULT_PUBLIC_MEMORIES_URL`, `PHOTOVAULT_WEBAUTHN_RP_ID`,
    /// `PHOTOVAULT_WEBAUTHN_RP_ORIGINS` (comma separated).
    pub fn apply_env(&mut self) {
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        if let Some(v) = env("PHOTOVAULT_ACCOUNTS_URL") {
            self.accounts_url = v;
        }
        if let Some(v) = env("PHOTOVAULT_FAMILY_URL") {
            self.family_url = v;
        }
        if let Some(v) = env("PHOTOVAULT_CAST_URL") {
            self.cast_url = v;
        }
        if let Some(v) = env("PHOTOVAULT_EMBED_ALBUMS_URL") {
            self.embed_albums_url = v;
        }
        if let Some(v) = env("PHOTOVAULT_CUSTOM_DOMAIN_CNAME") {
            self.custom_domain_cname = v;
        }
        if let Some(v) = env("PHOTOVAULT_PASTE_URL") {
            self.paste_url = v;
        }
        if let Some(v) = env("PHOTOVAULT_ALBUMS_URL") {
            self.public_albums_url = v;
        }
        if let Some(v) = env("PHOTOVAULT_LOCKER_SHARE_URL") {
            self.public_locker_url = v;
        }
        if let Some(v) = env("PHOTOVAULT_PUBLIC_MEMORIES_URL") {
            self.public_memories_url = v;
        }
        if let Some(v) = env("PHOTOVAULT_WEBAUTHN_RP_ID") {
            self.webauthn_rp_id = v;
        }
        if let Some(v) = env("PHOTOVAULT_WEBAUTHN_RP_ORIGINS") {
            self.webauthn_rp_origins = v.split(',').map(|s| s.trim().to_string()).collect();
        }
    }
}
