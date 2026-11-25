use reqwest::header::{HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Client, StatusCode, Url};
use serde_json::{json, Value};
use std::sync::OnceLock;
use std::time::Duration;

const DEEPL_BODY_LIMIT: usize = 128 * 1024;
const DEEPL_FREE_URL: &str = "https://api-free.deepl.com/v2/translate";
const DEEPL_PRO_URL: &str = "https://api.deepl.com/v2/translate";
const ENGLISH: &str = "en";

fn is_loopback(url: &Url) -> bool {
    matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
}

pub fn openai_endpoint(base_url: &str) -> Result<Url, String> {
    let base_url = base_url.trim();
    if !base_url.contains("://") || base_url.chars().any(char::is_control) {
        return Err("invalid_endpoint".into());
    }
    let mut url = Url::parse(base_url).map_err(|_| "invalid_endpoint")?;
    if url.host().is_none()
        || !(url.scheme() == "https" || (url.scheme() == "http" && is_loopback(&url)))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("invalid_endpoint".into());
    }
    url.path_segments_mut()
        .map_err(|_| "invalid_endpoint")?
        .pop_if_empty()
        .extend(["chat", "completions"]);
    Ok(url)
}

fn client(url: &Url) -> Result<&'static Client, String> {
    static REMOTE: OnceLock<Result<Client, reqwest::Error>> = OnceLock::new();
    static LOOPBACK: OnceLock<Result<Client, reqwest::Error>> = OnceLock::new();
    let loopback = is_loopback(url);
    let cached = if loopback { &LOOPBACK } else { &REMOTE };
    cached
        .get_or_init(|| {
            let builder = Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::none());
            if loopback {
                builder.no_proxy().build()
            } else {
                builder.build()
            }
        })
        .as_ref()
        .map_err(|_| "network_error".into())
}

fn deepl_endpoint(key: &str) -> &'static str {
    if key.ends_with(":fx") {
        DEEPL_FREE_URL
    } else {
        DEEPL_PRO_URL
    }
}

fn deepl_body(text: &str, source: &str, target: &str) -> Result<Vec<u8>, String> {
    let source_lang = if source == ENGLISH { "EN" } else { "ZH" };
    let target_lang = if target == ENGLISH {
        "EN-US"
    } else {
        "ZH-HANS"
    };
    let body = serde_json::to_vec(&json!({
        "text": [text],
        "source_lang": source_lang,
        "target_lang": target_lang,
    }))
    .map_err(|_| "invalid_request")?;
    if body.len() > DEEPL_BODY_LIMIT {
        return Err("too_large".into());
    }
    Ok(body)
}

fn openai_body(text: &str, source: &str, target: &str, model: &str) -> Value {
    let language = |code: &str| {
        if code == ENGLISH {
            "English"
        } else {
            "Simplified Chinese"
        }
    };
    let prompt = format!(
        "Translate from {} to {}. Return only the translation. Preserve line breaks and paragraphs.",
        language(source), language(target)
    );
    json!({
        "model": model,
        "messages": [
            { "role": "system", "content": prompt },
            { "role": "user", "content": text },
        ],
        "temperature": 0.2,
    })
}

fn network_error(error: reqwest::Error) -> String {
    if error.is_timeout() {
        "timeout".into()
    } else {
        "network_error".into()
    }
}

fn status_error(status: StatusCode, deepl: bool) -> String {
    match status.as_u16() {
        401 | 403 => "auth_failed".into(),
        429 => "quota_exceeded".into(),
        413 if deepl => "too_large".into(),
        456 if deepl => "quota_exceeded".into(),
        // A provider may echo credentials or request text in its error body.
        _ => format!(
            "upstream_error: HTTP {} {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or("")
        )
        .trim_end()
        .into(),
    }
}

fn response_text(body: &Value, deepl: bool, key: Option<&str>) -> Result<String, String> {
    let content = if deepl {
        &body["translations"][0]["text"]
    } else {
        &body["choices"][0]["message"]["content"]
    };
    content
        .as_str()
        .filter(|text| !text.trim().is_empty())
        .filter(|text| !key.is_some_and(|key| text.contains(key)))
        .map(str::to_string)
        .ok_or_else(|| "invalid_response".into())
}

fn authorization(scheme: &str, key: &str) -> Result<HeaderValue, String> {
    let mut header =
        HeaderValue::from_str(&format!("{scheme} {key}")).map_err(|_| "auth_failed")?;
    header.set_sensitive(true);
    Ok(header)
}

async fn translate(
    endpoint: Url,
    body: Vec<u8>,
    auth: Option<HeaderValue>,
    key: Option<&str>,
    deepl: bool,
) -> Result<String, String> {
    let mut request = client(&endpoint)?
        .post(endpoint)
        .header(CONTENT_TYPE, "application/json")
        .body(body);
    if let Some(auth) = auth {
        request = request.header(AUTHORIZATION, auth);
    }
    let response = request.send().await.map_err(network_error)?;
    if !response.status().is_success() {
        return Err(status_error(response.status(), deepl));
    }
    let body = response.json::<Value>().await.map_err(|error| {
        if error.is_decode() {
            "invalid_response".into()
        } else {
            network_error(error)
        }
    })?;
    response_text(&body, deepl, key)
}

