/// A sqlite cache that stores the result of Hacker News API requests.
///
/// This cache is used to store the results of Hacker News API requests so that we can serve them
/// faster to users. This cache is backed by an SQLite database.
use tokio_rusqlite::{params, Connection};

/// The cache struct that stores the connection to the SQLite database.
pub struct Cache {
    conn: Connection,
}

impl Cache {
    /// Create a new cache instance.
    ///
    /// This function creates a new cache instance and initializes the SQLite database.
    pub async fn new() -> anyhow::Result<Self> {
        // Call the asynchronous connect method using the runtime.
        let conn = Connection::open("db/cache.db").await?;

        conn.call(|conn| {
            conn.execute(
                "CREATE TABLE IF NOT EXISTS cache (
                id INTEGER PRIMARY KEY,
                url TEXT NOT NULL,
                response TEXT NOT NULL
            )",
                [],
            )?;

            tokio_rusqlite::Result::Ok(())
        })
        .await?;
        Ok(Self { conn })
    }

    /// Get a cached response from the cache.
    ///
    /// This function retrieves a cached response from the cache based on the URL provided.
    pub async fn get(&self, url: &str) -> anyhow::Result<Option<String>> {
        let url = url.to_string();

        let result = self
            .conn
            .call(move |conn| {
                let mut stmt = conn.prepare("SELECT response FROM cache WHERE url = ?")?;
                let mut rows = stmt.query(params![url])?;
                if let Some(row) = rows.next()? {
                    let response: String = row.get(0)?;
                    Ok(Some(response))
                } else {
                    Ok(None)
                }
            })
            .await?;

        Ok(result)
    }

    /// Set a cached response in the cache.
    ///
    /// This function sets a cached response in the cache based on the URL and response provided.
    pub async fn set(&self, url: &str, response: &str) -> anyhow::Result<()> {
        let url = url.to_string();
        let response = response.to_string();

        let result = self
            .conn
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO cache (url, response) VALUES (?1, ?2)",
                    params![url, response],
                )?;
                Ok(())
            })
            .await?;

        Ok(result)
    }
}
