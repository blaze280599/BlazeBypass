use std::collections::VecDeque;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const BYPASS_URL: &str = "https://api.bypass.vip/premium/bypass";
const REFRESH_URL: &str = "https://api.bypass.vip/premium/refresh";

const WORKERS: usize = 3;
const MAX_ATTEMPTS: u32 = 4;
// The API allows 10 requests per 10 seconds; stay just under it.
const RATE_LIMIT: usize = 9;
const RATE_WINDOW: Duration = Duration::from_secs(10);

pub enum Event {
    Done(usize, Result<String, String>),
    Finished,
}

/// Sliding-window limiter shared by all workers.
struct RateLimiter {
    sent: Mutex<VecDeque<Instant>>,
}

impl RateLimiter {
    fn acquire(&self) {
        loop {
            let wait = {
                let mut sent = self.sent.lock().unwrap();
                let now = Instant::now();
                while sent.front().is_some_and(|t| now.duration_since(*t) >= RATE_WINDOW) {
                    sent.pop_front();
                }
                if sent.len() < RATE_LIMIT {
                    sent.push_back(now);
                    return;
                }
                RATE_WINDOW - now.duration_since(*sent.front().unwrap())
            };
            thread::sleep(wait + Duration::from_millis(50));
        }
    }
}

/// Bypasses every link on background threads, reporting each result through `tx`.
pub fn spawn(links: Vec<String>, api_key: String, refresh: bool, tx: Sender<Event>, repaint: impl Fn() + Send + Sync + 'static) {
    let queue = Arc::new(Mutex::new(links.into_iter().enumerate().collect::<VecDeque<_>>()));
    let limiter = Arc::new(RateLimiter { sent: Mutex::new(VecDeque::new()) });
    let repaint = Arc::new(repaint);
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(90)))
        .user_agent("BlazeBypass/1.0")
        .build()
        .into();
    let endpoint = if refresh { REFRESH_URL } else { BYPASS_URL };
    let api_key: Arc<str> = api_key.into();

    thread::spawn(move || {
        let handles: Vec<_> = (0..WORKERS)
            .map(|_| {
                let (queue, limiter, agent, tx, repaint, api_key) =
                    (queue.clone(), limiter.clone(), agent.clone(), tx.clone(), repaint.clone(), api_key.clone());
                thread::spawn(move || {
                    loop {
                        let Some((index, link)) = queue.lock().unwrap().pop_front() else { break };
                        let result = bypass_one(&agent, &limiter, endpoint, &api_key, &link);
                        let _ = tx.send(Event::Done(index, result));
                        repaint();
                    }
                })
            })
            .collect();
        for handle in handles {
            let _ = handle.join();
        }
        let _ = tx.send(Event::Finished);
        repaint();
    });
}

fn bypass_one(agent: &ureq::Agent, limiter: &RateLimiter, endpoint: &str, api_key: &str, link: &str) -> Result<String, String> {
    let mut last_error = String::from("Unknown error");
    // Back off before a retry, but don't wait after the final attempt.
    let backoff = |attempt: u32| {
        if attempt + 1 < MAX_ATTEMPTS {
            thread::sleep(Duration::from_secs(2 << attempt));
        }
    };
    for attempt in 0..MAX_ATTEMPTS {
        limiter.acquire();
        let response = agent.get(endpoint).header("x-api-key", api_key).query("url", link).call();
        let mut response = match response {
            Ok(r) => r,
            Err(e) => {
                last_error = format!("Network error: {e}");
                backoff(attempt);
                continue;
            }
        };

        let status = response.status().as_u16();
        if status == 429 {
            let reset = response
                .headers()
                .get("ratelimit-reset")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<u64>().ok())
                .unwrap_or(10);
            last_error = String::from("Rate limited");
            if attempt + 1 < MAX_ATTEMPTS {
                thread::sleep(Duration::from_secs(reset.clamp(1, 60)));
            }
            continue;
        }

        let body = response.body_mut().read_to_string().unwrap_or_default();
        let json: serde_json::Value = match serde_json::from_str(&body) {
            Ok(v) => v,
            Err(_) if status >= 500 => {
                last_error = format!("Server error ({status})");
                backoff(attempt);
                continue;
            }
            Err(_) => return Err(format!("Unexpected response ({status})")),
        };

        let api_status = json["status"].as_str();
        let succeeded = api_status == Some("success") || ((200..300).contains(&status) && api_status != Some("error"));
        if succeeded {
            if let Some(url) = extract_url(&json) {
                return Ok(url);
            }
        }
        // The API reports bad links as JSON errors (sometimes with a 500), so only retry gateway errors.
        let message = json["message"].as_str().or(json["error"].as_str()).unwrap_or("Bypass failed");
        if matches!(status, 502..=504) {
            last_error = message.to_string();
            backoff(attempt);
            continue;
        }
        return Err(message.to_string());
    }
    Err(last_error)
}

fn extract_url(json: &serde_json::Value) -> Option<String> {
    ["result", "destination", "url", "bypassed", "link"]
        .iter()
        .find_map(|k| json[*k].as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}