pub async fn deepl(key: &str, text: &str, source: &str, target: &str) -> Result<String, String> {
    let body = deepl_body(text, source, target)?;
    let auth = authorization("DeepL-Auth-Key", key)?;
    let endpoint = Url::parse(deepl_endpoint(key)).expect("valid DeepL endpoint");
    translate(endpoint, body, Some(auth), Some(key), true).await
}

pub async fn openai(
    endpoint: Url,
    model: &str,
    key: Option<&str>,
    text: &str,
    source: &str,
    target: &str,
) -> Result<String, String> {
    let body = serde_json::to_vec(&openai_body(text, source, target, model))
        .map_err(|_| "invalid_request")?;
    let auth = key.map(|key| authorization("Bearer", key)).transpose()?;
    translate(endpoint, body, auth, key, false).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deepl_uses_official_endpoints_and_language_codes() {
        assert_eq!(deepl_endpoint("example:fx"), DEEPL_FREE_URL);
        assert_eq!(deepl_endpoint("example"), DEEPL_PRO_URL);
        let body = deepl_body("Hello\n\nWorld", "en", "zh-Hans").unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap(),
            json!({
                "text": ["Hello\n\nWorld"], "source_lang": "EN", "target_lang": "ZH-HANS",
            })
        );
        let body = deepl_body("你好", "zh-Hans", "en").unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["source_lang"], "ZH");
        assert_eq!(body["target_lang"], "EN-US");
        assert!(authorization("DeepL-Auth-Key", "example:fx")
            .unwrap()
            .is_sensitive());
    }

    #[test]
    fn deepl_size_limit_counts_the_serialized_body() {
        let overhead = deepl_body("", "en", "zh-Hans").unwrap().len();
        let max = "x".repeat(DEEPL_BODY_LIMIT - overhead);
        assert_eq!(
            deepl_body(&max, "en", "zh-Hans").unwrap().len(),
            DEEPL_BODY_LIMIT
        );
        assert_eq!(
            deepl_body(&(max + "界"), "en", "zh-Hans"),
            Err("too_large".into())
        );
        assert_eq!(
            deepl_body(&"\n".repeat(70_000), "en", "zh-Hans"),
            Err("too_large".into())
        );
    }

    #[test]
    fn openai_builds_chat_messages_and_keeps_the_entire_input() {
        let text = "你好\n\n世界";
        let body = openai_body(text, "zh-Hans", "en", "gpt-4o-mini");
        assert_eq!(body["model"], "gpt-4o-mini");
        assert_eq!(body["temperature"], 0.2);
        assert_eq!(body["messages"][0]["role"], "system");
        let system = body["messages"][0]["content"].as_str().unwrap();
        assert!(system.contains("Simplified Chinese to English"));
        assert!(
            system.contains("Return only the translation. Preserve line breaks and paragraphs.")
        );
        assert_eq!(
            body["messages"][1],
            json!({ "role": "user", "content": text })
        );
        assert!(authorization("Bearer", "example").unwrap().is_sensitive());
        assert_eq!(
            authorization("Bearer", "example\nkey"),
            Err("auth_failed".into())
        );
    }

    #[test]
    fn parses_provider_translations_and_drops_unexpected_responses() {
        let deepl = json!({ "translations": [{ "text": "甲\n\n乙" }] });
        let openai = json!({ "choices": [{ "message": { "content": "A\n\nB" } }] });
        assert_eq!(response_text(&deepl, true, None).unwrap(), "甲\n\n乙");
        assert_eq!(response_text(&openai, false, None).unwrap(), "A\n\nB");
        assert_eq!(
            response_text(&json!({ "choices": [] }), false, None),
            Err("invalid_response".into())
        );
        let echoed = json!({ "choices": [{ "message": { "content": "Bearer example-key" } }] });
        assert_eq!(
            response_text(&echoed, false, Some("example-key")),
            Err("invalid_response".into())
        );
    }

    #[test]
    fn maps_http_statuses_without_returning_the_upstream_body() {
        for (status, deepl, expected) in [
            (401, true, "auth_failed"),
            (403, false, "auth_failed"),
            (429, false, "quota_exceeded"),
            (456, true, "quota_exceeded"),
            (413, true, "too_large"),
            (456, false, "upstream_error: HTTP 456"),
            (502, false, "upstream_error: HTTP 502 Bad Gateway"),
        ] {
            assert_eq!(
                status_error(StatusCode::from_u16(status).unwrap(), deepl),
                expected
            );
        }
    }

    #[test]
    fn openai_requires_https_except_on_loopback() {
        assert_eq!(
            openai_endpoint("https://example.com/v1/").unwrap().as_str(),
            "https://example.com/v1/chat/completions",
        );
        for base in [
            "http://localhost:11434/v1",
            "http://127.0.0.1",
            "http://[::1]:8080",
        ] {
            assert!(openai_endpoint(base).is_ok(), "{base}");
        }
        for base in [
            "http://example.com",
            "http://localhost.example.com",
            "http://127.0.0.2",
            "http://[::2]",
            "file:///tmp/example",
            "ftp://localhost",
            "https:example.com",
            "https://a:b@example.com",
            "https://example.com?key=secret",
            "https://example.com/#x",
        ] {
            assert_eq!(
                openai_endpoint(base),
                Err("invalid_endpoint".into()),
                "{base}"
            );
        }
    }
}
