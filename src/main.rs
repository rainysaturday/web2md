use clap::Parser;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use regex::Regex;

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
    let window_re = Regex::new(r"window\.[a-zA-Z_][a-zA-Z0-9_]*\s*=\s*\{[^{}]*(?:\{[^{}]*\}[^{}]*)*\};?").unwrap();
    cleaned = window_re.replace_all(&cleaned, "").to_string();
    
    // Remove standalone CSS-like patterns (class definitions at start of text)
    let css_start_re = Regex::new(r"^\s*\.[a-zA-Z_][a-zA-Z0-9_-]*\s*\{[^}]*\}\s*").unwrap();
    cleaned = css_start_re.replace_all(&cleaned, "").to_string();
    
    cleaned
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

    // Remove images by default, unless --with-images is specified
    if !args.with_images {
        body_html = remove_images(&body_html);
    }

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
