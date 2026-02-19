use askama::Template;
use corre_core::publish::Edition;

#[derive(Template)]
#[template(path = "newspaper.html")]
pub struct NewspaperTemplate<'a> {
    pub title: &'a str,
    pub edition: &'a Edition,
}
