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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    // Fetch the webpage
    let response = reqwest::blocking::get(&args.url)?;
    let html = response.text()?;

    // Extract body content to remove script/style tags
    let mut body_html = extract_body_html(&html);

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
