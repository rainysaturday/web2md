use clap::Parser;
use regex::Regex;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "webmd")]
#[command(about = "Download a webpage and convert HTML to Markdown")]
struct Args {
    /// URL to download and convert to Markdown
    url: String,

    /// Output file path (defaults to stdout)
    #[arg(short = 'o', long = "output")]
    output: Option<PathBuf>,

    /// Include images in output (removed by default)
    #[arg(long = "with-images")]
    with_images: bool,
}

fn extract_body_html(html: &str) -> String {
    let document = scraper::Html::parse_document(html);
    let body_selector = scraper::Selector::parse("body").unwrap();

    if let Some(body) = document.select(&body_selector).next() {
        body.html()
    } else {
        html.to_string()
    }
}

fn remove_images(html: &str) -> String {
    let re = Regex::new(r"<img[^>]*>").unwrap();
    re.replace_all(html, "").to_string()
}

fn clean_html(html: &str) -> String {
    let mut cleaned = html.to_string();

    // Remove script tags and their content
    let script_re = Regex::new(r"(?s)<script[^>]*>.*?</script>").unwrap();
    cleaned = script_re.replace_all(&cleaned, "").to_string();

    // Remove style tags and their content
    let style_re = Regex::new(r"(?s)<style[^>]*>.*?</style>").unwrap();
    cleaned = style_re.replace_all(&cleaned, "").to_string();

    // Remove noscript tags and their content
    let noscript_re = Regex::new(r"(?s)<noscript[^>]*>.*?</noscript>").unwrap();
    cleaned = noscript_re.replace_all(&cleaned, "").to_string();

    // Remove inline event handlers (onclick, onload, etc.)
    let event_re = Regex::new(r#"\s+on\w+\s*=\s*["'][^"']*["']"#).unwrap();
    cleaned = event_re.replace_all(&cleaned, "").to_string();

    // Remove inline style attributes
    let style_attr_re = Regex::new(r#"\s+style\s*=\s*["'][^"']*["']"#).unwrap();
    cleaned = style_attr_re.replace_all(&cleaned, "").to_string();

    // Remove CSS class definitions that appear as text (e.g., .cls-1{fill:#fff})
    let css_def_re = Regex::new(r"\.[a-zA-Z_][a-zA-Z0-9_-]*\s*\{[^}]*\}").unwrap();
    cleaned = css_def_re.replace_all(&cleaned, "").to_string();

    // Remove window.* assignments (e.g., window.Di.bamData = {...})
    let window_re =
        Regex::new(r"window\.[a-zA-Z_][a-zA-Z0-9_]*\s*=\s*\{[^{}]*(?:\{[^{}]*\}[^{}]*)*\};?")
            .unwrap();
    cleaned = window_re.replace_all(&cleaned, "").to_string();

    // Remove standalone CSS-like patterns (class definitions at start of text)
    let css_start_re = Regex::new(r"^\s*\.[a-zA-Z_][a-zA-Z0-9_-]*\s*\{[^}]*\}\s*").unwrap();
    cleaned = css_start_re.replace_all(&cleaned, "").to_string();

    cleaned
}

/// Remove entire HTML elements that have `aria-hidden="true"`.
///
/// Uses `scraper` to find elements with `aria-hidden="true"` and removes them
/// from the HTML string by serializing each element and replacing it.
fn remove_aria_hidden_elements(html: &str) -> String {
    let document = scraper::Html::parse_document(html);
    let selector = match scraper::Selector::parse(r#"[aria-hidden="true"]"#) {
        Ok(s) => s,
        Err(_) => return html.to_string(),
    };

    // Collect all matched elements with their serialized outer HTML
    let mut matches: Vec<String> = Vec::new();
    for element in document.select(&selector) {
        matches.push(element.html());
    }

    if matches.is_empty() {
        return html.to_string();
    }

    // Remove each matched element from the HTML string
    let mut result = html.to_string();
    for outer_html in &matches {
        result = result.replacen(outer_html, "", 1);
    }

    result
}

/// Apply `aria-label` as alt text for `<img>` tags that lack an `alt` attribute.
///
/// For each `<img>` tag that has an `aria-label` attribute but no `alt` attribute,
/// this function sets the `alt` attribute to the value of `aria-label`.
fn apply_aria_labels_to_images(html: &str) -> String {
    let mut result = String::new();
    let mut pos = 0;
    let s = html;

    // Match <img ... > or <img ... />
    let img_re = Regex::new(r"<img\b[^>]*>").unwrap();
    // Match aria-label with either double or single quotes (captures the value)
    let aria_label_re =
        Regex::new(r#"\s+aria-label\s*=\s*"([^"]*)"|aria-label\s*=\s*'([^']*)'"#).unwrap();
    // Check if alt attribute exists
    let alt_check_re = Regex::new(r"\s+alt\s*=\s*").unwrap();

    while let Some(m) = img_re.find(&s[pos..]) {
        let abs_start = pos + m.start();
        let abs_end = pos + m.end();

        // Add text before this img tag
        result.push_str(&s[pos..abs_start]);

        let img_tag = &s[abs_start..abs_end];

        // Check if it already has an alt attribute
        let has_alt = alt_check_re.is_match(img_tag);

        if !has_alt {
            // Check for aria-label (either double-quoted or single-quoted value)
            if let Some(caps) = aria_label_re.captures(img_tag) {
                // Group 1 is double-quoted value, group 2 is single-quoted value
                let label_value = caps
                    .get(1)
                    .or_else(|| caps.get(2))
                    .map(|m| m.as_str())
                    .unwrap_or("");

                if !label_value.is_empty() {
                    // Insert alt="[value]" after <img
                    let after_img = 4; // after "<img"
                    let modified = format!(
                        "{} alt=\"{}\"{}",
                        &img_tag[..after_img],
                        label_value.replace('"', "&quot;"),
                        &img_tag[after_img..]
                    );
                    result.push_str(&modified);
                } else {
                    result.push_str(img_tag);
                }
            } else {
                result.push_str(img_tag);
            }
        } else {
            result.push_str(img_tag);
        }

        pos = abs_end;
    }

    // Add remaining text
    result.push_str(&s[pos..]);
    result
}

/// Strip all ARIA attributes (aria-*) from the HTML.
///
/// Removes attributes like aria-label, aria-hidden, aria-describedby, etc.
fn strip_aria_attributes(html: &str) -> String {
    // Match aria-xxx="..." or aria-xxx='...'
    let re_double =
        Regex::new(r#"\s+aria-[a-zA-Z_][a-zA-Z0-9_-]*\s*=\s*"(?:[^"\\]|\\.)*""#).unwrap();
    let re_single =
        Regex::new(r#"\s+aria-[a-zA-Z_][a-zA-Z0-9_-]*\s*=\s*'(?:[^'\\]|\\.)*'"#).unwrap();

    let mut result = re_double.replace_all(html, "").to_string();
    result = re_single.replace_all(&result, "").to_string();
    result
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    // Fetch the webpage
    let response = reqwest::blocking::get(&args.url)?;
    let html = response.text()?;

    // Extract body content to remove script/style tags
    let mut body_html = extract_body_html(&html);

    // Clean HTML - remove scripts, styles, CSS, JS
    body_html = clean_html(&body_html);

    // ARIA support: remove elements with aria-hidden="true"
    body_html = remove_aria_hidden_elements(&body_html);

    // ARIA support: use aria-label as fallback alt text for images
    body_html = apply_aria_labels_to_images(&body_html);

    // Remove images by default, unless --with-images is specified
    if !args.with_images {
        body_html = remove_images(&body_html);
    }

    // ARIA support: strip all remaining aria-* attributes from the output
    body_html = strip_aria_attributes(&body_html);

    // Convert HTML to Markdown
    let markdown = html2md::parse_html(&body_html);

    // Output to file or stdout
    if let Some(output_path) = args.output {
        let mut file = File::create(&output_path)?;
        file.write_all(markdown.as_bytes())?;
    } else {
        println!("{}", markdown);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_remove_aria_hidden_simple() {
        let html = r#"<div><span aria-hidden="true">hidden</span><span>visible</span></div>"#;
        let result = remove_aria_hidden_elements(html);
        assert_eq!(result, "<div><span>visible</span></div>");
    }

    #[test]
    fn test_remove_aria_hidden_nested() {
        let html =
            r#"<div aria-hidden="true"><div>nested</div><p>text</p></div><span>visible</span>"#;
        let result = remove_aria_hidden_elements(html);
        assert_eq!(result, "<span>visible</span>");
    }

    #[test]
    fn test_remove_aria_hidden_no_match() {
        let html = r#"<div><span>visible</span></div>"#;
        let result = remove_aria_hidden_elements(html);
        assert_eq!(result, html);
    }

    #[test]
    fn test_aria_label_on_img() {
        let html = r#"<img src="test.jpg" aria-label="A test image">"#;
        let result = apply_aria_labels_to_images(html);
        assert!(result.contains(r#"alt="A test image""#));
    }

    #[test]
    fn test_aria_label_not_applied_when_alt_exists() {
        let html = r#"<img src="test.jpg" alt="Existing" aria-label="Should not be used">"#;
        let result = apply_aria_labels_to_images(html);
        assert!(result.contains(r#"alt="Existing""#));
        assert!(!result.contains(r#"alt="Should not be used""#));
    }

    #[test]
    fn test_aria_label_single_quotes() {
        let html = r#"<img src='test.jpg' aria-label='A test image'>"#;
        let result = apply_aria_labels_to_images(html);
        assert!(result.contains(r#"alt="A test image""#));
    }

    #[test]
    fn test_strip_aria_attributes() {
        let html = r#"<div aria-hidden="true" aria-label="test" class="foo">content</div>"#;
        let result = strip_aria_attributes(html);
        assert!(!result.contains("aria-hidden"));
        assert!(!result.contains("aria-label"));
        assert!(result.contains("class=\"foo\""));
        assert!(result.contains("content"));
    }

    #[test]
    fn test_strip_aria_multiple() {
        let html = r#"<div aria-hidden="true"><span aria-label="test">text</span></div>"#;
        let result = strip_aria_attributes(html);
        assert!(!result.contains("aria-hidden"));
        assert!(!result.contains("aria-label"));
    }

    #[test]
    fn test_full_pipeline() {
        let html = r#"<body><div aria-hidden="true"><p>hidden text</p></div><p aria-label="hello">visible</p><img src="x.jpg" aria-label="photo"></body>"#;
        let mut body_html = extract_body_html(&html);
        body_html = clean_html(&body_html);
        body_html = remove_aria_hidden_elements(&body_html);
        body_html = apply_aria_labels_to_images(&body_html);
        body_html = strip_aria_attributes(&body_html);

        assert!(!body_html.contains("hidden text"));
        assert!(body_html.contains("visible"));
        assert!(body_html.contains(r#"alt="photo""#));
        assert!(!body_html.contains("aria-hidden"));
        assert!(!body_html.contains("aria-label"));
    }
}
