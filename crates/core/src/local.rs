//! Riot Client が書き出すローカルファイルの読み取り。

use crate::error::{Error, Result};
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Clone, PartialEq, Eq)]
pub struct Lockfile {
    pub port: u16,
    pub password: String,
}

// パスワードをログへ出さないため、Debug は手書きする。
impl fmt::Debug for Lockfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Lockfile")
            .field("port", &self.port)
            .field("password", &"<redacted>")
            .finish()
    }
}

impl Lockfile {
    /// `name:pid:port:password:protocol` 形式を解析する。
    pub fn parse(text: &str) -> Result<Self> {
        let fields: Vec<&str> = text.trim().split(':').collect();
        let [_name, _pid, port, password, _protocol] = fields.as_slice() else {
            return Err(Error::InvalidLocalFile("lockfile".into()));
        };
        let port = port
            .parse()
            .map_err(|_| Error::InvalidLocalFile("lockfile の port".into()))?;
        Ok(Self { port, password: (*password).to_owned() })
    }
}

/// `ShooterGame.log` から得る接続先とクライアントのバージョン。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientLogInfo {
    pub region: String,
    pub glz_region: String,
    pub glz_shard: String,
    /// ログに版の行がまだないときは `None`。呼び出し側で別の取得元を使う。
    pub client_version: Option<String>,
}

/// ログ全文を解析する。必要な行がなければ `None` を返す。
pub fn parse_client_log(text: &str) -> Option<ClientLogInfo> {
    let mut pd = None;
    let mut glz = None;
    let mut version = None;
    for line in text.lines() {
        if pd.is_none() {
            if let Some((head, _)) = line.split_once(".a.pvp.net/account-xp/v1/") {
                pd = head.rsplit('.').next().map(str::to_owned);
            }
        }
        if glz.is_none() {
            if let Some((_, tail)) = line.split_once("https://glz-") {
                let mut parts = tail.split('.');
                if let (Some(region), Some(shard)) = (parts.next(), parts.next()) {
                    glz = Some((region.to_owned(), shard.to_owned()));
                }
            }
        }
        if version.is_none() {
            if let Some((_, tail)) = line.split_once("CI server version: ") {
                version = Some(tail.trim().to_owned());
            }
        }
        if pd.is_some() && glz.is_some() && version.is_some() {
            break;
        }
    }
    let (region, (glz_region, glz_shard)) = match pd? {
        pbe if pbe == "pbe" => ("na".to_owned(), ("na-1".to_owned(), "na".to_owned())),
        region => (region, glz?),
    };
    Some(ClientLogInfo { region, glz_region, glz_shard, client_version: version })
}

pub struct LocalPaths {
    pub lockfile: PathBuf,
    pub client_log: PathBuf,
}

impl LocalPaths {
    pub fn from_local_app_data(root: &Path) -> Self {
        Self {
            lockfile: root.join(r"Riot Games").join("Riot Client").join("Config").join("lockfile"),
            client_log: root.join("VALORANT").join("Saved").join("Logs").join("ShooterGame.log"),
        }
    }

    pub fn detect() -> Result<Self> {
        let root = std::env::var_os("LOCALAPPDATA")
            .ok_or_else(|| Error::ClientNotFound("LOCALAPPDATA が未設定です".into()))?;
        Ok(Self::from_local_app_data(Path::new(&root)))
    }
}

/// VALORANT が起動中でもログを読めるよう、不正な UTF-8 は置き換えて読む。
pub fn read_lossy(path: &Path) -> Result<String> {
    std::fs::read(path)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        // フルパスにはユーザー名が含まれるので、ファイル名だけを伝える。
        .map_err(|_| {
            let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            Error::ClientNotFound(name)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_error_hides_the_user_folder() {
        let error = read_lossy(Path::new("/home/secret-user/lockfile")).unwrap_err();
        let text = error.to_string();
        assert!(text.contains("lockfile") && !text.contains("secret-user"));
    }

    #[test]
    fn parses_lockfile() {
        let lock = Lockfile::parse("Riot Client:1234:51234:s3cret:https\n").unwrap();
        assert_eq!(lock.port, 51234);
        assert_eq!(lock.password, "s3cret");
    }

    #[test]
    fn rejects_broken_lockfile() {
        assert!(Lockfile::parse("Riot Client:1234").is_err());
        assert!(Lockfile::parse("a:b:notaport:d:e").is_err());
    }

    #[test]
    fn debug_output_hides_password() {
        let lock = Lockfile::parse("n:1:2:s3cret:https").unwrap();
        assert!(!format!("{lock:?}").contains("s3cret"));
    }

    const LOG: &str = "\
[x] CI server version: release-11.09-shipping-5-123\n\
[x] https://glz-ap-1.ap.a.pvp.net/session/v1\n\
[x] GET https://pd.ap.a.pvp.net/account-xp/v1/players/abc\n";

    #[test]
    fn parses_client_log() {
        let info = parse_client_log(LOG).unwrap();
        assert_eq!(info.region, "ap");
        assert_eq!(info.glz_region, "ap-1");
        assert_eq!(info.glz_shard, "ap");
        assert_eq!(info.client_version.as_deref(), Some("release-11.09-shipping-5-123"));
    }

    #[test]
    fn pbe_maps_to_na() {
        let log = "CI server version: v\nGET https://pd.pbe.a.pvp.net/account-xp/v1/x\n";
        let info = parse_client_log(log).unwrap();
        assert_eq!((info.region.as_str(), info.glz_region.as_str()), ("na", "na-1"));
    }

    #[test]
    fn missing_version_still_returns_servers() {
        let log = "GET https://pd.eu.a.pvp.net/account-xp/v1/x\nhttps://glz-eu-1.eu.a.pvp.net/x\n";
        let info = parse_client_log(log).unwrap();
        assert_eq!(info.region, "eu");
        assert!(info.client_version.is_none());
    }

    // 元実装はこの場合に EOF で無限ループしていた。
    #[test]
    fn incomplete_log_returns_none() {
        assert!(parse_client_log("nothing useful\n").is_none());
    }
}
