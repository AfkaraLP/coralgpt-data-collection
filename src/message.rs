use chrono::Utc;
use rusqlite::params;

pub struct Message {
    pub name: String,
    pub content: String,
    pub stars: usize,
    pub timestamp: String,
    pub rank: String,
    pub ping: Option<i64>,
}
impl Message {
    #[must_use]
    pub fn parse(message: &str) -> Option<Self> {
        let timestamp = Utc::now().to_rfc3339();
        let (metadata, message) = message.split_once(':')?;
        let message = message.trim().to_string();

        let mut metadata = metadata.split_whitespace();

        let stars_str = metadata.next()?;
        let stars = stars_str
            .chars()
            .filter(char::is_ascii_digit)
            .collect::<String>()
            .parse::<usize>()
            .ok()?;

        let (rank, name) = match metadata.clone().count() {
            1 => ("Non".to_string(), metadata.next()?.to_string()),
            2 => {
                let rank = metadata.next()?.to_string();
                if [
                    "[AZZURRO]",
                    "[BIANCO]",
                    "[BLU]",
                    "[GIALLO]",
                    "[ROSA]",
                    "[ROSSO]",
                    "[VERDE]",
                ]
                .contains(&rank.as_str())
                {
                    return None;
                }
                (rank, metadata.next()?.to_string())
            }
            _ => return None,
        };

        Some(Self {
            name,
            content: message,
            stars,
            timestamp,
            rank,
            ping: None,
        })
    }

    #[must_use]
    pub const fn set_ping(mut self, ping: i64) -> Self {
        self.ping.replace(ping);
        self
    }
    #[allow(clippy::cast_possible_wrap)]
    pub fn insert(&self, conn: &rusqlite::Connection) -> rusqlite::Result<()> {
        conn.execute(
            "INSERT INTO messages (username, timestamp, starcount, rank, message, ping) 
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                self.name,
                self.timestamp,
                self.stars as i64,
                self.rank,
                self.content,
                self.ping
            ],
        )?;
        Ok(())
    }
}
