use serde::{Serialize, Serializer};
use std::fmt;

#[derive(Debug)]
pub enum AppError {
    Database(String),
    IO(String),
    Network(String),
    Internal(String),
    Validation(String),
    Encryption(String),
}

impl std::error::Error for AppError {}

// 前端约定：错误字符串格式为 "[tag] detail"，tag 由前端负责本地化，后端不写死任何语言。
impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::Database(e) => write!(f, "[db] {}", e),
            AppError::IO(e) => write!(f, "[io] {}", e),
            AppError::Network(e) => write!(f, "[network] {}", e),
            AppError::Internal(e) => write!(f, "[internal] {}", e),
            AppError::Validation(e) => write!(f, "[validation] {}", e),
            AppError::Encryption(e) => write!(f, "[encryption] {}", e),
        }
    }
}

// 允许 AppError 被序列化，以便直接返回给 Tauri 前端
impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

// 通用转换宏或实现，便于从 common errors 转换过来
impl From<rusqlite::Error> for AppError {
    fn from(err: rusqlite::Error) -> Self {
        AppError::Database(err.to_string())
    }
}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        AppError::IO(err.to_string())
    }
}

impl From<String> for AppError {
    fn from(err: String) -> Self {
        AppError::Internal(err)
    }
}

impl From<arboard::Error> for AppError {
    fn from(err: arboard::Error) -> Self {
        AppError::Internal(err.to_string())
    }
}

impl From<tauri::Error> for AppError {
    fn from(err: tauri::Error) -> Self {
        AppError::Internal(err.to_string())
    }
}

impl From<image::ImageError> for AppError {
    fn from(err: image::ImageError) -> Self {
        AppError::IO(err.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::AppError;

    #[test]
    fn display_uses_language_neutral_tags() {
        let cases = [
            (AppError::Database("locked".into()), "[db] locked"),
            (AppError::IO("missing".into()), "[io] missing"),
            (
                AppError::Network("HTTP 502 Bad Gateway".into()),
                "[network] HTTP 502 Bad Gateway",
            ),
            (AppError::Internal("boom".into()), "[internal] boom"),
            (
                AppError::Validation("url is empty".into()),
                "[validation] url is empty",
            ),
            (
                AppError::Encryption("bad key".into()),
                "[encryption] bad key",
            ),
        ];
        for (err, expected) in cases {
            assert_eq!(err.to_string(), expected);
            assert_eq!(
                serde_json::to_string(&err).unwrap(),
                serde_json::to_string(expected).unwrap()
            );
        }
    }
}
