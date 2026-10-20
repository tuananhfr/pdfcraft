//! Plain text represented as inert HTML paragraphs for rich clipboard consumers.

pub fn paragraphs(text: &str) -> String {
    let mut html = String::new();
    for line in text.split('\n') {
        html.push_str("<p>");
        for ch in line.strip_suffix('\r').unwrap_or(line).chars() {
            match ch {
                '&' => html.push_str("&amp;"),
                '<' => html.push_str("&lt;"),
                '>' => html.push_str("&gt;"),
                '"' => html.push_str("&quot;"),
                '\'' => html.push_str("&#39;"),
                _ => html.push(ch),
            }
        }
        html.push_str("</p>");
    }
    html
}

#[cfg(test)]
mod tests {
    use super::paragraphs;

    #[test]
    fn escapes_markup_and_preserves_lines() {
        assert_eq!(paragraphs("<&>\"'\r\n\nTiếng Việt\n"), "<p>&lt;&amp;&gt;&quot;&#39;</p><p></p><p>Tiếng Việt</p><p></p>");
        assert_eq!(paragraphs("<script>alert('x')</script>"), "<p>&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt;</p>");
        assert_eq!(paragraphs(""), "<p></p>");
    }
}
