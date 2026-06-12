use std::time::Duration;

use azalea::Client;
use std::fmt::Write as _;
use tokio::time::sleep;

use crate::State;

#[derive(Debug, Default)]
pub struct PlayerStats {
    pub level: Option<u32>,
    pub xp: Option<u32>,
    pub coins: Option<u64>,
    pub kills: Option<u32>,
    pub deaths: Option<u32>,
    pub kd: Option<f32>,
    pub final_kills: Option<u32>,
    pub final_deaths: Option<u32>,
    pub fk_fd: Option<f32>,
    pub beds_broken: Option<u32>,
    pub wins: Option<u32>,
    pub losses: Option<u32>,
    pub winrate: Option<f32>,
    pub division: Option<String>,
    pub division_xp: Option<u32>,
    pub winstreak: Option<u32>,
    pub max_winstreak: Option<u32>,
}

const STAT_PREFIXES: &[&str] = &[
    "Livello: ",
    "XP: ",
    "Coins: ",
    "Uccisioni: ",
    "Morti: ",
    "K/D: ",
    "Uccisioni finali: ",
    "Morti finali: ",
    "FK/FD: ",
    "Letti distrutti: ",
    "Partite vinte: ",
    "Divisione: ",
    "XP Divisione: ",
    "Winstreak/Max winstreak: ",
];

pub fn is_stat_line(line: &str) -> bool {
    let line = line.trim();
    STAT_PREFIXES.iter().any(|p| line.starts_with(p))
}

pub async fn get_stats(name: &str, bot: &Client, state: &State) -> PlayerStats {
    bot.chat(format!("/stats {name}"));
    sleep(Duration::from_secs(1)).await;
    let messages = state.messages.lock().await;
    let stat_lines: Vec<String> = messages
        .iter()
        .skip_while(|m| !is_stat_line(&m.to_string()))
        .take_while(|m| is_stat_line(&m.to_string()))
        .map(ToString::to_string)
        .collect();
    drop(messages);
    PlayerStats::parse_from_chat(stat_lines)
}

pub fn write_player_stats_markdown(stats: &PlayerStats, content: &mut String) {
    if let Some(level) = stats.level {
        _ = writeln!(content, "• Level: {level}");
    }
    if let (Some(k), Some(d)) = (stats.kills, stats.deaths) {
        _ = write!(content, "• Kills: {k} | Deaths: {d}");
        if let Some(kd) = stats.kd {
            _ = write!(content, " | K/D: {kd:.2}");
        }
        _ = writeln!(content);
    }
    if let (Some(fk), Some(fd)) = (stats.final_kills, stats.final_deaths) {
        _ = write!(content, "• Final Kills: {fk} | Final Deaths: {fd}");
        if let Some(r) = stats.fk_fd {
            _ = write!(content, " | FK/FD: {r:.2}");
        }
        _ = writeln!(content);
    }
    if let Some(beds) = stats.beds_broken {
        _ = writeln!(content, "• Beds Broken: {beds}");
    }
    if let (Some(w), Some(l)) = (stats.wins, stats.losses) {
        _ = write!(content, "• Wins: {w} | Losses: {l}");
        if let Some(rate) = stats.winrate {
            _ = write!(content, " | Winrate: {rate:.2}%");
        }
        _ = writeln!(content);
    }
    if let Some(div) = &stats.division {
        _ = write!(content, "• Division: {div}");
        if let Some(xp) = stats.division_xp {
            _ = write!(content, " ({xp} XP)");
        }
        _ = writeln!(content);
    }
    if let Some(ws) = stats.winstreak {
        _ = write!(content, "• Winstreak: {ws}");
        if let Some(max) = stats.max_winstreak {
            _ = write!(content, " (max: {max})");
        }
        _ = writeln!(content);
    }
}

