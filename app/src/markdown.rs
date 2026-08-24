//! Sanitised Markdown for assistant turns.

use pulldown_cmark::{Options, Parser, html};

pub(crate) fn render(markdown: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    let parser = Parser::new_ext(markdown, options);
    let mut html_output = String::new();
    html::push_html(&mut html_output, parser);
    ammonia::clean(&html_output)
}

pub(crate) fn escape_plain(text: &str) -> String {
    ammonia::clean_text(text).replace('\n', "<br>\n")
}

#[cfg(test)]
mod tests;
