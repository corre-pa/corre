use askama::Template;
use corre_core::publish::Edition;

/// Package version, baked in at compile time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Template)]
#[template(path = "newspaper.html")]
pub struct NewspaperTemplate<'a> {
    pub title: &'a str,
    pub edition: &'a Edition,
    pub version: &'a str,
}

#[derive(Template)]
#[template(path = "topics.html")]
pub struct TopicsTemplate<'a> {
    pub title: &'a str,
    pub topics_json: &'a str,
    pub token: &'a str,
}