impl PlayerStats {
    pub fn parse_from_chat<I>(lines: I) -> Self
    where
        I: IntoIterator,
        I::Item: AsRef<str>,
    {
        let mut stats = Self::default();

        for line in lines {
            let line = line.as_ref().trim();

            if let Some(v) = line.strip_prefix("Livello: ") {
                stats.level = v.trim().parse().ok();
            } else if let Some(v) = line.strip_prefix("XP: ") {
                stats.xp = v.replace(',', "").parse().ok();
            } else if let Some(v) = line.strip_prefix("Coins: ") {
                stats.coins = v.replace(',', "").parse().ok();
            } else if let Some(v) = line.strip_prefix("Uccisioni: ") {
                stats.kills = v.replace(',', "").parse().ok();
            } else if let Some(v) = line.strip_prefix("Morti: ") {
                stats.deaths = v.replace(',', "").parse().ok();
            } else if let Some(v) = line.strip_prefix("K/D: ") {
                stats.kd = v.parse().ok();
            } else if let Some(v) = line.strip_prefix("Uccisioni finali: ") {
                stats.final_kills = v.replace(',', "").parse().ok();
            } else if let Some(v) = line.strip_prefix("Morti finali: ") {
                stats.final_deaths = v.replace(',', "").parse().ok();
            } else if let Some(v) = line.strip_prefix("FK/FD: ") {
                stats.fk_fd = v.parse().ok();
            } else if let Some(v) = line.strip_prefix("Letti distrutti: ") {
                stats.beds_broken = v.replace(',', "").parse().ok();
            } else if let Some(v) = line.strip_prefix("Partite vinte: ") {
                let parts: Vec<_> = v.split_whitespace().collect();
                if let Some(win_loss) = parts.first() {
                    let mut wl = win_loss.split('/');
                    stats.wins = wl.next().and_then(|v| v.replace(',', "").parse().ok());
                    stats.losses = wl.next().and_then(|v| v.replace(',', "").parse().ok());
                }

                if let Some(rate) = parts.get(1) {
                    stats.winrate = rate
                        .trim_matches(|c| c == '(' || c == ')' || c == '%')
                        .parse()
                        .ok();
                }
            } else if let Some(v) = line.strip_prefix("Divisione: ") {
                if let Some((name, _)) = v.split_once('(') {
                    stats.division = Some(name.trim().to_string());
                }
            } else if let Some(v) = line.strip_prefix("XP Divisione: ") {
                stats.division_xp = v.replace(',', "").parse().ok();
            } else if let Some(v) = line.strip_prefix("Winstreak/Max winstreak: ") {
                let mut parts = v.split('/');
                stats.winstreak = parts.next().and_then(|v| v.trim().parse().ok());
                stats.max_winstreak = parts.next().and_then(|v| v.trim().parse().ok());
            }
        }

        stats
    }
    pub fn player_stats_to_llm_text(&self) -> String {
        let mut parts = Vec::new();

        if let Some(level) = self.level {
            parts.push(format!("level {level}"));
        }

        if let Some(coins) = self.coins {
            parts.push(format!("{coins} coins"));
        }

        if let (Some(k), Some(d)) = (self.kills, self.deaths) {
            if let Some(kd) = self.kd {
                parts.push(format!("{k} kills and {d} deaths (K/D {kd:.2})"));
            } else {
                parts.push(format!("{k} kills and {d} deaths"));
            }
        }

        if let (Some(fk), Some(fd)) = (self.final_kills, self.final_deaths) {
            if let Some(r) = self.fk_fd {
                parts.push(format!(
                    "{fk} final kills and {fd} final deaths (FK/FD {r:.2})"
                ));
            } else {
                parts.push(format!("{fk} final kills and {fd} final deaths"));
            }
        }

        if let Some(beds) = self.beds_broken {
            parts.push(format!("{beds} beds broken"));
        }

        if let (Some(w), Some(l)) = (self.wins, self.losses) {
            if let Some(rate) = self.winrate {
                parts.push(format!("{w} wins and {l} losses ({rate:.2}% winrate)"));
            } else {
                parts.push(format!("{w} wins and {l} losses"));
            }
        }

        if let Some(div) = &self.division {
            if let Some(xp) = self.division_xp {
                parts.push(format!("division {div} with {xp} division XP"));
            } else {
                parts.push(format!("division {div}"));
            }
        }

        if let Some(ws) = self.winstreak {
            if let Some(max) = self.max_winstreak {
                parts.push(format!("current winstreak {ws} (max {max})"));
            } else {
                parts.push(format!("winstreak {ws}"));
            }
        }

        parts.join(", ")
    }
}
