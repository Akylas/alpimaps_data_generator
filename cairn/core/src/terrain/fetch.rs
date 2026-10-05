//! Blocking HTTP GETs, byte ranges included, for use inside the synchronous render loop.
//!
//! The terrain renderer samples a pixel at a time and is wholly synchronous, while the rest of
//! this crate is async. A remote tile source has to be read from inside that loop, where there is
//! nothing to await with - and `Runtime::block_on` panics when it is called from a thread that is
//! already inside a runtime, which the render loop is: `cairn terrain` is an `async fn`. So the
//! requests are handed to a thread that owns a runtime of its own, and the caller blocks on a
//! channel instead.
//!
//! Failures are retried here rather than reported upwards, because the sampling path has no error
//! channel: a source that cannot answer looks exactly like a source with no data there, and a
//! transient 503 would otherwise punch a silent hole in the archive. What retrying cannot fix is
//! counted, so the render can say so at the end.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};

/// Sent with every request, so an operator of a public tile service can tell who is asking.
const USER_AGENT: &str = concat!("cairn/", env!("CARGO_PKG_VERSION"), " (terrain builder)");

/// Attempts per request, including the first.
const ATTEMPTS: u32 = 3;

struct Job {
    url: String,
    /// Inclusive byte range, as HTTP spells it.
    range: Option<(u64, u64)>,
    reply: Sender<Result<Option<Vec<u8>>>>,
}

/// A handle onto the fetch thread. Dropping every handle stops it.
pub struct Fetcher {
    jobs: Sender<Job>,
    requests: Arc<AtomicU64>,
    bytes: Arc<AtomicU64>,
    failures: Arc<AtomicU64>,
}

impl Default for Fetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Fetcher {
    pub fn new() -> Self {
        let (jobs, inbox) = channel::<Job>();
        let requests = Arc::new(AtomicU64::new(0));
        let bytes = Arc::new(AtomicU64::new(0));
        let failures = Arc::new(AtomicU64::new(0));
        let (counted, measured, failed) = (requests.clone(), bytes.clone(), failures.clone());

        std::thread::Builder::new()
            .name("terrain-fetch".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                let client = reqwest::Client::builder().user_agent(USER_AGENT).build();
                let (runtime, client) = match (runtime, client) {
                    (Ok(r), Ok(c)) => (r, c),
                    (runtime, client) => {
                        // without either one nothing can be fetched; answer every request with
                        // the reason rather than hanging the render on a silent channel
                        let why = runtime
                            .err()
                            .map(|e| e.to_string())
                            .or_else(|| client.err().map(|e| e.to_string()))
                            .unwrap_or_default();
                        for job in inbox {
                            let _ = job.reply.send(Err(anyhow!("no HTTP client: {why}")));
                        }
                        return;
                    }
                };
                for job in inbox {
                    counted.fetch_add(1, Ordering::Relaxed);
                    let out = runtime.block_on(attempt(&client, &job.url, job.range));
                    match &out {
                        Ok(Some(body)) => {
                            measured.fetch_add(body.len() as u64, Ordering::Relaxed);
                        }
                        Err(_) => {
                            failed.fetch_add(1, Ordering::Relaxed);
                        }
                        Ok(None) => {}
                    }
                    let _ = job.reply.send(out);
                }
            })
            .expect("spawning the terrain fetch thread");

        Self { jobs, requests, bytes, failures }
    }

    /// Fetch a URL, or a byte range of it. `None` means the server says there is nothing there.
    pub fn get(&self, url: &str, range: Option<(u64, u64)>) -> Result<Option<Vec<u8>>> {
        let (reply, answer) = channel();
        self.jobs
            .send(Job { url: url.to_string(), range, reply })
            .map_err(|_| anyhow!("the fetch thread has stopped"))?;
        answer.recv().map_err(|_| anyhow!("the fetch thread dropped a request"))?
    }

    /// `(requests, bytes, failures)` so far.
    pub fn stats(&self) -> (u64, u64, u64) {
        (
            self.requests.load(Ordering::Relaxed),
            self.bytes.load(Ordering::Relaxed),
            self.failures.load(Ordering::Relaxed),
        )
    }
}

async fn attempt(
    client: &reqwest::Client,
    url: &str,
    range: Option<(u64, u64)>,
) -> Result<Option<Vec<u8>>> {
    let mut last = None;
    for try_number in 0..ATTEMPTS {
        if try_number > 0 {
            // a short, growing pause: the failures worth retrying are the busy ones
            tokio::time::sleep(Duration::from_millis(250 << try_number)).await;
        }
        match get(client, url, range).await {
            Ok(body) => return Ok(body),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| anyhow!("{url} failed")))
}

async fn get(
    client: &reqwest::Client,
    url: &str,
    range: Option<(u64, u64)>,
) -> Result<Option<Vec<u8>>> {
    let mut request = client.get(url);
    if let Some((start, end)) = range {
        request = request.header(reqwest::header::RANGE, format!("bytes={start}-{end}"));
    }
    let response = request.send().await.with_context(|| format!("requesting {url}"))?;
    let status = response.status();

    // A tile that is not there is ordinary - the ocean has none - and a range past the end of an
    // archive means the same thing. Neither is worth stopping a build for.
    if status == reqwest::StatusCode::NOT_FOUND
        || status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE
    {
        return Ok(None);
    }
    if !status.is_success() {
        return Err(anyhow!("{url} returned {status}"));
    }
    // A server that ignores `Range` answers 200 with the whole file. For a planet archive that is
    // hundreds of gigabytes down a pipe nobody asked for, so it is refused before the body is
    // read rather than after.
    if range.is_some() && status != reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(anyhow!("{url} ignored the Range header (answered {status})"));
    }
    Ok(Some(response.bytes().await?.to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A host that cannot resolve must come back as an error rather than hanging the render.
    #[test]
    fn unreachable_hosts_return_an_error() {
        let fetcher = Fetcher::new();
        let out = fetcher.get("https://invalid.invalid/tile.webp", None);
        assert!(out.is_err(), "expected an error, got {out:?}");
        let (requests, _, failures) = fetcher.stats();
        assert_eq!(requests, 1);
        assert_eq!(failures, 1);
    }
}
