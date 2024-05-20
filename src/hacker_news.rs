use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

/// Get data from the Hacker News API.
use crate::cache::Cache;
use reqwest::Client;

use serde_json::Value;
use tokio::task::JoinSet;
use tracing::debug;

/// The base URL for the Hacker News API.
const BASE_URL: &str = "https://hacker-news.firebaseio.com/v0";

const BATCH_SIZE: usize = 20;

/// The client used to make requests to the Hacker News API.
struct State {
    client: Client,
    cache: Cache,
}

#[derive(Default)]
pub struct Counter {
    pub pending: usize,
    pub done: usize,
    pub total: usize,
}

impl Counter {
    fn pending(&mut self) {
        self.pending += 1;
    }

    fn done(&mut self) {
        self.done += 1;
    }

    pub fn new() -> Arc<RwLock<Counter>> {
        Default::default()
    }

    pub fn counter(self: &Self) -> (usize, usize, usize) {
        (self.pending, self.done, self.total)
    }
}

pub struct HackerNews {
    state: Arc<State>,
}

impl HackerNews {
    pub async fn new() -> anyhow::Result<Self> {
        let client = Client::new();
        let cache = Cache::new().await?;
        Ok(Self {
            state: Arc::new(State { client, cache }),
        })
    }

    /// Get the top stories from the Hacker News API.
    pub async fn get_top_videos(
        &self,
        counter: Option<Arc<RwLock<Counter>>>,
    ) -> anyhow::Result<Vec<String>> {
        let url = format!("{}/topstories.json", BASE_URL);

        debug!("Fetching fresh response for top stories");
        let top_stories: Vec<i32> = self.state.client.get(&url).send().await?.json().await?;

        if let Some(counter) = counter.as_ref() {
            counter.write().unwrap().total = top_stories.len();
        }

        let mut result = Vec::new();

        let arc = self.state.clone();

        // Fetch the items in batches to avoid hitting the rate limit.
        for i in (0..top_stories.len()).step_by(BATCH_SIZE) {
            let mut tasks = JoinSet::new();

            for id in top_stories.iter().skip(i).take(BATCH_SIZE) {
                tasks.spawn(arc.clone().get_item(counter.clone(), *id));
            }

            while let Some(item) = tasks.join_next().await {
                if let Ok(Some(item)) = item.unwrap() {
                    result.push(item);
                }
            }
        }

        Ok(result)
    }
}

impl State {
    async fn get_item(
        self: Arc<Self>,
        counter: Option<Arc<RwLock<Counter>>>,
        id: i32,
    ) -> anyhow::Result<Option<String>> {
        if let Some(counter) = counter.as_ref() {
            counter.write().unwrap().pending();
        }

        let url = format!("{}/item/{}.json", BASE_URL, id);
        let cached_response = self.cache.get(&url).await?;

        if let Some(json) = cached_response {
            debug!("Using cached response for item {}", id);
            if is_video(&json)? {
                if let Some(counter) = counter.as_ref() {
                    counter.write().unwrap().done();
                }
                return Ok(Some(json));
            }
        } else {
            debug!("Fetching fresh response for item {}", id);
            let json_text = self.client.get(&url).send().await?.text().await?;
            debug!("Fetched response for item {}", id);
            self.cache.set(&url, &json_text).await?;
            if is_video(&json_text)? {
                if let Some(counter) = counter.as_ref() {
                    counter.write().unwrap().done();
                }
                return Ok(Some(json_text));
            }
        }

        if let Some(counter) = counter.as_ref() {
            counter.write().unwrap().done();
        }
        Ok(None)
    }
}

fn is_video(json: &str) -> anyhow::Result<bool> {
    let item: HashMap<String, Value> = serde_json::from_str(&json)?;

    if let Some(item) = item.get("url") {
        if let Some(item) = item.as_str() {
            let item = item.to_ascii_lowercase();

            // if it is from youtube
            if item.contains("http://www.youtube.com/")
                || item.contains("https://www.youtube.com/")
                || item.contains("http://youtu.be/")
                || item.contains("https://youtu.be/")
            {
                return Ok(true);
            }

            // if is has a video tag
            if item.contains("[video]") {
                return Ok(true);
            }
        }
    }

    Ok(false)
}
