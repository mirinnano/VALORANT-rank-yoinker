//! HTTP 応答の扱い。元実装の無制限な再帰を、上限付きのループに置き換える。

use crate::error::{Error, Result};
use std::future::Future;
use std::time::Duration;

pub const MAX_ATTEMPTS: u32 = 4;
const BASE_DELAY: Duration = Duration::from_secs(1);
const MAX_DELAY: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub retry_after: Option<Duration>,
    pub body: String,
}

#[derive(Debug, PartialEq, Eq)]
enum Action {
    Done,
    Missing,
    Wait(Duration),
    RefreshToken,
    Fail,
}

fn backoff(attempt: u32) -> Duration {
    (BASE_DELAY * 2u32.saturating_pow(attempt)).min(MAX_DELAY)
}

fn classify(reply: &Reply, attempt: u32, refreshed: bool) -> Action {
    let expired = reply.status == 401
        || (reply.status == 400 && reply.body.contains("BAD_CLAIMS"));
    match reply.status {
        200..=299 => Action::Done,
        404 => Action::Missing,
        _ if expired && !refreshed => Action::RefreshToken,
        429 => Action::Wait(reply.retry_after.unwrap_or_else(|| backoff(attempt)).min(MAX_DELAY)),
        500..=599 => Action::Wait(backoff(attempt)),
        _ => Action::Fail,
    }
}

/// `send` を上限回数まで実行する。404 は `None` を返す。
pub async fn with_policy<S, SF, R, RF>(
    service: &'static str,
    mut send: S,
    mut refresh: R,
) -> Result<Option<String>>
where
    S: FnMut() -> SF,
    SF: Future<Output = Result<Reply>>,
    R: FnMut() -> RF,
    RF: Future<Output = Result<()>>,
{
    let mut refreshed = false;
    let mut last = String::new();
    for attempt in 0..MAX_ATTEMPTS {
        let reply = match send().await {
            Ok(reply) => reply,
            Err(Error::Http(error)) => {
                last = error.without_url().to_string();
                tokio::time::sleep(backoff(attempt)).await;
                continue;
            }
            Err(error) => return Err(error),
        };
        match classify(&reply, attempt, refreshed) {
            Action::Done => return Ok(Some(reply.body)),
            Action::Missing => return Ok(None),
            Action::Wait(delay) => {
                tracing::warn!(service, status = reply.status, wait_ms = delay.as_millis() as u64, "再試行まで待機");
                last = format!("{service}: HTTP {}", reply.status);
                tokio::time::sleep(delay).await;
            }
            Action::RefreshToken => {
                refreshed = true;
                refresh().await?;
            }
            Action::Fail => return Err(Error::Status { service, status: reply.status }),
        }
    }
    Err(Error::RetriesExhausted(last))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::collections::VecDeque;
    use std::cell::RefCell;

    fn reply(status: u16, body: &str) -> Reply {
        Reply { status, retry_after: None, body: body.into() }
    }

    struct Script {
        replies: RefCell<VecDeque<Reply>>,
        sends: Cell<u32>,
        refreshes: Cell<u32>,
    }

    impl Script {
        fn new(replies: Vec<Reply>) -> Self {
            Self { replies: RefCell::new(replies.into()), sends: Cell::new(0), refreshes: Cell::new(0) }
        }

        async fn run(&self) -> Result<Option<String>> {
            with_policy(
                "test",
                || async {
                    self.sends.set(self.sends.get() + 1);
                    Ok(self.replies.borrow_mut().pop_front().expect("想定より多く送信した"))
                },
                || async {
                    self.refreshes.set(self.refreshes.get() + 1);
                    Ok(())
                },
            )
            .await
        }
    }

    #[tokio::test(start_paused = true)]
    async fn success_returns_body() {
        let s = Script::new(vec![reply(200, "ok")]);
        assert_eq!(s.run().await.unwrap().as_deref(), Some("ok"));
    }

    #[tokio::test(start_paused = true)]
    async fn not_found_is_none_without_retry() {
        let s = Script::new(vec![reply(404, "")]);
        assert_eq!(s.run().await.unwrap(), None);
        assert_eq!(s.sends.get(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn rate_limit_honors_retry_after() {
        let mut limited = reply(429, "");
        limited.retry_after = Some(Duration::from_secs(7));
        let s = Script::new(vec![limited, reply(200, "ok")]);
        let start = tokio::time::Instant::now();
        assert!(s.run().await.unwrap().is_some());
        assert_eq!(start.elapsed(), Duration::from_secs(7));
    }

    #[tokio::test(start_paused = true)]
    async fn retries_stop_at_the_limit() {
        let s = Script::new(vec![reply(503, ""); MAX_ATTEMPTS as usize]);
        assert!(matches!(s.run().await, Err(Error::RetriesExhausted(_))));
        assert_eq!(s.sends.get(), MAX_ATTEMPTS);
    }

    #[tokio::test(start_paused = true)]
    async fn bad_claims_refreshes_once_then_succeeds() {
        let s = Script::new(vec![reply(400, r#"{"errorCode":"BAD_CLAIMS"}"#), reply(200, "ok")]);
        assert!(s.run().await.unwrap().is_some());
        assert_eq!(s.refreshes.get(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn repeated_auth_failure_does_not_loop() {
        let s = Script::new(vec![reply(401, ""), reply(401, "")]);
        assert!(matches!(s.run().await, Err(Error::Status { status: 401, .. })));
        assert_eq!(s.refreshes.get(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn client_errors_fail_immediately() {
        let s = Script::new(vec![reply(403, "")]);
        assert!(matches!(s.run().await, Err(Error::Status { status: 403, .. })));
        assert_eq!(s.sends.get(), 1);
    }
}
