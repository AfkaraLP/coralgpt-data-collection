use chrono::Utc;
use rusqlite::params;

pub struct Message {
    name: String,
    message: String,
    stars: usize,
    timestamp: String,
    rank: String,
}
impl Message {
    #[must_use]
    pub fn parse(message: &str) -> Option<Self> {
        let timestamp = Utc::now().to_rfc3339();
        let (metadata, message) = message.split_once(':')?;
        let message = message.trim().to_string();

        let mut metadata = metadata.trim().split_whitespace();

        let stars_str = metadata.next()?;
        let stars = stars_str
            .chars()
            .filter(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse::<usize>()
            .ok()?;

        let (rank, name) = match metadata.clone().count() {
            1 => ("Non".to_string(), metadata.next()?.to_string()),
            2 => (metadata.next()?.to_string(), metadata.next()?.to_string()),
            _ => return None,
        };

        Some(Self {
            name,
            message,
            stars,
            timestamp,
            rank,
        })
    }

    pub fn insert(&self, conn: &rusqlite::Connection) -> rusqlite::Result<()> {
        conn.execute(
            "INSERT INTO messages (username, timestamp, starcount, rank, message) 
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                self.name,
                self.timestamp,
                self.stars as i64,
                self.rank,
                self.message
            ],
        )?;
        Ok(())
    }
}
