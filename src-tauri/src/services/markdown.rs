//! 选择粘贴格式 › Markdown 文本.

use crate::services::clipboard::{
    split_rich_html_and_image_fallback, split_rich_html_and_named_formats,
};

/// Code gets a ``` fence, web rich text is converted, plain text stays as is.
pub fn entry_to_markdown(content: &str, content_type: &str, html: Option<&str>) -> String {
    if content_type == "code" {
        return fenced_code(content);
    }
    html.filter(|html| !html.trim().is_empty())
        .and_then(html_to_markdown)
        .unwrap_or_else(|| content.to_string())
}

fn html_to_markdown(html: &str) -> Option<String> {
    // Drop the markers this app stores with rich text.
    let (html, _) = split_rich_html_and_named_formats(html);
    let (html, _) = split_rich_html_and_image_fallback(&html);
    let markdown = htmd::convert(&html).ok()?;
    let markdown = markdown.trim();
    (!markdown.is_empty()).then(|| markdown.to_string())
}

fn fenced_code(code: &str) -> String {
    // The fence must be longer than any run of backticks in the code.
    let longest_run = code
        .split(|c| c != '`')
        .map(str::len)
        .max()
        .unwrap_or(0);
    let fence = "`".repeat(longest_run.max(2) + 1);
    format!("{fence}\n{}\n{fence}", code.trim_end_matches('\n'))
}

#[cfg(test)]
mod tests {
    use super::entry_to_markdown;

    #[test]
    fn rich_text_becomes_markdown() {
        let html = "<p>Hello <strong>world</strong></p><ul><li>a</li></ul>";
        let markdown = entry_to_markdown("Hello world a", "rich_text", Some(html));
        assert!(markdown.contains("**world**"), "{markdown}");
        assert!(markdown.contains("a"), "{markdown}");
    }

    #[test]
    fn plain_text_stays_as_it_is() {
        assert_eq!(entry_to_markdown("hello", "text", None), "hello");
        assert_eq!(entry_to_markdown("https://a.b", "url", None), "https://a.b");
    }

    #[test]
    fn code_is_fenced_longer_than_its_backticks() {
        assert_eq!(entry_to_markdown("let a = 1;\n", "code", None), "```\nlet a = 1;\n```");
        assert_eq!(entry_to_markdown("a ```` b", "code", None), "`````\na ```` b\n`````");
    }
}
