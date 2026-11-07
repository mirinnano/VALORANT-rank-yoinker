use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("VALORANT のローカルファイルが見つかりません: {0}")]
    ClientNotFound(String),
    #[error("VALORANT の起動を待っています")]
    ClientNotReady,
    #[error("VALORANT の準備が完了していません: {0}")]
    ClientStep(&'static str),
    #[error("ローカルファイルの形式が不正です: {0}")]
    InvalidLocalFile(String),
    #[error("通信に失敗しました: {0}")]
    Http(reqwest::Error),
    #[error("{service} が {status} を返しました")]
    Status { service: &'static str, status: u16 },
    #[error("再試行の上限に達しました: {0}")]
    RetriesExhausted(String),
    #[error("応答の形式が不正です: {0}")]
    Decode(String),
    #[error("JSON の解析に失敗しました: {0}")]
    Json(#[from] serde_json::Error),
    #[error("データベースのエラーです: {0}")]
    Database(#[from] rusqlite::Error),
}

// URL には puuid が含まれるので、ログや画面へ出る前に取り除く。
impl From<reqwest::Error> for Error {
    fn from(error: reqwest::Error) -> Self {
        Self::Http(error.without_url())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    // 接続できないポートへ送り、URL 付きの通信エラーを作る。外部とは通信しない。
    #[tokio::test]
    async fn http_errors_do_not_carry_the_url() {
        let reply = reqwest::Client::new().get("http://127.0.0.1:1/players/secret-puuid").send().await;
        let raw = reply.expect_err("port 1 is closed");
        assert!(raw.to_string().contains("secret-puuid"), "前提: 元のエラーには URL が含まれる");
        assert!(!Error::from(raw).to_string().contains("secret-puuid"));
    }
}
