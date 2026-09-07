web2md is a small Rust-based CLI tool for downloading a webpage and converting the HTML into Markdown.

This makes it easy to get a minimal readable representation of the webpage from CLI.

It is written in rust, uses clap for CLI argument parsing, and takes one argument, the URL to download.
Optionally it can support the '-o <file>' flag to write to a file instead of STDOUT.
