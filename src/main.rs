mod cache;
mod hacker_news;

use std::{borrow::Cow, collections::HashMap, sync::Arc};

use anyhow::bail;
use askama::Template;
use axum::{
    error_handling::HandleErrorLayer,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::get,
    Extension, Router,
};
use axum_macros::debug_handler;
use serde_json::Value;
use tower::{BoxError, ServiceBuilder};
use tower_http::services::ServeDir;
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // initialize tracing
    tracing_subscriber::fmt::init();

    let state = SharedState::new(State::new().await);

    // Fresh all hacker news video first
    {
        let s = state.clone();
        let counter = hacker_news::Counter::new();

        let c = counter.clone();
        let job = tokio::spawn(async move {
            let _ = s.hn.get_top_videos(Some(c)).await?;
            Ok::<(), anyhow::Error>(())
        });

        let mut pb: Option<pbr::ProgressBar<std::io::Stdout>> = None;

        loop {
            let (_, d, t) = counter.read().unwrap().counter();
            match pb {
                Some(ref mut p) => {
                    p.set(d as u64);
                }
                None if t != 0 => {
                    let mut p = pbr::ProgressBar::new(t as u64);
                    p.set(d as u64);
                    pb = Some(p);
                }
                _ => {
                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                    continue;
                }
            }
            if job.is_finished() {
                break;
            }

            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        }

        // Throw error if job failed
        let _ = job.await??;
    }

    let s = ServiceBuilder::new()
        .layer(HandleErrorLayer::new(handle_error))
        .load_shed()
        .concurrency_limit(1024)
        // .timeout(Duration::from_secs(10))
        .layer(Extension(state));

    // build our application with a route
    let app = Router::new()
        .route("/", get(root))
        .nest_service("/assets", ServeDir::new("assets"))
        .layer(s);

    // run our app with hyper, listening globally on port 3000
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;

    info!("Listening on: {}", listener.local_addr()?);
    axum::serve(listener, app).await?;

    Ok(())
}

#[debug_handler]
async fn root(Extension(state): Extension<SharedState>) -> Result<impl IntoResponse, AppError> {
    macro_rules! field {
        ($v:ident, $field:literal, $type:ident) => {
            match $v.get($field) {
                Some(Value::$type(val)) => val,
                _ => bail!(concat!($field, " not found")),
            }
        };
    }

    let videos: Result<Vec<Video>, anyhow::Error> = state
        .hn
        .get_top_videos(None)
        .await?
        .into_iter()
        .map(|json| -> anyhow::Result<Video> {
            let video: HashMap<String, Value> = serde_json::from_str(&json)?;
            let url = field!(video, "url", String).clone();
            let title = field!(video, "title", String).clone();
            let hn_link = format!(
                "https://news.ycombinator.com/item?id={}",
                field!(video, "id", Number)
            );

            Ok(Video {
                title,
                hn_link,
                url,
            })
        })
        .collect();

    let videos = videos?;
    let template = IndexTemplate { videos };
    Ok(HtmlTemplate(template))
}

/// Make our own error that wraps `anyhow::Error`.
struct AppError(anyhow::Error);

// Tell axum how to convert `AppError` into a response.
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Something went wrong: {}", self.0),
        )
            .into_response()
    }
}

// This enables using `?` on functions that return `Result<_, anyhow::Error>` to turn them into
// `Result<_, AppError>`. That way you don't need to do that manually.
impl<E> From<E> for AppError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        Self(err.into())
    }
}

type SharedState = Arc<State>;

struct State {
    hn: hacker_news::HackerNews,
}

impl State {
    async fn new() -> Self {
        Self {
            hn: hacker_news::HackerNews::new()
                .await
                .expect("Failed to create HackerNews instance"),
        }
    }
}

struct Video {
    title: String,
    hn_link: String,
    url: String,
}

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate {
    videos: Vec<Video>,
}

/// A wrapper type that we'll use to encapsulate HTML parsed by askama into valid HTML for axum to serve.
struct HtmlTemplate<T>(T);

/// Allows us to convert Askama HTML templates into valid HTML for axum to serve in the response.
impl<T> IntoResponse for HtmlTemplate<T>
where
    T: Template,
{
    fn into_response(self) -> Response {
        // Attempt to render the template with askama
        match self.0.render() {
            // If we're able to successfully parse and aggregate the template, serve it
            Ok(html) => Html(html).into_response(),
            // If we're not, return an error or some bit of fallback HTML
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template. Error: {}", err),
            )
                .into_response(),
        }
    }
}

async fn handle_error(error: BoxError) -> impl IntoResponse {
    if error.is::<tower::timeout::error::Elapsed>() {
        return (StatusCode::REQUEST_TIMEOUT, Cow::from("request timed out"));
    }

    if error.is::<tower::load_shed::error::Overloaded>() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Cow::from("service is overloaded, try again later"),
        );
    }

    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Cow::from(format!("Unhandled internal error: {}", error)),
    )
}
