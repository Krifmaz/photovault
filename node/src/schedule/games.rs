//! Matching running processes against the game/launcher list.
//!
//! Each pattern is one of:
//!
//! - `name` or `name:<glob>`: the process name, case-insensitively, where `*`
//!   matches any run of characters and `?` one character
//!   (`gamescope`, `*.exe`, `Battle.net*`);
//! - `cmd:<text>`: the full command line contains `<text>`, case-insensitively
//!   (`cmd:SteamLaunch` catches Steam's `reaper` wrapper that runs every game
//!   started from Steam on Linux).
//!
//! Matching is pure; the probe feeds it names and command lines.

/// A compiled game/launcher pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GamePattern {
    /// Glob over the process name (stored lower-case).
    Name(String),
    /// Substring of the command line (stored lower-case).
    Cmd(String),
}

impl GamePattern {
    /// Parses one pattern; see the module docs for the syntax.
    pub fn parse(s: &str) -> Self {
        if let Some(rest) = s.strip_prefix("cmd:") {
            Self::Cmd(rest.to_lowercase())
        } else {
            Self::Name(s.strip_prefix("name:").unwrap_or(s).to_lowercase())
        }
    }

    /// Whether a process with this name and command line matches.
    pub fn matches(&self, name: &str, cmdline: &str) -> bool {
        match self {
            Self::Name(g) => glob_match(g, &name.to_lowercase()),
            Self::Cmd(t) => !t.is_empty() && cmdline.to_lowercase().contains(t.as_str()),
        }
    }
}

/// A set of patterns plus processes to ignore.
#[derive(Debug, Clone, Default)]
pub struct GameMatcher {
    patterns: Vec<GamePattern>,
    ignore: Vec<GamePattern>,
}

impl GameMatcher {
    /// Builds a matcher from pattern strings and ignore strings.
    pub fn new<S: AsRef<str>>(patterns: &[S], ignore: &[S]) -> Self {
        Self {
            patterns: patterns
                .iter()
                .map(|p| GamePattern::parse(p.as_ref()))
                .collect(),
            ignore: ignore
                .iter()
                .map(|p| GamePattern::parse(p.as_ref()))
                .collect(),
        }
    }

    /// Whether the process matches a pattern and no ignore pattern.
    pub fn check(&self, name: &str, cmdline: &str) -> bool {
        self.patterns.iter().any(|p| p.matches(name, cmdline))
            && !self.ignore.iter().any(|p| p.matches(name, cmdline))
    }

    /// Returns the first matching process name from `(name, cmdline)` pairs.
    pub fn first_match<'a, I>(&self, procs: I) -> Option<String>
    where
        I: IntoIterator<Item = (&'a str, &'a str)>,
    {
        procs
            .into_iter()
            .find(|(n, c)| self.check(n, c))
            .map(|(n, _)| n.to_string())
    }
}

/// Default game/launcher patterns for this platform.
///
/// Linux: Steam's `reaper` (any Steam game, native or Proton), gamescope,
/// Wine/Proton executables (`*.exe`), and common launchers. Windows/macOS:
/// common launchers and Steam's per-game overlay process.
pub fn default_game_patterns() -> Vec<String> {
    let mut v: Vec<&str> = vec![
        // Steam: overlay helper runs once per running game (Windows).
        "gameoverlayui*",
        // Launchers.
        "EpicGamesLauncher*",
        "Battle.net*",
        "GalaxyClient*",
        "EADesktop*",
        "EALauncher*",
        "upc.exe",
        "UbisoftConnect*",
        "RiotClientServices*",
        "heroic",
        "lutris",
        "legendary",
        "minecraft-launcher",
    ];
    if cfg!(target_os = "linux") {
        v.extend([
            // Steam's game wrapper: `reaper SteamLaunch AppId=… -- …`.
            "cmd:SteamLaunch",
            "gamescope",
            "gamescope-wl",
            "gamemoderun",
            // Anything running under Wine/Proton.
            "*.exe",
            "wine64-preloader",
            "wine-preloader",
        ]);
    }
    v.into_iter().map(String::from).collect()
}

/// Default processes never treated as games (Wine helpers that idle in the
/// background).
pub fn default_ignore_patterns() -> Vec<String> {
    if cfg!(target_os = "linux") {
        [
            "services.exe",
            "explorer.exe",
            "winedevice.exe",
            "plugplay.exe",
            "svchost.exe",
            "rpcss.exe",
            "tabtip.exe",
            "conhost.exe",
        ]
        .into_iter()
        .map(String::from)
        .collect()
    } else {
        Vec::new()
    }
}

/// Case-sensitive glob match with `*` and `?` (callers lower-case both sides).
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ti));
            pi += 1;
        } else if let Some((sp, st)) = star {
            pi = sp + 1;
            ti = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs() {
        assert!(glob_match("*.exe", "witcher3.exe"));
        assert!(!glob_match("*.exe", "witcher3.exe.bak"));
        assert!(glob_match("battle.net*", "battle.net.exe"));
        assert!(glob_match("a?c", "abc"));
        assert!(!glob_match("a?c", "ac"));
        assert!(glob_match("*", ""));
        assert!(glob_match("gamescope", "gamescope"));
        assert!(!glob_match("gamescope", "gamescope-wl"));
        assert!(glob_match("*a*b*c", "xxaxxbxxc"));
    }

    #[test]
    fn patterns() {
        let m = GameMatcher::new(
            &["cmd:SteamLaunch", "*.exe", "Battle.net*"],
            &["services.exe"],
        );
        assert!(m.check(
            "reaper",
            "/home/u/.steam/ubuntu12_32/reaper SteamLaunch AppId=1091500 -- …"
        ));
        assert!(m.check("Cyberpunk2077.exe", ""));
        assert!(m.check("battle.net.exe", ""));
        assert!(!m.check("services.exe", ""));
        assert!(!m.check("firefox", "/usr/bin/firefox"));
        assert_eq!(
            m.first_match([("bash", "bash"), ("Game.EXE", ""), ("x.exe", "")]),
            Some("Game.EXE".to_string())
        );
    }

    #[test]
    fn empty_cmd_pattern_matches_nothing() {
        assert!(!GamePattern::parse("cmd:").matches("x", "anything"));
    }

    #[test]
    fn defaults_parse() {
        let m = GameMatcher::new(&default_game_patterns(), &default_ignore_patterns());
        assert!(m.check("EpicGamesLauncher.exe", ""));
        assert!(!m.check("photovault-node", ""));
        assert!(!m.check("ffmpeg", "ffmpeg -i x"));
    }
}
